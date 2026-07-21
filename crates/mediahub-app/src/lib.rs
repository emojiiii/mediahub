//! Application services and runtime-independent ports for `MediaHub`.
//!
//! Runtime entries call the services in this crate after authentication. The
//! adapters behind the ports own database connections, filesystems, queues and
//! cloud bindings; none of those runtime details are exposed here.

mod administration;
mod async_job;
mod control_plane;
mod data_plane;
mod error;
mod image;
mod memory;
#[cfg(feature = "object-store-contract-tests")]
#[doc(hidden)]
pub mod object_store_contract;
mod ports;
mod s3_multipart;
mod upload;
mod upload_session;
mod variant;
mod webhook;

pub use administration::{
    AdminApplicationSummary, AdminBootstrapOutcome, AdminJobSummary, AdminMetricsSnapshot,
    AdminRepository, AdminStorageSummary, AdminSystemSettings, AdminUserSummary, AuditEvent,
    AuditRepository, DEFAULT_DOWNLOAD_BYTES_PER_SECOND, MAX_DOWNLOAD_BYTES_PER_SECOND,
    MIN_DOWNLOAD_BYTES_PER_SECOND, SecretKeyVersionRepository,
};
pub use async_job::{
    AsyncJobApplicationError, AsyncJobCancellation, AsyncJobCompletion, AsyncJobCreation,
    AsyncJobDetails, AsyncJobFailure, AsyncJobReceipt, AsyncJobRepository, AsyncJobService,
    CancelAsyncJobRequest, CompleteAsyncJobRequest, CreateAsyncJobRequest,
    DEFAULT_ASYNC_JOB_LEASE_SECONDS, DEFAULT_ASYNC_JOB_MAX_ATTEMPTS, FailAsyncJobRequest,
    LeasedAsyncJob, MAX_ASYNC_JOB_CLAIM_LIMIT,
};
pub use control_plane::{
    AccessKeyRecord, AccessKeyRepository, ApplicationRepository, ApplicationSummary,
    AuthRepository, NewAccessKey, OneTimeTokenPurpose, SessionRecord, UserAccount,
};
pub use data_plane::{
    CompletedIdempotencyResponse, IdempotencyClaim, IdempotencyContext, MediaDirectoryListCursor,
    MediaDirectoryListQuery, MediaDirectoryPage, MediaListCursor, MediaListQuery, MediaPage,
    PendingMediaDeletion, S3MediaListQuery, S3MediaPage,
};
pub use error::ApplicationError;
pub(crate) use error::Redacted;
pub use image::{ImageProcessor, ImageProcessorError, ProcessedVariant, variant_cache_key};
pub use memory::{
    FixedClock, InMemoryBucketRepository, InMemoryMediaRepository, InMemoryObjectStore,
    QuotaSnapshot,
};
pub use ports::{
    BucketRepository, Clock, ComposedObject, LeasedMediaUpload, LeasedWebhookDelivery,
    MediaRepository, ObjectMetadata, ObjectPage, ObjectStore, ObjectStoreError, OutboxEvent,
    OutboxRepository, RepositoryError, StreamedObject, StreamingUploadError,
    UploadSessionCancellation, UploadSessionCompletion, UploadSessionExpiration,
    UploadSessionRepository, UploadSessionStorage, WebhookDelivery, WebhookDeliveryEndpoint,
    WebhookDeliveryFailureDisposition, WebhookDeliveryRepository,
};
pub use s3_multipart::{
    CompletedS3MultipartPart, MAX_S3_MULTIPART_ACTIVE_UPLOADS_PER_APPLICATION,
    MAX_S3_MULTIPART_EXPIRY_LIMIT, MAX_S3_MULTIPART_PART_NUMBER, MIN_S3_MULTIPART_PART_NUMBER,
    NewS3MultipartPart, NewS3MultipartUpload, S3MultipartAbort, S3MultipartCompletionClaim,
    S3MultipartCompletionFinish, S3MultipartCompletionRelease, S3MultipartExpiredUpload,
    S3MultipartManifest, S3MultipartManifestError, S3MultipartPart, S3MultipartPartPut,
    S3MultipartRepository, S3MultipartUpload, S3MultipartUploadState,
};
pub use upload::{
    MEDIA_UPLOAD_HEARTBEAT_SECONDS, MEDIA_UPLOAD_LEASE_SECONDS, StagedUploadMediaRequest,
    UploadMediaRequest, UploadMediaService, UploadReceipt,
};
pub use upload_session::{
    CancelUploadSessionReceipt, CancelUploadSessionRequest, CompleteUploadSessionRequest,
    CompletedUploadSessionReceipt, CreateUploadSessionRequest, DEFAULT_UPLOAD_SESSION_TTL,
    ExpireUploadSessionsReceipt, PreparedUpload, StoredUpload, UploadSessionReceipt,
    UploadSessionService, UploadTarget,
};
pub use variant::{
    DEFAULT_VARIANT_LEASE_SECONDS, GenerateVariantRequest, NewVariant, VariantApplicationError,
    VariantClaim, VariantReceipt, VariantRecord, VariantRepository, VariantService, VariantState,
};
pub use webhook::{
    NewWebhookEndpoint, WebhookDeliveryHistoryCursor, WebhookDeliveryHistoryItem,
    WebhookDeliveryHistoryPage, WebhookDeliveryHistoryQuery, WebhookDeliveryHistoryStatus,
    WebhookEndpoint, WebhookEndpointRepository, WebhookEndpointUpdate,
};
