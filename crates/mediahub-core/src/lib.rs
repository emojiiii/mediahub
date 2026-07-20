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
