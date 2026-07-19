use thiserror::Error;

use crate::{MediaState, UploadSessionState};

pub type DomainResult<T> = Result<T, DomainError>;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum DomainError {
    #[error("bucket name must be between 1 and 255 bytes")]
    InvalidBucketName,

    #[error("bucket lifecycle rules are invalid")]
    InvalidLifecycleRule,

    #[error("bucket TTL and maximum object size must be positive when configured")]
    InvalidBucketPolicy,

    #[error("object key must be between 1 and 1024 bytes and contain no NUL byte")]
    InvalidObjectKey,

    #[error("display name must be between 1 and 255 bytes")]
    InvalidDisplayName,

    #[error("{field} must not be empty or exceed {max_bytes} bytes")]
    InvalidTextField {
        field: &'static str,
        max_bytes: usize,
    },

    #[error("invalid MIME type: {value}")]
    InvalidMimeType { value: String },

    #[error("invalid SHA-256 digest")]
    InvalidSha256,

    #[error("metadata version must be greater than zero")]
    InvalidMetadataVersion,

    #[error("media cannot transition from {from:?} to {to:?}")]
    InvalidMediaStateTransition { from: MediaState, to: MediaState },

    #[error("media in {state:?} state cannot be read")]
    MediaNotReadable { state: MediaState },

    #[error("media revision conflict: expected {expected}, actual {actual}")]
    RevisionConflict { expected: u64, actual: u64 },

    #[error("upload session expiry must be in the future")]
    InvalidUploadSessionExpiry,

    #[error("upload session has not reached its expiry time")]
    UploadSessionNotExpired,

    #[error("upload session is expired")]
    UploadSessionExpired,

    #[error("upload session cannot transition from {from:?} to {to:?}")]
    InvalidUploadSessionStateTransition {
        from: UploadSessionState,
        to: UploadSessionState,
    },

    #[error("metadata submitted by a client must not contain the system namespace")]
    RestrictedMetadataNamespace,

    #[error("unknown metadata namespace: {namespace}")]
    UnknownMetadataNamespace { namespace: String },

    #[error("metadata root and namespaces must be JSON objects")]
    InvalidMetadataShape,

    #[error("serialized metadata exceeds the {max_bytes}-byte limit")]
    MetadataTooLarge { max_bytes: usize },

    #[error("metadata nesting exceeds the maximum depth of {max_depth}")]
    MetadataTooDeep { max_depth: usize },

    #[error("metadata contains more than {max_keys} object keys")]
    MetadataTooManyKeys { max_keys: usize },

    #[error("metadata string exceeds the {max_bytes}-byte limit")]
    MetadataStringTooLong { max_bytes: usize },

    #[error("object size {actual} exceeds bucket limit {maximum}")]
    ObjectTooLarge { actual: u64, maximum: u64 },

    #[error("MIME type {mime} is not allowed by the bucket policy")]
    MimeTypeNotAllowed { mime: String },
}
