//! Unified S3 identity- and bucket-policy authorization decisions.
//!
//! Authentication is deliberately outside this service. A runtime must use
//! [`S3AuthorizationPrincipal::Anonymous`] only when the request carried no
//! authentication material. A failed or incomplete signature must be rejected
//! by the runtime and must never be converted to an anonymous principal.

use std::net::IpAddr;

use mediahub_core::{
    MAX_S3_POLICY_RUNTIME_KEY_BYTES, MAX_S3_POLICY_STRING_BYTES, S3BucketPolicy, S3PolicyAction,
    S3PolicyDecision, S3PolicyQuery, S3PolicyRequest, S3PolicyResourceScope,
};
use thiserror::Error;

use crate::{
    RepositoryError, S3AccountId, S3BucketIdentity, S3BucketPolicyRepository,
    S3BucketPolicySnapshot,
};

/// A signature-verified S3 caller and its already evaluated identity-policy
/// decision. Legacy access-key permissions are intentionally not represented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3SignedPrincipal {
    account_id: S3AccountId,
    principal_arn: String,
    userid: String,
    identity_decision: S3PolicyDecision,
}

impl S3SignedPrincipal {
    /// Builds a signed principal only after signature authentication succeeds.
    ///
    /// The ARN must be a supported IAM/STS principal in `account_id`. Keeping
    /// these fields private prevents an unchecked owner/account mismatch from
    /// reaching policy evaluation.
    pub fn new(
        account_id: S3AccountId,
        principal_arn: impl Into<String>,
        userid: impl Into<String>,
        identity_decision: S3PolicyDecision,
    ) -> Result<Self, S3PrincipalValidationError> {
        let principal_arn = principal_arn.into();
        let userid = userid.into();
        validate_principal_arn(&principal_arn, account_id.as_str())?;
        if userid.is_empty()
            || userid.len() > MAX_S3_POLICY_STRING_BYTES
            || userid.chars().any(char::is_control)
        {
            return Err(S3PrincipalValidationError::InvalidUserId);
        }
        Ok(Self {
            account_id,
            principal_arn,
            userid,
            identity_decision,
        })
    }

    #[must_use]
    pub const fn account_id(&self) -> &S3AccountId {
        &self.account_id
    }

    #[must_use]
    pub fn principal_arn(&self) -> &str {
        &self.principal_arn
    }

    #[must_use]
    pub fn userid(&self) -> &str {
        &self.userid
    }

