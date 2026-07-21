use std::{collections::BTreeMap, fmt, ops::Range};

use async_trait::async_trait;
use mediahub_core::{
    ApplicationId, Bucket, BucketId, Media, MediaId, OffsetDateTime, UploadSession, UploadSessionId,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    Redacted,
    upload_session::{PreparedUpload, StoredUpload},
};

/// Backend-neutral facts returned by `head` and prefix listing operations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectMetadata {
    pub key: String,
    pub size: u64,
    pub content_type: Option<String>,
    pub etag: Option<String>,
    pub version: Option<String>,
    pub checksum_sha256: Option<String>,
    pub provider_metadata: BTreeMap<String, String>,
}

/// One stable, lexicographically ordered page of object metadata.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectPage {
    pub objects: Vec<ObjectMetadata>,
    pub next_cursor: Option<String>,
}

/// One ordinary upload lease claimed for recovery. The opaque token fences
/// every commit and rollback after ownership changes between instances.
#[derive(Clone)]
pub struct LeasedMediaUpload {
    pub media: Media,
    pub temporary_key: String,
    pub lease_token: String,
    pub leased_until: OffsetDateTime,
}

impl fmt::Debug for LeasedMediaUpload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeasedMediaUpload")
            .field("media", &self.media)
            .field("temporary_key", &Redacted(&self.temporary_key))
            .field("lease_token", &Redacted(&self.lease_token))
            .field("leased_until", &self.leased_until)
            .finish()
    }
}

/// Facts derived while composing ordered temporary objects into one staged
/// object. The SHA-256 is calculated from the complete byte sequence and is
/// independent from provider multipart ETags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposedObject {
    pub size: u64,
    pub sha256: String,
}

