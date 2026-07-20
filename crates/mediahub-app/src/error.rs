use std::fmt;

use mediahub_core::DomainError;
use thiserror::Error;

use crate::{ObjectStoreError, RepositoryError};

pub(crate) struct Redacted<T>(pub T);

impl<T> fmt::Debug for Redacted<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted>")
    }
}

/// Stable application-level failures. Runtime entries map these errors to their
/// transport-specific response formats without exposing adapter internals.
#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("bucket was not found")]
    BucketNotFound,

    #[error("bucket does not belong to the application")]
    BucketDoesNotBelongToApplication,

    #[error("an object already exists for this bucket and object key")]
    ObjectAlreadyExists,

    #[error("application quota is exhausted")]
    QuotaExceeded,

    #[error("upload session was not found")]
    UploadSessionNotFound,

    #[error("upload session does not belong to the application")]
    UploadSessionDoesNotBelongToApplication,

    #[error("upload session is expired")]
    UploadSessionExpired,

    #[error("upload session was cancelled")]
    UploadSessionCancelled,

    #[error("upload session has already completed")]
    UploadSessionAlreadyCompleted,

    #[error("uploaded object does not match the upload session contract")]
    UploadSessionVerificationFailed,

    #[error(transparent)]
    Domain(#[from] DomainError),

    #[error(transparent)]
    Repository(#[from] RepositoryError),

    #[error(transparent)]
    ObjectStore(#[from] ObjectStoreError),
}