    #[must_use]
    pub const fn identity_decision(&self) -> S3PolicyDecision {
        self.identity_decision
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum S3PrincipalValidationError {
    #[error("the S3 principal ARN is invalid")]
    InvalidPrincipalArn,

    #[error("the S3 principal ARN belongs to a different account")]
    PrincipalAccountMismatch,

    #[error("the S3 principal user ID is invalid")]
    InvalidUserId,
}

/// Explicit authentication state supplied by the S3 protocol runtime.
///
/// `Anonymous` means no authentication material was present. It is not an
/// authentication-failure fallback.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3AuthorizationPrincipal {
    Anonymous,
    Signed(S3SignedPrincipal),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3AuthorizationRequest {
    pub action: S3PolicyAction,
    pub bucket_name: String,
    pub object_key: Option<String>,
    pub version_id: Option<String>,
    pub principal: S3AuthorizationPrincipal,
    pub secure_transport: bool,
    pub source_ip: Option<IpAddr>,
    pub prefix: Option<String>,
    pub delimiter: Option<String>,
    pub max_keys: Option<u64>,
}

/// Authorization outcome plus the globally resolved tenant identity.
///
/// Denied outcomes retain the bucket identity for owner-aware HTTP mappings,
/// while callers must use tenant data from `Allowed` before executing the S3
/// operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3AuthorizationOutcome {
    BucketNotFound,
    Allowed { bucket: S3BucketIdentity },
    ExplicitDeny { bucket: S3BucketIdentity },
    ImplicitDeny { bucket: S3BucketIdentity },
}

impl S3AuthorizationOutcome {
    #[must_use]
    pub const fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    #[must_use]
    pub const fn bucket(&self) -> Option<&S3BucketIdentity> {
        match self {
            Self::BucketNotFound => None,
            Self::Allowed { bucket }
            | Self::ExplicitDeny { bucket }
            | Self::ImplicitDeny { bucket } => Some(bucket),
        }
    }
}

pub struct S3AuthorizationService<R> {
    repository: R,
}

impl<R> S3AuthorizationService<R>
where
    R: S3BucketPolicyRepository,
{
    #[must_use]
    pub const fn new(repository: R) -> Self {
        Self { repository }
    }

    /// Resolves a globally unique bucket and combines identity and resource
    /// policy decisions using AWS same-account/cross-account semantics.
    pub async fn authorize(
        &self,
        request: &S3AuthorizationRequest,
    ) -> Result<S3AuthorizationOutcome, RepositoryError> {
        let Some(snapshot) = self
            .repository
            .get_s3_bucket_policy(&request.bucket_name)
            .await?
        else {
            return Ok(S3AuthorizationOutcome::BucketNotFound);
        };

        if snapshot.identity.bucket_name != request.bucket_name {
            return Err(RepositoryError::Invariant(
                "global S3 bucket lookup returned a different bucket".into(),
            ));
        }

        let identity_decision = match &request.principal {
            S3AuthorizationPrincipal::Anonymous => S3PolicyDecision::ImplicitDeny,
            S3AuthorizationPrincipal::Signed(principal) => principal.identity_decision(),
        };
        if identity_decision == S3PolicyDecision::ExplicitDeny {
            return Ok(explicit_deny(snapshot.identity));
        }

        if !request_is_well_formed(request) {
            return Ok(implicit_deny(snapshot.identity));
        }

        let core_request = core_policy_request(request);
        let bucket_decision = match evaluate_bucket_policy(&snapshot, &core_request) {
            BucketPolicyEvaluation::Decision(decision) => decision,
            BucketPolicyEvaluation::InvalidPersistedPolicy => {
                return Ok(implicit_deny(snapshot.identity));
            }
        };

        if bucket_decision == S3PolicyDecision::ExplicitDeny {
            return Ok(explicit_deny(snapshot.identity));
        }

        let allowed = match &request.principal {
            S3AuthorizationPrincipal::Anonymous => bucket_decision == S3PolicyDecision::Allow,
            S3AuthorizationPrincipal::Signed(principal)
                if principal.account_id() == &snapshot.identity.owner_account_id =>
            {
                identity_decision == S3PolicyDecision::Allow
                    || bucket_decision == S3PolicyDecision::Allow
            }
            S3AuthorizationPrincipal::Signed(_) => {
                identity_decision == S3PolicyDecision::Allow
                    && bucket_decision == S3PolicyDecision::Allow
            }
        };

        if allowed {
            Ok(S3AuthorizationOutcome::Allowed {
                bucket: snapshot.identity,
            })
        } else {
            Ok(implicit_deny(snapshot.identity))
        }
    }
}

enum BucketPolicyEvaluation {
    Decision(S3PolicyDecision),
    InvalidPersistedPolicy,
}

fn evaluate_bucket_policy(
    snapshot: &S3BucketPolicySnapshot,
    request: &S3PolicyRequest<'_>,
) -> BucketPolicyEvaluation {
    let Some(document) = &snapshot.policy else {
        return BucketPolicyEvaluation::Decision(S3PolicyDecision::ImplicitDeny);
    };
    let Ok(bytes) = serde_json::to_vec(document.document()) else {
        return BucketPolicyEvaluation::InvalidPersistedPolicy;
    };
    let Ok(policy) = S3BucketPolicy::parse(&bytes, &snapshot.identity.bucket_name) else {
        return BucketPolicyEvaluation::InvalidPersistedPolicy;
    };
    BucketPolicyEvaluation::Decision(policy.evaluate(request))
}

fn core_policy_request(request: &S3AuthorizationRequest) -> S3PolicyRequest<'_> {
    let (principal_arn, principal_account, userid) = match &request.principal {
        S3AuthorizationPrincipal::Anonymous => (None, None, None),
        S3AuthorizationPrincipal::Signed(principal) => (
            Some(principal.principal_arn()),
            Some(principal.account_id().as_str()),
            Some(principal.userid()),
        ),
    };
    S3PolicyRequest {
        action: request.action,
        bucket: &request.bucket_name,
        key: request.object_key.as_deref(),
        version_id: request.version_id.as_deref(),
        principal_arn,
        principal_account,
        userid,
        secure_transport: request.secure_transport,
        source_ip: request.source_ip,
        query: S3PolicyQuery {
            prefix: request.prefix.as_deref(),
            delimiter: request.delimiter.as_deref(),
            max_keys: request.max_keys,
        },
    }
}