/// Facts calculated while streaming one request body into temporary storage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StreamedObject {
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StreamingUploadError {
    #[error("uploaded content size {actual} does not match expected size {expected}")]
    SizeMismatch { expected: u64, actual: u64 },

    #[error("upload stream failed: {0}")]
    Stream(String),

    #[error(transparent)]
    Storage(#[from] ObjectStoreError),
}

/// Logical object storage. Implementations may use temporary files or native
/// multipart uploads internally, but the application sees only opaque keys.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait ObjectStore: Send + Sync {
    /// Identifies the configured storage implementation persisted with media.
    fn backend_name(&self) -> &str;

    /// Writes data that is not visible to readers until it is promoted.
    async fn put_temporary(
        &self,
        temporary_key: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError>;

    /// Concatenates ordered temporary source objects into a new temporary
    /// object without materializing the complete result in application memory.
    async fn compose_temporary(
        &self,
        temporary_key: &str,
        source_keys: &[String],
        content_type: &str,
    ) -> Result<ComposedObject, ObjectStoreError>;

    /// Makes a previously staged object available under its final storage key.
    async fn commit_temporary(
        &self,
        temporary_key: &str,
        final_key: &str,
    ) -> Result<(), ObjectStoreError>;

    /// Reads a committed immutable object in full.
    async fn read(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError>;

    /// Reads a half-open byte range without exposing backend range syntax.
    async fn read_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ObjectStoreError>;

    /// Returns immutable object facts without treating ETag as a checksum.
    async fn head(&self, key: &str) -> Result<ObjectMetadata, ObjectStoreError>;

    /// Returns a trustworthy SHA-256 without requiring the caller to buffer
    /// the complete object. Backends may use verified provider metadata or a
    /// streaming digest.
    async fn checksum_sha256(&self, key: &str) -> Result<String, ObjectStoreError> {
        self.head(key).await?.checksum_sha256.ok_or_else(|| {
            ObjectStoreError::Unavailable(
                "object backend did not provide a trustworthy SHA-256".into(),
            )
        })
    }

    /// Lists one lexicographically ordered page below a relative prefix.
    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ObjectPage, ObjectStoreError>;

    /// Deletes either a temporary or final object. Repeated deletion is safe.
    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError>;

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError>;
}

/// Backend-neutral direct-upload operations. Adapters may create a presigned
/// PUT URL, a multipart target, or an internal gateway target; the service
/// treats the returned target and storage key as opaque.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait UploadSessionStorage: Send + Sync {
    async fn prepare_upload(
        &self,
        upload_session_id: UploadSessionId,
        media_id: MediaId,
        expected_size: u64,
        expected_mime: &str,
        expires_at: OffsetDateTime,
    ) -> Result<PreparedUpload, ObjectStoreError>;

    /// Reads immutable object facts after client transfer. The adapter must
    /// independently calculate or retrieve a trustworthy SHA-256; ETag alone
    /// is insufficient unless its algorithm is explicitly proven.
    async fn inspect_upload(
        &self,
        session: &UploadSession,
    ) -> Result<StoredUpload, ObjectStoreError>;

    /// Promotes a verified session-scoped temporary object to the immutable
    /// media key. Implementations must make this operation idempotent and
    /// must never overwrite an existing final object.
    async fn finalize_upload(
        &self,
        _session: &UploadSession,
        _final_storage_key: &str,
    ) -> Result<(), ObjectStoreError> {
        Ok(())
    }

    /// Terminates multipart state and removes uncommitted objects. Repeated
    /// calls must be safe because cancellation and expiry are retried.
    async fn abort_upload(&self, session: &UploadSession) -> Result<(), ObjectStoreError>;
}

/// Media persistence and quota accounting. `commit_upload` is one durable
/// transaction: it transitions media, transfers reserved bytes to used bytes,
/// and persists the supplied outbox event together.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait MediaRepository: Send + Sync {
    async fn find_by_object_key(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        object_key: &str,
    ) -> Result<Option<Media>, RepositoryError>;

    /// Atomically claims expired ordinary uploads for reconciliation. Each
    /// returned row owns a freshly rotated fencing token. Multipart uploads
    /// retain their separate completion protocol and must be excluded.
    async fn claim_stale_uploading(
        &self,
        now: OffsetDateTime,
        leased_until: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<LeasedMediaUpload>, RepositoryError>;

    /// Atomically checks available quota and increases reserved bytes.
    async fn reserve_quota(
        &self,
        application_id: ApplicationId,
        bytes: u64,
    ) -> Result<(), RepositoryError>;

    /// Persists an `uploading` media record. The unique key constraint must
    /// cover `(application_id, bucket_id, object_key)`.
    async fn create_uploading(
        &self,
        media: Media,
        temporary_key: &str,
        lease_token: &str,
        leased_until: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    /// Extends an unexpired ordinary upload lease without rotating its token.
    /// Returns `false` after ownership is lost or the row leaves `uploading`.
    async fn renew_upload_lease(
        &self,
        media_id: MediaId,
        lease_token: &str,
        now: OffsetDateTime,
        leased_until: OffsetDateTime,
    ) -> Result<bool, RepositoryError>;

    /// Atomically activates the media, commits its reservation, and writes the
    /// outbox event. It must reject any record not in `uploading` state.
    async fn commit_upload(
        &self,
        media_id: MediaId,
        lease_token: &str,
        committed_at: OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<Media, RepositoryError>;

    /// Removes an uploading record and releases its reservation in one durable
    /// operation. Calling it after a partial failure must be idempotent.
    async fn abort_upload(
        &self,
        media_id: MediaId,
        lease_token: &str,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    /// Releases a reservation when no media record was successfully created.
    async fn release_quota(
        &self,
        application_id: ApplicationId,
        bytes: u64,
    ) -> Result<(), RepositoryError>;

    /// Applies a metadata-only mutation when its expected revision matches.
    async fn update_media(
        &self,
        media: Media,
        expected_revision: u64,
        event: OutboxEvent,
    ) -> Result<(), RepositoryError>;

    /// Marks an active object unavailable and schedules its physical deletion.
    /// The implementation must append the event in the same transaction.
    async fn schedule_delete(
        &self,
        media_id: MediaId,
        deleted_at: OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<Media, RepositoryError>;
}

/// Persistence boundary for the direct-upload lifecycle. Each mutating method
/// is a single durable operation: it owns the session state transition and the
/// corresponding quota transfer or release. Repeating a terminal operation
/// must return its existing outcome without charging or releasing quota again.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait UploadSessionRepository: Send + Sync {
    /// Atomically reserves `session.reserved_bytes()` and creates a pending
    /// session. It must reject conflicts with media or other pending sessions
    /// at the same `(application, bucket, object_key)`.
    async fn create_upload_session(&self, session: UploadSession) -> Result<(), RepositoryError>;

    async fn find_upload_session(
        &self,
        upload_session_id: UploadSessionId,
    ) -> Result<Option<UploadSession>, RepositoryError>;

    /// Atomically marks a session complete, activates `media`, transfers the
    /// reservation to used quota, and writes `event`. A repeated complete
    /// operation returns `AlreadyCompleted` with the original media.
    async fn complete_upload_session(
        &self,
        upload_session_id: UploadSessionId,
        media: Media,
        completed_at: OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<UploadSessionCompletion, RepositoryError>;

    async fn completed_upload_media(
        &self,
        upload_session_id: UploadSessionId,
    ) -> Result<Option<Media>, RepositoryError>;

    /// Atomically cancels a pending session and releases its reservation.
    async fn cancel_upload_session(
        &self,
        upload_session_id: UploadSessionId,
        cancelled_at: OffsetDateTime,
    ) -> Result<UploadSessionCancellation, RepositoryError>;

    /// Atomically expires one due session and releases its reservation.
    async fn expire_upload_session(
        &self,
        upload_session_id: UploadSessionId,
        expired_at: OffsetDateTime,
    ) -> Result<UploadSessionExpiration, RepositoryError>;

    /// Atomically expires up to `limit` pending sessions that are due. Every
    /// returned session has already released its reservation exactly once.
    async fn expire_upload_sessions(
        &self,
        expired_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<UploadSession>, RepositoryError>;

    /// Acknowledges physical object cleanup after a terminal session has been
    /// expired or cancelled. Unacknowledged terminal rows are returned by the
    /// next expiry scan so a transient storage failure is retried.
    async fn complete_upload_session_cleanup(
        &self,
        upload_session_id: UploadSessionId,
    ) -> Result<bool, RepositoryError>;
}

#[derive(Clone, Debug)]
pub enum UploadSessionCompletion {
    Completed(Media),
    AlreadyCompleted(Media),
    Cancelled,
    Expired,
}

#[derive(Clone, Debug)]
pub enum UploadSessionCancellation {
    Cancelled(UploadSession),
    AlreadyCancelled(UploadSession),
    Completed,
    Expired,
}

#[derive(Clone, Debug)]
pub enum UploadSessionExpiration {
    Expired(UploadSession),
    AlreadyExpired(UploadSession),
    Completed,
    Cancelled,
    NotDue,
}

/// Bucket lookup is scoped by the calling application in the use case, not by
/// a client-controlled bucket name.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait BucketRepository: Send + Sync {
    async fn find_by_id(&self, bucket_id: BucketId) -> Result<Option<Bucket>, RepositoryError>;
}

/// Port used by outbox workers. Upload commit writes events through the media
/// transaction above, while workers use this port to claim and acknowledge
/// persisted events.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait OutboxRepository: Send + Sync {
    async fn list_pending(
        &self,
        now: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<OutboxEvent>, RepositoryError>;

    async fn mark_delivered(
        &self,
        event_id: &str,
        delivered_at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    async fn mark_failed(
        &self,
        event_id: &str,
        retry_at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;
}

/// Port used by webhook workers to process each endpoint independently.
/// Implementations must fence acknowledgements with the lease token so a
/// worker cannot complete a delivery after another worker has reclaimed it.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait WebhookDeliveryRepository: Send + Sync {
    async fn materialize_webhook_deliveries(&self, event_id: &str) -> Result<u64, RepositoryError>;

    async fn finalize_unsubscribed_outbox_events(
        &self,
        limit: usize,
    ) -> Result<u64, RepositoryError>;

    async fn claim_webhook_deliveries(
        &self,
        now: OffsetDateTime,
        lease_until: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<LeasedWebhookDelivery>, RepositoryError>;

    async fn mark_webhook_delivery_delivered(
        &self,
        event_id: &str,
        endpoint_id: &str,
        lease_token: &str,
        delivered_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError>;

    async fn mark_webhook_delivery_delivered_with_status(
        &self,
        event_id: &str,
        endpoint_id: &str,
        lease_token: &str,
        delivered_at: OffsetDateTime,
        response_status: Option<u16>,
    ) -> Result<bool, RepositoryError>;

    #[allow(clippy::too_many_arguments)]
    async fn record_webhook_delivery_failure(
        &self,
        event_id: &str,
        endpoint_id: &str,
        lease_token: &str,
        failed_at: OffsetDateTime,
        retry_at: OffsetDateTime,
        max_attempts: u32,
        last_error: &str,
    ) -> Result<Option<WebhookDeliveryFailureDisposition>, RepositoryError>;

    #[allow(clippy::too_many_arguments)]
    async fn record_webhook_delivery_failure_with_status(
        &self,
        event_id: &str,
        endpoint_id: &str,
        lease_token: &str,
        failed_at: OffsetDateTime,
        retry_at: OffsetDateTime,
        max_attempts: u32,
        response_status: Option<u16>,
        last_error: &str,
    ) -> Result<Option<WebhookDeliveryFailureDisposition>, RepositoryError>;
}

/// Source of application time, making lifecycle-sensitive services testable.
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEvent {
    pub id: String,
    pub application_id: ApplicationId,
    pub event_type: String,
    pub aggregate_id: String,
    pub payload: serde_json::Value,
    pub created_at: OffsetDateTime,
    pub delivered_at: Option<OffsetDateTime>,
    pub next_attempt_at: Option<OffsetDateTime>,
    pub attempt_count: u32,
}

/// Endpoint data required to perform one webhook delivery. The encrypted
/// secret remains opaque to the application layer and is decrypted only by
/// the server worker immediately before signing a request.
#[derive(Clone, PartialEq, Eq)]
pub struct WebhookDeliveryEndpoint {
    pub id: String,
    pub application_id: ApplicationId,
    pub url: String,
    pub secret_ciphertext: String,
    pub secret_key_version: u32,
}

impl fmt::Debug for WebhookDeliveryEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("WebhookDeliveryEndpoint")
            .field("id", &self.id)
            .field("application_id", &self.application_id)
            .field("url", &self.url)
            .field("secret_ciphertext", &Redacted(&self.secret_ciphertext))
            .field("secret_key_version", &self.secret_key_version)
            .finish()
    }
}

/// Durable state for one Outbox event and one subscribed webhook endpoint.
#[derive(Clone, Debug, PartialEq)]
pub struct WebhookDelivery {
    pub event: OutboxEvent,
    pub endpoint: WebhookDeliveryEndpoint,
    pub attempt_count: u32,
    pub next_attempt_at: Option<OffsetDateTime>,
    pub delivered_at: Option<OffsetDateTime>,
    pub dead_lettered_at: Option<OffsetDateTime>,
    pub last_error: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// A delivery claim fenced by a unique lease token.
#[derive(Clone, PartialEq)]
pub struct LeasedWebhookDelivery {
    pub delivery: WebhookDelivery,
    pub lease_token: String,
    pub leased_until: OffsetDateTime,
}

impl fmt::Debug for LeasedWebhookDelivery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeasedWebhookDelivery")
            .field("delivery", &self.delivery)
            .field("lease_token", &Redacted(&self.lease_token))
            .field("leased_until", &self.leased_until)
            .finish()
    }
}

/// Result of recording a failed delivery attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebhookDeliveryFailureDisposition {
    RetryScheduled {
        attempt_count: u32,
        next_attempt_at: OffsetDateTime,
    },
    DeadLettered {
        attempt_count: u32,
        dead_lettered_at: OffsetDateTime,
    },
}

impl OutboxEvent {
    #[must_use]
    pub fn media_uploaded(media: &Media, created_at: OffsetDateTime) -> Self {
        Self {
            // Object keys cannot be reused, so this is stable across a replayed
            // commit while still being distinct for every uploaded media item.
            id: format!("media.uploaded:{}", media.id()),
            application_id: media.application_id(),
            event_type: "media.uploaded".to_owned(),
            aggregate_id: media.id().to_string(),
            payload: serde_json::json!({
                "media_id": media.id().to_string(),
                "bucket_id": media.bucket_id().to_string(),
                "object_key": media.object_key(),
                "size": media.size(),
                "mime": media.mime(),
            }),
            created_at,
            delivered_at: None,
            next_attempt_at: Some(created_at),
            attempt_count: 0,
        }
    }

    #[must_use]
    pub fn media_delete_scheduled(media: &Media, created_at: OffsetDateTime, reason: &str) -> Self {
        Self {
            id: format!("media.delete_scheduled:{}", media.id()),
            application_id: media.application_id(),
            event_type: "media.delete_scheduled".to_owned(),
            aggregate_id: media.id().to_string(),
            payload: serde_json::json!({
                "media_id": media.id().to_string(),
                "bucket_id": media.bucket_id().to_string(),
                "object_key": media.object_key(),
                "reason": reason,
            }),
            created_at,
            delivered_at: None,
            next_attempt_at: Some(created_at),
            attempt_count: 0,
        }
    }

    #[must_use]
    pub fn media_metadata_updated(media: &Media, created_at: OffsetDateTime) -> Self {
        Self {
            id: format!("media.metadata_updated:{}:{}", media.id(), media.revision()),
            application_id: media.application_id(),
            event_type: "media.metadata_updated".to_owned(),
            aggregate_id: media.id().to_string(),
            payload: serde_json::json!({
                "media_id": media.id().to_string(),
                "revision": media.revision(),
            }),
            created_at,
            delivered_at: None,
            next_attempt_at: Some(created_at),
            attempt_count: 0,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RepositoryError {
    #[error("record was not found")]
    NotFound,

    #[error("concurrent write conflict")]
    Conflict,

    #[error("quota has insufficient available bytes")]
    QuotaExceeded,

    #[error("repository invariant was violated: {0}")]
    Invariant(String),

    #[error("repository failed: {0}")]
    Unavailable(String),
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ObjectStoreError {
    #[error("object was not found")]
    NotFound,

    #[error("object key already exists")]
    AlreadyExists,

    #[error("object byte range is invalid")]
    InvalidRange,

    #[error("object list cursor is invalid")]
    InvalidCursor,

    #[error("object list limit is invalid")]
    InvalidLimit,

    #[error("object store failed: {0}")]
    Unavailable(String),
}
