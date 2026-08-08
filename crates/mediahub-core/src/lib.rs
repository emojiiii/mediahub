//! MediaHub's runtime-independent domain model.
//!
//! This crate deliberately has no dependency on HTTP, async runtimes, database
//! clients, object stores, or local filesystem paths. Runtime adapters convert
//! their inputs into these types before applying domain rules.

mod async_job;
mod bucket;
mod error;
mod ids;
mod media;
mod metadata;
mod object_version;
mod s3_bucket;
mod s3_cors;
mod s3_lifecycle;
mod s3_policy;
mod s3_storage;
mod s3_tagging;
mod upload_session;
mod variant;
mod visibility;

pub use async_job::{
    AsyncJob, AsyncJobAction, AsyncJobError, AsyncJobFailureDisposition, AsyncJobId,
    AsyncJobItemResult, AsyncJobItemState, AsyncJobResult, AsyncJobState, AsyncJobTransition,
    MAX_ASYNC_JOB_ATTEMPTS, MAX_ASYNC_JOB_ERROR_BYTES, MAX_ASYNC_JOB_IDEMPOTENCY_KEY_BYTES,
    MAX_ASYNC_JOB_ITEMS, MAX_ASYNC_JOB_LEASE_SECONDS, MAX_ASYNC_JOB_OPERATION_SCOPE_BYTES,
    MAX_ASYNC_JOB_REQUEST_ID_BYTES, NewAsyncJob, PersistedAsyncJob,
};
pub use bucket::{Bucket, BucketPolicy, LifecycleRule, MAX_LIFECYCLE_RULES};
pub use error::{DomainError, DomainResult};
pub use ids::{AccessKeyId, ApplicationId, BucketId, MediaId, UploadSessionId, UserId, VariantId};
pub use media::{
    Media, MediaDimensions, MediaState, NewMedia, PersistedMedia, PersistedSystemMetadata,
};
pub use metadata::{
    CURRENT_METADATA_VERSION, ClientMetadata, MAX_METADATA_BYTES, MAX_METADATA_DEPTH,
    MAX_METADATA_KEYS, MAX_METADATA_STRING_BYTES, MediaMetadata, SystemMetadata,
};
pub use object_version::{
    ObjectId, ObjectRetention, ObjectRetentionUpdateError, ObjectVersion, ObjectVersionId,
    ObjectVersionPayload, ObjectVersionState, PersistedObjectVersion, PersistedS3Object, S3Object,
    S3VersionId, SourceProtocol, StoredObjectVersion,
};
pub use s3_bucket::{
    BucketS3Configuration, DefaultRetention, DefaultRetentionPeriod,
    PersistedBucketS3Configuration, PersistedS3Bucket, RetentionMode, S3Bucket, S3ModelError,
    VersioningStatus,
};
pub use s3_cors::{
    MAX_S3_CORS_HEADER_BYTES, MAX_S3_CORS_HEADERS_PER_RULE, MAX_S3_CORS_ID_CHARACTERS,
    MAX_S3_CORS_ORIGIN_BYTES, MAX_S3_CORS_ORIGINS_PER_RULE, MAX_S3_CORS_RULES, S3CorsConfiguration,
    S3CorsError, S3CorsMethod, S3CorsRule,
};
pub use s3_lifecycle::{
    MAX_S3_LIFECYCLE_RULES, S3AbortIncompleteMultipartUpload, S3Expiration,
    S3LifecycleConfiguration, S3LifecycleFilter, S3LifecycleRule, S3LifecycleRuleStatus,
    S3NoncurrentVersionExpiration,
};
pub use s3_policy::{
    MAX_S3_POLICY_ACTIONS, MAX_S3_POLICY_BYTES, MAX_S3_POLICY_CONDITION_KEYS,
    MAX_S3_POLICY_CONDITION_OPERATORS, MAX_S3_POLICY_CONDITION_VALUES, MAX_S3_POLICY_DEPTH,
    MAX_S3_POLICY_PRINCIPALS, MAX_S3_POLICY_RESOURCES, MAX_S3_POLICY_RUNTIME_KEY_BYTES,
    MAX_S3_POLICY_SID_BYTES, MAX_S3_POLICY_STATEMENTS, MAX_S3_POLICY_STRING_BYTES,
    S3_POLICY_LEGACY_VERSION, S3_POLICY_VERSION, S3ActionClause, S3AwsPrincipal, S3BucketPolicy,
    S3BucketPolicyStatus, S3ConditionKey, S3ConditionOperator, S3IdentityPolicy,
    S3IdentityPolicyRequest, S3IdentityPolicyResource, S3IdentityPolicyStatement, S3PolicyAction,
    S3PolicyCondition, S3PolicyDecision, S3PolicyEffect, S3PolicyError, S3PolicyErrorCode,
    S3PolicyErrorKind, S3PolicyPrincipal, S3PolicyQuery, S3PolicyRequest, S3PolicyResourceScope,
    S3PolicyStatement, S3PrincipalClause, S3ResourceClause,
};
pub use s3_storage::{
    Checksum, ChecksumAlgorithm, EntityTag, NewStorageGcTask, PersistedUploadIntent,
    StorageGcReason, StorageGcTask, StorageGcTaskId, StorageGcTaskState, UploadIntent,
    UploadIntentId, UploadIntentState,
};
pub use s3_tagging::{
    MAX_S3_BUCKET_TAGS, MAX_S3_OBJECT_TAG_KEY_CHARS, MAX_S3_OBJECT_TAG_VALUE_CHARS,
    MAX_S3_OBJECT_TAGS, S3BucketTagSet, S3ObjectTag, S3ObjectTagSet, S3TaggingError,
};
pub use upload_session::{
    NewUploadSession, PersistedUploadSession, UploadSession, UploadSessionState,
    UploadSessionTransition,
};
pub use variant::{
    CropPosition, MAX_VARIANT_DIMENSION, MAX_VARIANT_OUTPUT_PIXELS, VariantError, VariantFit,
    VariantFormat, VariantTransform,
};
pub use visibility::Visibility;

pub use time::OffsetDateTime;