fn request_is_well_formed(request: &S3AuthorizationRequest) -> bool {
    let valid_resource = match (request.action.scope(), request.object_key.as_deref()) {
        (S3PolicyResourceScope::Bucket, None) => true,
        (S3PolicyResourceScope::Object, Some(key)) => {
            !key.is_empty() && key.len() <= MAX_S3_POLICY_RUNTIME_KEY_BYTES
        }
        _ => false,
    };
    valid_bucket_name(&request.bucket_name)
        && valid_resource
        && bounded(&request.version_id, MAX_S3_POLICY_STRING_BYTES)
        && bounded(&request.prefix, MAX_S3_POLICY_RUNTIME_KEY_BYTES)
        && bounded(&request.delimiter, MAX_S3_POLICY_STRING_BYTES)
}

fn valid_bucket_name(value: &str) -> bool {
    (3..=63).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && !value.contains("..")
        && !value.contains(".-")
        && !value.contains("-.")
        && value.parse::<std::net::Ipv4Addr>().is_err()
}

fn bounded(value: &Option<String>, max_len: usize) -> bool {
    value.as_ref().is_none_or(|value| value.len() <= max_len)
}

fn explicit_deny(bucket: S3BucketIdentity) -> S3AuthorizationOutcome {
    S3AuthorizationOutcome::ExplicitDeny { bucket }
}

fn implicit_deny(bucket: S3BucketIdentity) -> S3AuthorizationOutcome {
    S3AuthorizationOutcome::ImplicitDeny { bucket }
}

fn validate_principal_arn(
    principal_arn: &str,
    expected_account_id: &str,
) -> Result<(), S3PrincipalValidationError> {
    if principal_arn.len() > MAX_S3_POLICY_STRING_BYTES {
        return Err(S3PrincipalValidationError::InvalidPrincipalArn);
    }
    let parts = principal_arn.splitn(6, ':').collect::<Vec<_>>();
    if parts.len() != 6
        || parts[0] != "arn"
        || parts[1] != "aws"
        || !matches!(parts[2], "iam" | "sts")
        || !parts[3].is_empty()
        || parts[4].len() != 12
        || !parts[4].bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(S3PrincipalValidationError::InvalidPrincipalArn);
    }
    if parts[4] != expected_account_id {
        return Err(S3PrincipalValidationError::PrincipalAccountMismatch);
    }
    let valid_resource = match parts[2] {
        "iam" => {
            parts[5] == "root"
                || valid_principal_path(parts[5], "user/")
                || valid_principal_path(parts[5], "role/")
        }
        "sts" => valid_principal_path(parts[5], "assumed-role/"),
        _ => false,
    };
    valid_resource
        .then_some(())
        .ok_or(S3PrincipalValidationError::InvalidPrincipalArn)
}

