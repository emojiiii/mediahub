use async_trait::async_trait;
use mediahub_core::{ApplicationId, BucketId, MAX_S3_POLICY_BYTES, OffsetDateTime};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::RepositoryError;

pub const S3_ACCOUNT_ID_DIGITS: usize = 12;

/// Stable AWS-compatible account identity assigned by the database.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct S3AccountId(String);

impl S3AccountId {
    pub fn new(value: impl Into<String>) -> Result<Self, RepositoryError> {
        let value = value.into();
        if value.len() != S3_ACCOUNT_ID_DIGITS || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(RepositoryError::Invariant(
                "S3 account ID must contain exactly 12 decimal digits".into(),
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Globally resolvable bucket identity needed before resource-policy auth.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3BucketIdentity {
    pub bucket_id: BucketId,
    pub application_id: ApplicationId,
    pub bucket_name: String,
    pub owner_account_id: S3AccountId,
}

/// Canonical JSON policy payload and the digest of the exact persisted form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3BucketPolicyDocument {
    document: Value,
    sha256: String,
}

impl S3BucketPolicyDocument {
    pub fn new(document: Value) -> Result<Self, RepositoryError> {
        if !document.is_object() {
            return Err(RepositoryError::Invariant(
                "S3 bucket policy document must be a JSON object".into(),
            ));
        }
        let canonical = serde_json::to_vec(&document)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        if canonical.len() > MAX_S3_POLICY_BYTES {
            return Err(RepositoryError::Invariant(
                "S3 bucket policy document exceeds the persistence limit".into(),
            ));
        }
        let sha256 = hex::encode(Sha256::digest(&canonical));
        Ok(Self { document, sha256 })
    }

    #[must_use]
    pub const fn document(&self) -> &Value {
        &self.document
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// One global bucket lookup, including an optional installed policy.
///
/// Revision zero means the bucket has never received a policy mutation. A
/// positive revision with no document represents a successfully deleted policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3BucketPolicySnapshot {
    pub identity: S3BucketIdentity,
    pub policy: Option<S3BucketPolicyDocument>,
    pub revision: u64,
    pub updated_at: Option<OffsetDateTime>,
}

impl S3BucketPolicySnapshot {
    pub fn new(
        identity: S3BucketIdentity,
        policy: Option<S3BucketPolicyDocument>,
        revision: u64,
        updated_at: Option<OffsetDateTime>,
    ) -> Result<Self, RepositoryError> {
        if (revision == 0) != updated_at.is_none() || (revision == 0 && policy.is_some()) {
            return Err(RepositoryError::Invariant(
                "S3 bucket policy document, revision, and update timestamp disagree".into(),
            ));
        }
        Ok(Self {
            identity,
            policy,
            revision,
            updated_at,
        })
    }
}

/// Persistence port for S3 Bucket Policy identity and state.
///
/// Global reads are intentionally suitable for anonymous request resolution.
/// Every mutation must fence both tenant and globally unique bucket name, lock
/// the bucket before any other mutable identity, and increment revision in the
/// same database statement that changes the document.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3BucketPolicyRepository: Send + Sync {
    async fn resolve_s3_bucket_identity(
        &self,
        bucket_name: &str,
    ) -> Result<Option<S3BucketIdentity>, RepositoryError>;

    async fn get_s3_bucket_policy(
        &self,
        bucket_name: &str,
    ) -> Result<Option<S3BucketPolicySnapshot>, RepositoryError>;

    async fn put_s3_bucket_policy(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        policy: S3BucketPolicyDocument,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketPolicySnapshot>, RepositoryError>;

    async fn delete_s3_bucket_policy(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketPolicySnapshot>, RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn account_identity_requires_exactly_twelve_digits() {
        assert!(S3AccountId::new("123456789012").is_ok());
        assert!(S3AccountId::new("12345678901").is_err());
        assert!(S3AccountId::new("12345678901x").is_err());
    }

    #[test]
    fn policy_document_digest_is_stable_for_canonical_json() {
        let first = S3BucketPolicyDocument::new(json!({"b": 2, "a": 1})).expect("policy document");
        let second = S3BucketPolicyDocument::new(json!({"a": 1, "b": 2})).expect("policy document");
        assert_eq!(first.sha256(), second.sha256());
        assert!(S3BucketPolicyDocument::new(json!([])).is_err());
    }
}
