use async_trait::async_trait;
use mediahub_core::{ApplicationId, OffsetDateTime, S3IdentityPolicy, S3PolicyError};
use sha2::{Digest, Sha256};

use crate::{RepositoryError, S3AccountId};

/// Canonical, validated identity-policy document ready for persistence.
///
/// Construction always passes through Core's strict parser. The exact stable
/// JSON emitted by Core is both persisted and hashed, so adapters cannot save
/// an unvalidated policy or hash a representation different from the stored
/// authorization document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3IdentityPolicyDocument {
    stable_json: String,
    sha256: String,
}

impl S3IdentityPolicyDocument {
    pub fn parse(document: &[u8]) -> Result<Self, S3PolicyError> {
        let policy = S3IdentityPolicy::parse(document)?;
        Ok(Self::from_policy(&policy))
    }

    #[must_use]
    pub fn from_policy(policy: &S3IdentityPolicy) -> Self {
        let stable_json = policy.to_stable_json();
        let sha256 = hex::encode(Sha256::digest(stable_json.as_bytes()));
        Self {
            stable_json,
            sha256,
        }
    }

    #[must_use]
    pub fn stable_json(&self) -> &str {
        &self.stable_json
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }
}

/// Tenant identity of the access key to which an identity policy is attached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3AccessKeyIdentity {
    pub application_id: ApplicationId,
    /// Stable account ID of the application that owns this access key.
    /// This is the signed principal's account, never the target bucket owner.
    pub account_id: S3AccountId,
    pub access_key_id: String,
}

/// Durable policy state for one access key.
///
/// Revision zero means no policy mutation has occurred. A positive revision
/// with no document records a deleted policy and preserves monotonic history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3IdentityPolicySnapshot {
    pub identity: S3AccessKeyIdentity,
    pub policy: Option<S3IdentityPolicyDocument>,
    pub revision: u64,
    pub updated_at: Option<OffsetDateTime>,
}

impl S3IdentityPolicySnapshot {
    pub fn new(
        identity: S3AccessKeyIdentity,
        policy: Option<S3IdentityPolicyDocument>,
        revision: u64,
        updated_at: Option<OffsetDateTime>,
    ) -> Result<Self, RepositoryError> {
        if (revision == 0) != updated_at.is_none() || (revision == 0 && policy.is_some()) {
            return Err(RepositoryError::Invariant(
                "S3 identity policy document, revision, and update timestamp disagree".into(),
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

/// Tenant-fenced request to install or replace an access-key identity policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PutS3IdentityPolicy {
    pub application_id: ApplicationId,
    pub access_key_id: String,
    pub policy: S3IdentityPolicyDocument,
    pub updated_at: OffsetDateTime,
}

/// Tenant-fenced request to remove an access-key identity policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteS3IdentityPolicy {
    pub application_id: ApplicationId,
    pub access_key_id: String,
    pub updated_at: OffsetDateTime,
}

/// Persistence port for policies attached to SigV4 access keys.
///
/// The global read accepts the already-parsed access-key ID from SigV4. It
/// does not authenticate the key: callers must first reject an unknown,
/// revoked, expired, or incorrectly signed key and must never downgrade a bad
/// signature to anonymous access. Mutations return `None` when the composite
/// `application_id + access_key_id` ownership fence does not match. Delete is
/// idempotent: a key whose policy is already absent is returned unchanged.
/// The returned snapshot carries the signing key application's stable S3
/// account ID; authorization must not substitute the addressed bucket's owner
/// account. A missing policy means implicit deny. Implementations and callers
/// must not synthesize an identity policy from legacy `permissions` or use
/// those permissions as an authorization fallback.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3IdentityPolicyRepository: Send + Sync {
    async fn get_s3_identity_policy(
        &self,
        access_key_id: &str,
    ) -> Result<Option<S3IdentityPolicySnapshot>, RepositoryError>;

    async fn put_s3_identity_policy(
        &self,
        command: &PutS3IdentityPolicy,
    ) -> Result<Option<S3IdentityPolicySnapshot>, RepositoryError>;

    async fn delete_s3_identity_policy(
        &self,
        command: &DeleteS3IdentityPolicy,
    ) -> Result<Option<S3IdentityPolicySnapshot>, RepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use mediahub_core::S3PolicyErrorCode;

    #[test]
    fn document_is_strictly_parsed_and_stably_hashed() {
        let first = S3IdentityPolicyDocument::parse(
            br#"{"Statement":{"Resource":"arn:aws:s3:::photos/*","Effect":"Allow","Action":["s3:PutObject","s3:GetObject"]},"Version":"2012-10-17"}"#,
        )
        .expect("valid identity policy");
        let second = S3IdentityPolicyDocument::parse(
            br#"{"Version":"2012-10-17","Statement":[{"Effect":"Allow","Action":["s3:GetObject","s3:PutObject"],"Resource":"arn:aws:s3:::photos/*"}]}"#,
        )
        .expect("equivalent identity policy");
        assert_eq!(first, second);
        assert_eq!(first.sha256().len(), 64);
        assert_eq!(
            S3IdentityPolicyDocument::parse(
                br#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"*"}}"#,
            )
            .expect_err("identity policies must reject principals")
            .code(),
            S3PolicyErrorCode::UnsupportedField
        );
    }
}
