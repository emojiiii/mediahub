use async_trait::async_trait;
use mediahub_core::{
    ApplicationId, BucketId, Checksum, EntityTag, ObjectId, ObjectVersionId, OffsetDateTime,
    S3ObjectTagSet, UploadIntentId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::RepositoryError;

pub const MIN_S3_MULTIPART_PART_NUMBER: u16 = 1;
pub const MAX_S3_MULTIPART_PART_NUMBER: u16 = 10_000;
pub const MAX_S3_MULTIPART_EXPIRY_LIMIT: usize = 1_000;
pub const MAX_S3_MULTIPART_ACTIVE_UPLOADS_PER_APPLICATION: usize = 1_000;
pub const DEFAULT_S3_MULTIPART_GC_MAX_ATTEMPTS: u32 = 10;

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
    pub user_metadata: Value,
    pub object_tags: S3ObjectTagSet,
    pub storage_backend: String,
    pub expires_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
}

impl NewS3MultipartUpload {
    pub fn validate(&self) -> Result<(), RepositoryError> {
        if self.upload_id.is_empty()
            || self.object_key.is_empty()
            || self.content_type.is_empty()
            || self.storage_backend.is_empty()
            || !self.user_metadata.is_object()
            || self.object_tags.validate().is_err()
        {
            return Err(RepositoryError::Invariant(
                "multipart identity, content type, backend, and metadata are invalid".into(),
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
    pub user_metadata: Value,
    pub object_tags: S3ObjectTagSet,
    pub storage_backend: String,
    pub state: S3MultipartUploadState,
    pub expires_at: OffsetDateTime,
    pub completion_lease_until: Option<OffsetDateTime>,
    pub upload_intent_id: Option<UploadIntentId>,
    pub object_id: Option<ObjectId>,
    pub object_version_id: Option<ObjectVersionId>,
    pub final_etag: Option<EntityTag>,
    pub final_checksum: Option<Checksum>,
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
    pub md5: String,
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
        if !is_sha256_hex(&self.sha256) {
            return Err(RepositoryError::Invariant(
                "multipart part sha256 must be 64 hexadecimal characters".into(),
            ));
        }
        if !is_lowercase_md5_hex(&self.md5) || self.etag != self.md5 {
            return Err(RepositoryError::Invariant(
                "multipart part md5 and etag must be the same lowercase MD5 hex value".into(),
            ));
        }
        if self.storage_key.is_empty() {
            return Err(RepositoryError::Invariant(
                "multipart part storage key must not be empty".into(),
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
    pub sha256: String,
    pub md5: String,
    /// Canonical unquoted lowercase MD5 hex.
    pub etag: String,
    pub storage_key: String,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3MultipartPartPut {
    Stored(S3MultipartPart),
    NotPending(S3MultipartUpload),
    Expired(S3MultipartUpload),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletedS3MultipartPart {
    pub part_number: u16,
    /// Canonical unquoted lowercase MD5 hex supplied by CompleteMultipartUpload.
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
    Expired(S3MultipartUpload),
    InvalidManifest(S3MultipartManifestError),
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
    Aborted(S3MultipartUpload),
    AlreadyAborted(S3MultipartUpload),
    Completing(S3MultipartUpload),
    Completed(S3MultipartUpload),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3MultipartExpiredUpload {
    pub upload: S3MultipartUpload,
}

#[must_use]
pub fn is_lowercase_md5_hex(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[must_use]
pub fn is_multipart_etag(value: &str) -> bool {
    let Some((digest, part_count)) = value.split_once('-') else {
        return false;
    };
    is_lowercase_md5_hex(digest)
        && !part_count.is_empty()
        && !part_count.starts_with('0')
        && part_count
            .parse::<u16>()
            .is_ok_and(|count| (1..=MAX_S3_MULTIPART_PART_NUMBER).contains(&count))
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Durable boundary for the S3 multipart lifecycle. Implementations lock the
/// upload while replacing parts or changing state. Storage cleanup is always
/// represented by persistent GC tasks written in the same transaction as the
/// metadata transition.
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

    async fn claim_multipart_completion(
        &self,
        upload_id: &str,
        manifest: &[CompletedS3MultipartPart],
        completion_token: &str,
        lease_until: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<S3MultipartCompletionClaim, RepositoryError>;

    /// Fenced association created after the manifest has frozen total size and
    /// the caller has persisted a matching UploadIntent.
    async fn attach_multipart_upload_intent(
        &self,
        upload_id: &str,
        completion_token: &str,
        intent_id: UploadIntentId,
        now: OffsetDateTime,
    ) -> Result<S3MultipartUpload, RepositoryError>;

    /// Returns a failed completion to Pending. Any attached intent is aborted
    /// and all of its reserved storage keys are durably queued for GC.
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

    async fn expire_multipart_uploads(
        &self,
        now: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3MultipartExpiredUpload>, RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_canonical_part_and_multipart_etags() {
        assert!(is_lowercase_md5_hex("d41d8cd98f00b204e9800998ecf8427e"));
        assert!(!is_lowercase_md5_hex("D41D8CD98F00B204E9800998ECF8427E"));
        assert!(is_multipart_etag("0123456789abcdef0123456789abcdef-10000"));
        assert!(!is_multipart_etag("0123456789abcdef0123456789abcdef-0"));
        assert!(!is_multipart_etag("0123456789abcdef0123456789abcdef-10001"));
    }
}
