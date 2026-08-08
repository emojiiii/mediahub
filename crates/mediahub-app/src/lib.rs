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
#[cfg(test)]
mod memory_s3;
#[cfg(feature = "object-store-contract-tests")]
#[doc(hidden)]
pub mod object_store_contract;
mod ports;
mod s3_lifecycle;
mod s3_multipart;
mod s3_object_service;
mod s3_repository;
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
pub use s3_lifecycle::{
    DEFAULT_S3_LIFECYCLE_BATCH_SIZE, DEFAULT_S3_LIFECYCLE_GC_MAX_ATTEMPTS,
    ExecuteS3LifecycleCommand, MAX_S3_LIFECYCLE_BATCH_SIZE, S3CurrentExpirationCandidate,
    S3ExpiredDeleteMarkerCandidate, S3LifecycleBatchCursor, S3LifecycleBatchReceipt,
    S3LifecycleExecutionOutcome, S3LifecycleRepository, S3LifecycleService, S3LifecycleTarget,
    S3LifecycleTargetIdentity, S3MultipartLifecycleCandidate, S3NoncurrentExpirationCandidate,
    lifecycle_action_time, lifecycle_days_cutoff, lifecycle_prefix,
    lifecycle_rule_aborts_multipart, lifecycle_rule_is_current_due,
    lifecycle_rule_is_noncurrent_due, lifecycle_rule_removes_expired_marker,
    lifecycle_rule_removes_expired_marker_at,
};
pub use s3_multipart::{
    CompletedS3MultipartPart, DEFAULT_S3_MULTIPART_GC_MAX_ATTEMPTS,
    MAX_S3_MULTIPART_ACTIVE_UPLOADS_PER_APPLICATION, MAX_S3_MULTIPART_EXPIRY_LIMIT,
    MAX_S3_MULTIPART_PART_NUMBER, MIN_S3_MULTIPART_PART_NUMBER, NewS3MultipartPart,
    NewS3MultipartUpload, S3MultipartAbort, S3MultipartCompletionClaim,
    S3MultipartCompletionRelease, S3MultipartExpiredUpload, S3MultipartManifest,
    S3MultipartManifestError, S3MultipartPart, S3MultipartPartPut, S3MultipartRepository,
    S3MultipartUpload, S3MultipartUploadState, is_lowercase_md5_hex, is_multipart_etag,
};
pub use s3_object_service::{
    AbortStagedPutRequest, BeginPutObjectReceipt, BeginPutObjectRequest, CompletePutObjectReceipt,
    CompletePutObjectRequest, DEFAULT_S3_UPLOAD_INTENT_TTL_SECONDS,
    DEFAULT_S3_UPLOAD_LEASE_SECONDS, DeleteObjectReceipt, DeleteObjectRequest,
    ListObjectVersionsRequest, NewS3ObjectLock, PrepareClaimedUploadCommitRequest,
    PutObjectLegalHoldRequest, PutObjectRetentionRequest, PutObjectTaggingRequest,
    S3GetObjectReceipt, S3HeadObjectReceipt, S3ObjectRequest, S3ObjectService,
    S3ObjectServiceError,
};
pub use s3_repository::{
    DeleteS3ObjectCommand, DeleteS3ObjectOutcome, DeletedS3ObjectVersion,
    MAX_S3_MULTIPART_UPLOAD_LIST_LIMIT, MAX_S3_OBJECT_LIST_LIMIT, MAX_S3_VERSION_LIST_LIMIT,
    MAX_STORAGE_GC_CLAIM_LIMIT, MAX_UPLOAD_INTENT_EXPIRY_LIMIT, PutS3ObjectLockCommand,
    PutS3ObjectLockOutcome, S3BucketRepository, S3DeleteLockReason, S3ListingRepository,
    S3MultipartUploadListItem, S3MultipartUploadListQuery, S3MultipartUploadPage,
    S3ObjectCommitTarget, S3ObjectListItem, S3ObjectListQuery, S3ObjectLockMutation, S3ObjectPage,
    S3ObjectRepository, S3ObjectTaggingCommand, S3ObjectTaggingOutcome, S3ObjectVersionCommit,
    S3ObjectVersionListItem, S3ObjectVersionListQuery, S3ObjectVersionPage, S3ObjectVersionRead,
    S3UploadIntentRepository, StorageGcRepository,
};
pub use upload::{
    MEDIA_UPLOAD_HEARTBEAT_SECONDS, MEDIA_UPLOAD_LEASE_SECONDS, UploadMediaRequest,
    UploadMediaService, UploadReceipt,
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