fn valid_principal_path(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|suffix| {
        !suffix.is_empty()
            && !suffix.starts_with('/')
            && !suffix.ends_with('/')
            && suffix.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, '/' | '+' | '=' | ',' | '.' | '@' | '_' | '-')
            })
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use futures::executor::block_on;
    use mediahub_core::{ApplicationId, BucketId, OffsetDateTime};
    use serde_json::{Value, json};

    use super::*;
    use crate::{S3BucketPolicyDocument, S3BucketPolicyRepository};

    const OWNER_ACCOUNT: &str = "123456789012";
    const FOREIGN_ACCOUNT: &str = "210987654321";
    const BUCKET: &str = "photos-example";
    const OWNER_ARN: &str = "arn:aws:iam::123456789012:user/alice";
    const FOREIGN_ARN: &str = "arn:aws:iam::210987654321:role/readers/bob";

    #[derive(Clone, Default)]
    struct FakePolicyRepository {
        snapshots: Arc<Mutex<HashMap<String, S3BucketPolicySnapshot>>>,
        lookups: Arc<Mutex<Vec<String>>>,
    }

    impl FakePolicyRepository {
        fn with_snapshot(snapshot: S3BucketPolicySnapshot) -> Self {
            Self {
                snapshots: Arc::new(Mutex::new(HashMap::from([(
                    snapshot.identity.bucket_name.clone(),
                    snapshot,
                )]))),
                lookups: Arc::default(),
            }
        }

        fn insert(&self, snapshot: S3BucketPolicySnapshot) {
            self.snapshots
                .lock()
                .expect("fake policy snapshots")
                .insert(snapshot.identity.bucket_name.clone(), snapshot);
        }
    }

    #[async_trait]
    impl S3BucketPolicyRepository for FakePolicyRepository {
        async fn resolve_s3_bucket_identity(
            &self,
            bucket_name: &str,
        ) -> Result<Option<S3BucketIdentity>, RepositoryError> {
            Ok(self
                .snapshots
                .lock()
                .expect("fake policy snapshots")
                .get(bucket_name)
                .map(|snapshot| snapshot.identity.clone()))
        }

        async fn get_s3_bucket_policy(
            &self,
            bucket_name: &str,
        ) -> Result<Option<S3BucketPolicySnapshot>, RepositoryError> {
            self.lookups
                .lock()
                .expect("fake policy lookups")
                .push(bucket_name.to_owned());
            Ok(self
                .snapshots
                .lock()
                .expect("fake policy snapshots")
                .get(bucket_name)
                .cloned())
        }

        async fn put_s3_bucket_policy(
            &self,
            _application_id: ApplicationId,
            _bucket_name: &str,
            _policy: S3BucketPolicyDocument,
            _updated_at: OffsetDateTime,
        ) -> Result<Option<S3BucketPolicySnapshot>, RepositoryError> {
            Err(RepositoryError::Unavailable(
                "not implemented by fake".into(),
            ))
        }

        async fn delete_s3_bucket_policy(
            &self,
            _application_id: ApplicationId,
            _bucket_name: &str,
            _updated_at: OffsetDateTime,
        ) -> Result<Option<S3BucketPolicySnapshot>, RepositoryError> {
            Err(RepositoryError::Unavailable(
                "not implemented by fake".into(),
            ))
        }
    }

    #[test]
    fn signed_principal_is_strictly_validated() {
        let owner = account(OWNER_ACCOUNT);
        assert!(
            S3SignedPrincipal::new(
                owner.clone(),
                OWNER_ARN,
                "AIDAALICE",
                S3PolicyDecision::Allow,
            )
            .is_ok()
        );
        assert!(
            S3SignedPrincipal::new(
                owner.clone(),
                "arn:aws:sts::123456789012:assumed-role/media/session-1",
                "AROAMEDIA:session-1",
                S3PolicyDecision::Allow,
            )
            .is_ok()
        );
        assert_eq!(
            S3SignedPrincipal::new(
                owner.clone(),
                FOREIGN_ARN,
                "AIDABOB",
                S3PolicyDecision::Allow,
            ),
            Err(S3PrincipalValidationError::PrincipalAccountMismatch)
        );
        assert_eq!(
            S3SignedPrincipal::new(
                owner.clone(),
                "arn:aws:s3::123456789012:user/alice",
                "AIDAALICE",
                S3PolicyDecision::Allow,
            ),
            Err(S3PrincipalValidationError::InvalidPrincipalArn)
        );
        assert_eq!(
            S3SignedPrincipal::new(owner, OWNER_ARN, "bad\nuserid", S3PolicyDecision::Allow,),
            Err(S3PrincipalValidationError::InvalidUserId)
        );
    }

    #[test]
    fn aws_identity_and_bucket_policy_combinations_are_enforced() {
        struct Case {
            name: &'static str,
            principal: S3AuthorizationPrincipal,
            policy_effect: Option<&'static str>,
            expected: ExpectedOutcome,
        }

        let cases = [
            Case {
                name: "anonymous bucket allow",
                principal: S3AuthorizationPrincipal::Anonymous,
                policy_effect: Some("Allow"),
                expected: ExpectedOutcome::Allowed,
            },
            Case {
                name: "anonymous missing policy",
                principal: S3AuthorizationPrincipal::Anonymous,
                policy_effect: None,
                expected: ExpectedOutcome::ImplicitDeny,
            },
            Case {
                name: "anonymous bucket deny",
                principal: S3AuthorizationPrincipal::Anonymous,
                policy_effect: Some("Deny"),
                expected: ExpectedOutcome::ExplicitDeny,
            },
            Case {
                name: "same account identity allow",
                principal: signed(OWNER_ACCOUNT, OWNER_ARN, S3PolicyDecision::Allow),
                policy_effect: None,
                expected: ExpectedOutcome::Allowed,
            },
            Case {
                name: "same account bucket allow",
                principal: signed(OWNER_ACCOUNT, OWNER_ARN, S3PolicyDecision::ImplicitDeny),
                policy_effect: Some("Allow"),
                expected: ExpectedOutcome::Allowed,
            },
            Case {
                name: "same account no allow",
                principal: signed(OWNER_ACCOUNT, OWNER_ARN, S3PolicyDecision::ImplicitDeny),
                policy_effect: None,
                expected: ExpectedOutcome::ImplicitDeny,
            },
            Case {
                name: "identity explicit deny wins",
                principal: signed(OWNER_ACCOUNT, OWNER_ARN, S3PolicyDecision::ExplicitDeny),
                policy_effect: Some("Allow"),
                expected: ExpectedOutcome::ExplicitDeny,
            },
            Case {
                name: "bucket explicit deny wins",
                principal: signed(OWNER_ACCOUNT, OWNER_ARN, S3PolicyDecision::Allow),
                policy_effect: Some("Deny"),
                expected: ExpectedOutcome::ExplicitDeny,
            },
            Case {
                name: "cross account has both allows",
                principal: signed(FOREIGN_ACCOUNT, FOREIGN_ARN, S3PolicyDecision::Allow),
                policy_effect: Some("Allow"),
                expected: ExpectedOutcome::Allowed,
            },
            Case {
                name: "cross account lacks bucket allow",
                principal: signed(FOREIGN_ACCOUNT, FOREIGN_ARN, S3PolicyDecision::Allow),
                policy_effect: None,
                expected: ExpectedOutcome::ImplicitDeny,
            },
            Case {
                name: "cross account lacks identity allow",
                principal: signed(FOREIGN_ACCOUNT, FOREIGN_ARN, S3PolicyDecision::ImplicitDeny),
                policy_effect: Some("Allow"),
                expected: ExpectedOutcome::ImplicitDeny,
            },
        ];

        for case in cases {
            let repository = FakePolicyRepository::with_snapshot(snapshot(
                BUCKET,
                OWNER_ACCOUNT,
                case.policy_effect.map(|effect| policy(BUCKET, effect)),
            ));
            let outcome = block_on(
                S3AuthorizationService::new(repository)
                    .authorize(&object_request(BUCKET, case.principal)),
            )
            .expect(case.name);
            assert_outcome(case.expected, &outcome, case.name);
        }
    }

    #[test]
    fn missing_bucket_is_distinct() {
        let outcome = block_on(
            S3AuthorizationService::new(FakePolicyRepository::default())
                .authorize(&object_request(BUCKET, S3AuthorizationPrincipal::Anonymous)),
        )
        .expect("missing bucket decision");
        assert_eq!(outcome, S3AuthorizationOutcome::BucketNotFound);
    }

    #[test]
    fn all_supported_request_conditions_reach_the_core_evaluator() {
        let condition_policy = policy_value(json!({
            "Version": "2012-10-17",
            "Statement": {
                "Effect": "Allow",
                "Principal": "*",
                "Action": "s3:ListBucket",
                "Resource": format!("arn:aws:s3:::{BUCKET}"),
                "Condition": {
                    "Bool": {"aws:SecureTransport": true},
                    "IpAddress": {"aws:SourceIp": "203.0.113.0/24"},
                    "StringEquals": {
                        "s3:prefix": "images/",
                        "s3:delimiter": "/",
                        "s3:VersionId": "v7"
                    },
                    "NumericLessThanEquals": {"s3:max-keys": 100}
                }
            }
        }));
        let service = S3AuthorizationService::new(FakePolicyRepository::with_snapshot(snapshot(
            BUCKET,
            OWNER_ACCOUNT,
            Some(condition_policy),
        )));
        let matching = S3AuthorizationRequest {
            action: S3PolicyAction::ListBucket,
            bucket_name: BUCKET.to_owned(),
            object_key: None,
            version_id: Some("v7".to_owned()),
            principal: S3AuthorizationPrincipal::Anonymous,
            secure_transport: true,
            source_ip: Some("203.0.113.42".parse().expect("test IP")),
            prefix: Some("images/".to_owned()),
            delimiter: Some("/".to_owned()),
            max_keys: Some(100),
        };
        assert!(
            block_on(service.authorize(&matching))
                .expect("matching conditions")
                .is_allowed()
        );

        let mismatches = [
            S3AuthorizationRequest {
                secure_transport: false,
                ..matching.clone()
            },
            S3AuthorizationRequest {
                source_ip: Some("198.51.100.1".parse().expect("test IP")),
                ..matching.clone()
            },
            S3AuthorizationRequest {
                prefix: Some("private/".to_owned()),
                ..matching.clone()
            },
            S3AuthorizationRequest {
                delimiter: Some(":".to_owned()),
                ..matching.clone()
            },
            S3AuthorizationRequest {
                max_keys: Some(101),
                ..matching.clone()
            },
            S3AuthorizationRequest {
                version_id: Some("v8".to_owned()),
                ..matching
            },
        ];
        for mismatch in mismatches {
            assert!(matches!(
                block_on(service.authorize(&mismatch)).expect("condition mismatch"),
                S3AuthorizationOutcome::ImplicitDeny { .. }
            ));
        }
    }

    #[test]
    fn invalid_persisted_policy_fails_closed_even_with_identity_allow() {
        let invalid = policy_value(json!({"not": "an S3 policy"}));
        let service = S3AuthorizationService::new(FakePolicyRepository::with_snapshot(snapshot(
            BUCKET,
            OWNER_ACCOUNT,
            Some(invalid),
        )));
        let outcome = block_on(service.authorize(&object_request(
            BUCKET,
            signed(OWNER_ACCOUNT, OWNER_ARN, S3PolicyDecision::Allow),
        )))
        .expect("fail-closed decision");
        assert!(matches!(
            outcome,
            S3AuthorizationOutcome::ImplicitDeny { .. }
        ));
    }

    #[test]
    fn malformed_runtime_context_fails_closed_before_same_account_union() {
        let principal = signed(OWNER_ACCOUNT, OWNER_ARN, S3PolicyDecision::Allow);
        let service = S3AuthorizationService::new(FakePolicyRepository::with_snapshot(snapshot(
            BUCKET,
            OWNER_ACCOUNT,
            None,
        )));
        let mut missing_object_key = object_request(BUCKET, principal.clone());
        missing_object_key.object_key = None;
        assert!(matches!(
            block_on(service.authorize(&missing_object_key)).expect("malformed object request"),
            S3AuthorizationOutcome::ImplicitDeny { .. }
        ));

        let invalid_bucket = "Bad_Bucket";
        let service = S3AuthorizationService::new(FakePolicyRepository::with_snapshot(snapshot(
            invalid_bucket,
            OWNER_ACCOUNT,
            None,
        )));
        assert!(matches!(
            block_on(service.authorize(&object_request(invalid_bucket, principal)))
                .expect("malformed bucket request"),
            S3AuthorizationOutcome::ImplicitDeny { .. }
        ));
    }

    #[test]
    fn global_lookup_preserves_tenant_isolation() {
        let first = snapshot(
            "first-bucket",
            OWNER_ACCOUNT,
            Some(policy("first-bucket", "Allow")),
        );
        let second = snapshot(
            "second-bucket",
            FOREIGN_ACCOUNT,
            Some(policy("second-bucket", "Allow")),
        );
        let second_application = second.identity.application_id;
        let repository = FakePolicyRepository::with_snapshot(first);
        repository.insert(second);
        let outcome = block_on(S3AuthorizationService::new(repository.clone()).authorize(
            &object_request(
                "second-bucket",
                signed(OWNER_ACCOUNT, OWNER_ARN, S3PolicyDecision::Allow),
            ),
        ))
        .expect("cross-tenant resource-policy decision");
        let S3AuthorizationOutcome::Allowed { bucket } = outcome else {
            panic!("cross-account request should have both allows");
        };
        assert_eq!(bucket.application_id, second_application);
        assert_eq!(bucket.owner_account_id.as_str(), FOREIGN_ACCOUNT);
        assert_eq!(
            *repository.lookups.lock().expect("fake policy lookups"),
            vec!["second-bucket"]
        );
    }

    #[derive(Clone, Copy)]
    enum ExpectedOutcome {
        Allowed,
        ExplicitDeny,
        ImplicitDeny,
    }

    fn assert_outcome(expected: ExpectedOutcome, actual: &S3AuthorizationOutcome, case_name: &str) {
        let matches = match expected {
            ExpectedOutcome::Allowed => matches!(actual, S3AuthorizationOutcome::Allowed { .. }),
            ExpectedOutcome::ExplicitDeny => {
                matches!(actual, S3AuthorizationOutcome::ExplicitDeny { .. })
            }
            ExpectedOutcome::ImplicitDeny => {
                matches!(actual, S3AuthorizationOutcome::ImplicitDeny { .. })
            }
        };
        assert!(matches, "unexpected outcome for {case_name}: {actual:?}");
    }

    fn object_request(
        bucket_name: &str,
        principal: S3AuthorizationPrincipal,
    ) -> S3AuthorizationRequest {
        S3AuthorizationRequest {
            action: S3PolicyAction::GetObject,
            bucket_name: bucket_name.to_owned(),
            object_key: Some("image.jpg".to_owned()),
            version_id: None,
            principal,
            secure_transport: true,
            source_ip: None,
            prefix: None,
            delimiter: None,
            max_keys: None,
        }
    }

    fn signed(
        account_id: &str,
        principal_arn: &str,
        decision: S3PolicyDecision,
    ) -> S3AuthorizationPrincipal {
        S3AuthorizationPrincipal::Signed(
            S3SignedPrincipal::new(account(account_id), principal_arn, "AIDATEST", decision)
                .expect("valid signed principal"),
        )
    }

    fn snapshot(
        bucket_name: &str,
        owner_account_id: &str,
        policy: Option<S3BucketPolicyDocument>,
    ) -> S3BucketPolicySnapshot {
        S3BucketPolicySnapshot::new(
            S3BucketIdentity {
                bucket_id: BucketId::new(),
                application_id: ApplicationId::new(),
                bucket_name: bucket_name.to_owned(),
                owner_account_id: account(owner_account_id),
            },
            policy,
            1,
            Some(OffsetDateTime::UNIX_EPOCH),
        )
        .expect("valid policy snapshot")
    }

    fn policy(bucket_name: &str, effect: &str) -> S3BucketPolicyDocument {
        policy_value(json!({
            "Version": "2012-10-17",
            "Statement": {
                "Effect": effect,
                "Principal": "*",
                "Action": "s3:GetObject",
                "Resource": format!("arn:aws:s3:::{bucket_name}/*")
            }
        }))
    }

    fn policy_value(value: Value) -> S3BucketPolicyDocument {
        S3BucketPolicyDocument::new(value).expect("policy document")
    }

    fn account(value: &str) -> S3AccountId {
        S3AccountId::new(value).expect("valid account ID")
    }
}
