use std::{
    collections::{BTreeMap, HashMap, HashSet},
    env,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration as StdDuration, Instant},
};

use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::{ConnectInfo, DefaultBodyLimit, Extension, Multipart, Path, Query, Request, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
        header::{
            ACCEPT_RANGES, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_SECURITY_POLICY, CONTENT_TYPE, ETAG, IF_NONE_MATCH, RANGE, REFERRER_POLICY,
            SET_COOKIE, X_CONTENT_TYPE_OPTIONS,
        },
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, patch, post, put},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{StreamExt, stream};
use hmac::{Hmac, KeyInit, Mac};
#[cfg(not(all(feature = "docker-libvips", target_os = "linux")))]
use mediahub_adapter_image::RustImageProcessor as RuntimeImageProcessor;
#[cfg(all(feature = "docker-libvips", target_os = "linux"))]
use mediahub_adapter_image::VipsImageProcessor as RuntimeImageProcessor;
use mediahub_adapter_local::{LocalObjectStore, LocalUploadError};
use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{
    AccessKeyRecord, AccessKeyRepository, AdminApplicationSummary, AdminBootstrapOutcome,
    AdminJobSummary, AdminRepository, AdminStorageSummary, AdminSystemSettings, AdminUserSummary,
    ApplicationError, ApplicationRepository, ApplicationSummary, AsyncJobApplicationError,
    AsyncJobService, AuditEvent, AuditRepository, AuthRepository, CancelAsyncJobRequest,
    CancelUploadSessionRequest, Clock, CompleteAsyncJobRequest, CompleteUploadSessionRequest,
    CompletedIdempotencyResponse, CreateAsyncJobRequest, CreateUploadSessionRequest,
    FailAsyncJobRequest, IdempotencyClaim, IdempotencyContext, MAX_DOWNLOAD_BYTES_PER_SECOND,
    MEDIA_UPLOAD_HEARTBEAT_SECONDS, MEDIA_UPLOAD_LEASE_SECONDS, MIN_DOWNLOAD_BYTES_PER_SECOND,
    MediaDirectoryListCursor, MediaDirectoryListQuery, MediaListCursor, MediaListQuery,
    MediaRepository, NewAccessKey, NewWebhookEndpoint, ObjectStore, ObjectStoreError,
    OneTimeTokenPurpose, OutboxEvent, S3MultipartRepository, SecretKeyVersionRepository,
    SessionRecord, UploadMediaRequest, UploadMediaService, UploadSessionRepository,
    UploadSessionService, UploadSessionStorage, UploadTarget, UserAccount, VariantApplicationError,
    VariantService, WebhookDelivery, WebhookDeliveryFailureDisposition,
    WebhookDeliveryHistoryCursor, WebhookDeliveryHistoryItem, WebhookDeliveryHistoryQuery,
    WebhookDeliveryHistoryStatus, WebhookDeliveryRepository, WebhookEndpoint,
    WebhookEndpointRepository, WebhookEndpointUpdate,
};
use mediahub_core::{
    ApplicationId, AsyncJobAction, AsyncJobId, AsyncJobItemResult, Bucket, BucketId, BucketPolicy,
    ClientMetadata, CropPosition, DomainError, LifecycleRule, Media, MediaId, MediaState,
    OffsetDateTime, UploadSession, UploadSessionId, UploadSessionState, UserId, VariantFit,
    VariantFormat, VariantTransform, Visibility,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{info, warn};
use url::{Host, Url};

use mediahub_server::{
    access_key::{AccessKeyCipher, AccessKeyCipherError, generate_secret},
    identity::{hash_password, normalize_email, verify_password},
    security::{CanonicalRequest, HmacError, MAX_SIGNATURE_AGE, verify_hmac},
};

mod email;
mod runtime_storage;
mod s3_gateway;
mod s3_http;
mod s3_list;
mod s3_multipart_storage;
mod s3_xml;
mod server_config;
mod webdav;

use email::{AuthEmailKind, ResendEmailProvider};
use runtime_storage::RuntimeObjectStore;
use server_config::{CookieConfig, ServerConfig, StorageBackend};

const SESSION_COOKIE: &str = "mediahub_session";
const CSRF_COOKIE: &str = "mediahub_csrf";
const SESSION_SECONDS: i64 = 60 * 60 * 24 * 14;
const MAX_REQUEST_BYTES: usize = 20 * 1024 * 1024;
const MAX_S3_CONTROL_REQUEST_BYTES: usize = 64 * 1024 * 1024;
const MAX_UPLOAD_OBJECT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_ERROR_RESPONSE_BYTES: usize = 64 * 1024;
const IDEMPOTENCY_SECONDS: i64 = 60 * 60 * 24;
const DEFAULT_APPLICATION_QUOTA_BYTES: u64 = 1_073_741_824;
const SIGNED_MEDIA_URL_SECONDS: i64 = 60 * 5;
const VERIFY_EMAIL_TOKEN_SECONDS: i64 = 60 * 30;
const RESET_PASSWORD_TOKEN_SECONDS: i64 = 60 * 15;
const WEBHOOK_MAX_ATTEMPTS: u32 = 8;
const SYNC_BATCH_LIMIT: usize = 25;
const MAX_BATCH_ITEMS: usize = 1_000;
const STORAGE_CONSISTENCY_SAMPLE_SIZE: usize = 8;
const DOWNLOAD_BODY_CHUNK_BYTES: usize = 64 * 1024;
const WEBHOOK_EVENT_TYPES: [&str; 4] = [
    "media.uploaded",
    "media.metadata_updated",
    "media.delete_scheduled",
    "media.deleted",
];
const ACCESS_KEY_PERMISSIONS: [&str; 9] = [
    "application:read",
    "bucket:list",
    "bucket:manage",
    "media:list",
    "media:read",
    "media:upload",
    "media:update",
    "media:delete",
    "webhook:manage",
];

type HmacSha256 = Hmac<Sha256>;

struct MediaUrlSigner {
    key: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedMediaUrlToken {
    media_id: String,
    expires_at: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SignedUploadUrlToken {
    upload_session_id: String,
    expires_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SignedMediaUrlError {
    Invalid,
    Expired,
}

impl MediaUrlSigner {
    fn new(key: Vec<u8>) -> Self {
        Self { key }
    }

    fn sign(&self, media_id: MediaId, expires_at: OffsetDateTime) -> String {
        let payload = serde_json::to_vec(&SignedMediaUrlToken {
            media_id: media_id.to_string(),
            expires_at: expires_at.unix_timestamp(),
        })
        .expect("signed media URL payload serializes");
        let payload = URL_SAFE_NO_PAD.encode(payload);
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts keys of any size");
        mac.update(payload.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{payload}.{signature}")
    }

    fn verify(
        &self,
        token: &str,
        expected_media_id: MediaId,
        now: OffsetDateTime,
    ) -> Result<(), SignedMediaUrlError> {
        if token.is_empty() || token.len() > 2048 {
            return Err(SignedMediaUrlError::Invalid);
        }
        let mut parts = token.split('.');
        let (Some(payload), Some(signature), None) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(SignedMediaUrlError::Invalid);
        };
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| SignedMediaUrlError::Invalid)?;
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts keys of any size");
        mac.update(payload.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| SignedMediaUrlError::Invalid)?;
        let payload = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| SignedMediaUrlError::Invalid)?;
        let token = serde_json::from_slice::<SignedMediaUrlToken>(&payload)
            .map_err(|_| SignedMediaUrlError::Invalid)?;
        if token.media_id != expected_media_id.to_string() {
            return Err(SignedMediaUrlError::Invalid);
        }
        if token.expires_at <= now.unix_timestamp() {
            return Err(SignedMediaUrlError::Expired);
        }
        Ok(())
    }

    fn sign_upload_content(
        &self,
        upload_session_id: UploadSessionId,
        expires_at: OffsetDateTime,
    ) -> String {
        let payload = serde_json::to_vec(&SignedUploadUrlToken {
            upload_session_id: upload_session_id.to_string(),
            expires_at: expires_at.unix_timestamp(),
        })
        .expect("signed upload URL payload serializes");
        let payload = URL_SAFE_NO_PAD.encode(payload);
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts keys of any size");
        mac.update(b"mediahub-upload-content-v1\nPUT\n");
        mac.update(payload.as_bytes());
        let signature = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{payload}.{signature}")
    }

    fn verify_upload_content(
        &self,
        token: &str,
        expected_upload_session_id: UploadSessionId,
        now: OffsetDateTime,
    ) -> Result<(), SignedMediaUrlError> {
        if token.is_empty() || token.len() > 2048 {
            return Err(SignedMediaUrlError::Invalid);
        }
        let mut parts = token.split('.');
        let (Some(payload), Some(signature), None) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(SignedMediaUrlError::Invalid);
        };
        let signature = URL_SAFE_NO_PAD
            .decode(signature)
            .map_err(|_| SignedMediaUrlError::Invalid)?;
        let mut mac = HmacSha256::new_from_slice(&self.key).expect("HMAC accepts keys of any size");
        mac.update(b"mediahub-upload-content-v1\nPUT\n");
        mac.update(payload.as_bytes());
        mac.verify_slice(&signature)
            .map_err(|_| SignedMediaUrlError::Invalid)?;
        let payload = URL_SAFE_NO_PAD
            .decode(payload)
            .map_err(|_| SignedMediaUrlError::Invalid)?;
        let token = serde_json::from_slice::<SignedUploadUrlToken>(&payload)
            .map_err(|_| SignedMediaUrlError::Invalid)?;
        if token.upload_session_id != expected_upload_session_id.to_string() {
            return Err(SignedMediaUrlError::Invalid);
        }
        if token.expires_at <= now.unix_timestamp() {
            return Err(SignedMediaUrlError::Expired);
        }
        Ok(())
    }
}

#[derive(Clone)]
struct AppState {
    repository: PostgresRepository,
    object_store: RuntimeObjectStore,
    webdav: webdav::WebDavService,
    access_key_cipher: Arc<AccessKeyCipher>,
    media_url_signer: Arc<MediaUrlSigner>,
    cookie_config: CookieConfig,
    cors_allowed_origins: Vec<HeaderValue>,
    registration_enabled: bool,
    expose_auth_tokens: bool,
    email_provider: Option<Arc<ResendEmailProvider>>,
    auth_rate_limiter: AuthRateLimiter,
    variant_slots: Arc<tokio::sync::Semaphore>,
    http_metrics: HttpMetrics,
    metrics_bearer_token: Option<Arc<str>>,
}

#[derive(Clone, Default)]
struct HttpMetrics {
    requests: Arc<AtomicU64>,
    errors: Arc<AtomicU64>,
    duration_micros: Arc<AtomicU64>,
    uploaded_bytes: Arc<AtomicU64>,
    variant_cache_hits: Arc<AtomicU64>,
    variant_cache_misses: Arc<AtomicU64>,
}

#[derive(Clone, Default)]
struct AuthRateLimiter {
    buckets: Arc<Mutex<HashMap<String, AuthRateBucket>>>,
}

struct AuthRateBucket {
    window_started: Instant,
    attempts: u32,
}

fn validate_upload_expected_size(expected_size: u64) -> Result<(), ApiError> {
    if expected_size == 0 {
        return Err(ApiError::bad_request(
            "expected_size must be greater than zero",
        ));
    }
    if expected_size > MAX_UPLOAD_OBJECT_BYTES {
        return Err(ApiError::payload_too_large(
            "expected_size exceeds the 2 GiB object limit",
        ));
    }
    Ok(())
}

// Keep each responsibility in its own source file while retaining the original
// crate-level visibility until the boundaries can be tightened safely.
include!("api_error.rs");
include!("api_types.rs");
include!("media_http.rs");
include!("handlers_webhooks.rs");
include!("handlers_path.rs");
include!("handlers_media.rs");
include!("handlers_admin_auth.rs");
include!("http.rs");

mod workers {
    use super::*;
    include!("workers.rs");
}

#[cfg(test)]
use workers::validate_referenced_key_versions;
use workers::{execute_batch_action, validate_storage_database_consistency};

mod bootstrap {
    use super::*;
    include!("bootstrap.rs");
}

#[cfg(test)]
mod tests;

// Keep the binary entrypoint small; startup wiring lives in bootstrap.rs.
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    bootstrap::run().await
}
