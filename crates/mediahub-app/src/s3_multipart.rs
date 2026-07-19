use async_trait::async_trait;
use mediahub_core::{ApplicationId, BucketId, Media, MediaId, OffsetDateTime, Visibility};
use serde::{Deserialize, Serialize};

use crate::{OutboxEvent, RepositoryError};

pub const MIN_S3_MULTIPART_PART_NUMBER: u16 = 1;
pub const MAX_S3_MULTIPART_PART_NUMBER: u16 = 10_000;
pub const MAX_S3_MULTIPART_EXPIRY_LIMIT: usize = 1_000;
pub const MAX_S3_MULTIPART_ACTIVE_UPLOADS_PER_APPLICATION: usize = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3MultipartUploadState {
    Pending,
    Completing,
    Completed,
    Aborted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewS3MultipartUpload {
    pub upload_id: String,
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub content_type: String,
    pub visibility_override: Option<Visibility>,
    pub expires_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
}

impl NewS3MultipartUpload {
    pub fn validate(&self) -> Result<(), RepositoryError> {
        if self.upload_id.is_empty() || self.object_key.is_empty() || self.content_type.is_empty() {
            return Err(RepositoryError::Invariant(
                "multipart upload id, object key, and content type must not be empty".into(),
            ));
        }
        if self.expires_at <= self.created_at {
            return Err(RepositoryError::Invariant(
                "multipart upload expiry must be after creation".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3MultipartUpload {
    pub upload_id: String,
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub content_type: String,
    pub visibility_override: Option<Visibility>,
    pub state: S3MultipartUploadState,
    pub expires_at: OffsetDateTime,
    pub completion_lease_until: Option<OffsetDateTime>,
    pub media_id: Option<MediaId>,
    pub final_etag: Option<String>,
    pub completed_at: Option<OffsetDateTime>,
    pub aborted_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewS3MultipartPart {
    pub part_number: u16,
    pub size: u64,
    pub sha256: String,
    pub etag: String,
    pub storage_key: String,
}

impl NewS3MultipartPart {
    pub fn validate(&self) -> Result<(), RepositoryError> {
        if !(MIN_S3_MULTIPART_PART_NUMBER..=MAX_S3_MULTIPART_PART_NUMBER)
            .contains(&self.part_number)
        {
            return Err(RepositoryError::Invariant(
                "multipart part number must be between 1 and 10000".into(),
            ));
        }
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(RepositoryError::Invariant(
                "multipart part sha256 must be 64 hexadecimal characters".into(),
            ));
        }
        if self.etag.is_empty() || self.storage_key.is_empty() {
            return Err(RepositoryError::Invariant(
                "multipart part etag and storage key must not be empty".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3MultipartPart {
    pub upload_id: String,
    pub part_number: u16,
    pub size: u64,
    /// Independently calculated checksum for this part. This is never derived
    /// from, or substituted with, the S3 multipart ETag.
    pub sha256: String,
    pub etag: String,
    pub storage_key: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3MultipartPartPut {
    Stored {
        part: S3MultipartPart,
        replaced_storage_key: Option<String>,
    },
    NotPending(S3MultipartUpload),
    Expired {
        upload: S3MultipartUpload,
        storage_keys: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedS3MultipartPart {
    pub part_number: u16,
    pub etag: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3MultipartManifest {
    pub upload: S3MultipartUpload,
    pub parts: Vec<S3MultipartPart>,
    pub total_size: u64,
    pub unused_storage_keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3MultipartManifestError {
    Empty,
    InvalidPartNumber(u16),
    InvalidPartOrder,
    MissingPart(u16),
    EtagMismatch(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3MultipartCompletionClaim {
    Claimed(S3MultipartManifest),
    AlreadyCompleted(S3MultipartUpload),
    InProgress(S3MultipartUpload),
    Aborted(S3MultipartUpload),
    Expired {
        upload: S3MultipartUpload,
        storage_keys: Vec<String>,
    },
    InvalidManifest(S3MultipartManifestError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3MultipartCompletionFinish {
    Completed(S3MultipartUpload),
    AlreadyCompleted(S3MultipartUpload),
    OwnershipLost(S3MultipartUpload),
    NotCompleting(S3MultipartUpload),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3MultipartCompletionRelease {
    Released(S3MultipartUpload),
    AlreadyPending(S3MultipartUpload),
    OwnershipLost(S3MultipartUpload),
    Terminal(S3MultipartUpload),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3MultipartAbort {
    Aborted {
        upload: S3MultipartUpload,
        storage_keys: Vec<String>,
    },
    AlreadyAborted {
        upload: S3MultipartUpload,
        storage_keys: Vec<String>,
    },
    Completing(S3MultipartUpload),
    Completed(S3MultipartUpload),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3MultipartExpiredUpload {
    pub upload: S3MultipartUpload,
    pub storage_keys: Vec<String>,
}

/// Durable boundary for the S3 multipart lifecycle. Implementations must lock
/// the upload while replacing parts or changing state so concurrent handlers
/// cannot claim, finish, replace, or abort the same upload inconsistently.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3MultipartRepository: Send + Sync {
    async fn create_multipart_upload(
        &self,
        upload: NewS3MultipartUpload,
    ) -> Result<S3MultipartUpload, RepositoryError>;

    async fn find_multipart_upload(
        &self,
        upload_id: &str,
    ) -> Result<Option<S3MultipartUpload>, RepositoryError>;

    async fn put_multipart_part(
        &self,
        upload_id: &str,
        part: NewS3MultipartPart,
        maximum_upload_size: u64,
        now: OffsetDateTime,
    ) -> Result<S3MultipartPartPut, RepositoryError>;

    async fn list_multipart_parts(
        &self,
        upload_id: &str,
    ) -> Result<Vec<S3MultipartPart>, RepositoryError>;

    /// Atomically validates the ordered client manifest and acquires a
    /// completion lease. An expired lease may be taken over with a new token.
    async fn claim_multipart_completion(
        &self,
        upload_id: &str,
        manifest: &[CompletedS3MultipartPart],
        completion_token: &str,
        lease_until: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<S3MultipartCompletionClaim, RepositoryError>;

    /// Creates the uploading Media record for the current completion owner
    /// without reserving quota again. Uploaded parts already own the required
    /// reservation.
    async fn create_uploading_for_multipart(
        &self,
        upload_id: &str,
        completion_token: &str,
        media: Media,
    ) -> Result<(), RepositoryError>;

    /// Removes a failed uploading Media record without releasing quota. The
    /// reservation remains owned by the persisted multipart parts.
    async fn abort_uploading_for_multipart(
        &self,
        upload_id: &str,
        completion_token: &str,
        media_id: MediaId,
    ) -> Result<(), RepositoryError>;

    /// Activates the multipart Media, transfers its selected part reservation
    /// to used quota, and appends the outbox event only while the caller still
    /// owns the completion token.
    async fn commit_upload_for_multipart(
        &self,
        upload_id: &str,
        completion_token: &str,
        media_id: MediaId,
        committed_at: OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<Media, RepositoryError>;

    async fn finish_multipart_completion(
        &self,
        upload_id: &str,
        completion_token: &str,
        media_id: MediaId,
        final_etag: &str,
        now: OffsetDateTime,
    ) -> Result<S3MultipartCompletionFinish, RepositoryError>;

    async fn release_multipart_completion(
        &self,
        upload_id: &str,
        completion_token: &str,
        now: OffsetDateTime,
    ) -> Result<S3MultipartCompletionRelease, RepositoryError>;

    async fn abort_multipart_upload(
        &self,
        upload_id: &str,
        now: OffsetDateTime,
    ) -> Result<S3MultipartAbort, RepositoryError>;

    /// Atomically claims due active uploads and terminal uploads whose part
    /// cleanup must be retried. `FOR UPDATE SKIP LOCKED` implementations allow
    /// concurrent lifecycle workers to divide the work.
    async fn expire_multipart_uploads(
        &self,
        now: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3MultipartExpiredUpload>, RepositoryError>;

    /// Removes persisted part metadata after all temporary objects have been
    /// deleted. Terminal upload rows remain available for idempotent S3
    /// replays. Pending or completing uploads must be rejected as conflicts.
    async fn clear_multipart_parts(&self, upload_id: &str) -> Result<usize, RepositoryError>;
}
