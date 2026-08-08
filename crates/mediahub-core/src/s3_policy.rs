//! Strict, runtime-independent S3 bucket/identity-policy parsing and evaluation.
//!
//! This module intentionally implements a conservative subset of the AWS
//! `2012-10-17` policy language. Unsupported actions, principals, condition
//! operators, and condition keys are rejected while a policy is installed;
//! identity policies forbid principal clauses entirely. Evaluation itself is
//! total and never turns malformed input into access.

use std::{
    collections::BTreeSet,
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};

pub const S3_POLICY_VERSION: &str = "2012-10-17";
pub const S3_POLICY_LEGACY_VERSION: &str = "2008-10-17";
pub const MAX_S3_POLICY_BYTES: usize = 20 * 1024;
pub const MAX_S3_POLICY_DEPTH: usize = 16;
pub const MAX_S3_POLICY_STATEMENTS: usize = 100;
pub const MAX_S3_POLICY_ACTIONS: usize = 64;
pub const MAX_S3_POLICY_RESOURCES: usize = 64;
pub const MAX_S3_POLICY_PRINCIPALS: usize = 64;
pub const MAX_S3_POLICY_CONDITION_OPERATORS: usize = 16;
pub const MAX_S3_POLICY_CONDITION_KEYS: usize = 32;
pub const MAX_S3_POLICY_CONDITION_VALUES: usize = 64;
pub const MAX_S3_POLICY_STRING_BYTES: usize = 2_048;
pub const MAX_S3_POLICY_SID_BYTES: usize = 128;
pub const MAX_S3_POLICY_RUNTIME_KEY_BYTES: usize = 1_024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3PolicyErrorKind {
    Syntax,
    Validation,
    LimitExceeded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3PolicyErrorCode {
    MalformedJson,
    PolicyTooLarge,
    PolicyTooDeep,
    InvalidDocument,
    UnsupportedVersion,
    UnsupportedField,
    InvalidStatement,
    InvalidPrincipal,
    UnsupportedAction,
    InvalidResource,
    UnsupportedCondition,
    LimitExceeded,
}

/// A deliberately non-sensitive policy error suitable for mapping to an S3
/// protocol error. It never contains source JSON or rejected values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct S3PolicyError {
    kind: S3PolicyErrorKind,
    code: S3PolicyErrorCode,
}

impl S3PolicyError {
    pub const fn kind(self) -> S3PolicyErrorKind {
        self.kind
    }

    pub const fn code(self) -> S3PolicyErrorCode {
        self.code
    }

    const fn syntax(code: S3PolicyErrorCode) -> Self {
        Self {
            kind: S3PolicyErrorKind::Syntax,
            code,
        }
    }

    const fn validation(code: S3PolicyErrorCode) -> Self {
        Self {
            kind: S3PolicyErrorKind::Validation,
            code,
        }
    }

    const fn limit(code: S3PolicyErrorCode) -> Self {
        Self {
            kind: S3PolicyErrorKind::LimitExceeded,
            code,
        }
    }
}

impl fmt::Display for S3PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self.kind {
            S3PolicyErrorKind::Syntax => "bucket policy JSON is malformed",
            S3PolicyErrorKind::Validation => "bucket policy is not valid",
            S3PolicyErrorKind::LimitExceeded => "bucket policy exceeds a supported limit",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for S3PolicyError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3PolicyEffect {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3PolicyDecision {
    Allow,
    ExplicitDeny,
    ImplicitDeny,
}

impl S3PolicyDecision {
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allow)
    }
}

/// Conservative public-access status for an S3 bucket policy.
///
/// `is_public` means that at least one supported `Allow` statement may grant
/// access outside the bucket owner's account without a restriction that this
/// analyzer can prove trustworthy. The analyzer deliberately prefers false
/// positives over false negatives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct S3BucketPolicyStatus {
    is_public: bool,
}

impl S3BucketPolicyStatus {
    pub const fn is_public(self) -> bool {
        self.is_public
    }

    pub const fn is_trusted(self) -> bool {
        !self.is_public
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum S3PolicyResourceScope {
    Account,
    Bucket,
    Object,
}

/// Actions currently understood by PrismArk's S3 policy layer.
/// Copy and multipart operations use AWS's underlying GetObject/PutObject/
/// AbortMultipartUpload permissions rather than inventing non-standard
/// actions. Account-level actions are accepted only by identity policies; a
/// policy attached to one bucket cannot authorize them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum S3PolicyAction {
    ListAllMyBuckets,
    CreateBucket,
    ListBucket,
    ListBucketVersions,
    ListBucketMultipartUploads,
    GetBucketLocation,
    GetBucketVersioning,
    PutBucketVersioning,
    GetLifecycleConfiguration,
    PutLifecycleConfiguration,
    DeleteLifecycleConfiguration,
    GetBucketObjectLockConfiguration,
    PutBucketObjectLockConfiguration,
    GetBucketPolicy,
    GetBucketPolicyStatus,
    PutBucketPolicy,
    DeleteBucketPolicy,
    DeleteBucket,
    GetObject,
    GetObjectVersion,
    GetObjectAcl,
    PutObjectAcl,
    PutObject,
    DeleteObject,
    DeleteObjectVersion,
    GetObjectTagging,
    GetObjectVersionTagging,
    PutObjectTagging,
    PutObjectVersionTagging,
    DeleteObjectTagging,
    DeleteObjectVersionTagging,
    GetObjectRetention,
    PutObjectRetention,
    GetObjectLegalHold,
    PutObjectLegalHold,
    /// Authorizes governance bypass only. Callers must separately authorize
    /// the requested retention mutation or version deletion action.
    BypassGovernanceRetention,
    AbortMultipartUpload,
    ListMultipartUploadParts,
}

impl S3PolicyAction {
    pub const ALL: [Self; 38] = [
        Self::ListAllMyBuckets,
        Self::CreateBucket,
        Self::ListBucket,
        Self::ListBucketVersions,
        Self::ListBucketMultipartUploads,
        Self::GetBucketLocation,
        Self::GetBucketVersioning,
        Self::PutBucketVersioning,
        Self::GetLifecycleConfiguration,
        Self::PutLifecycleConfiguration,
        Self::DeleteLifecycleConfiguration,
        Self::GetBucketObjectLockConfiguration,
        Self::PutBucketObjectLockConfiguration,
        Self::GetBucketPolicy,
        Self::GetBucketPolicyStatus,
        Self::PutBucketPolicy,
        Self::DeleteBucketPolicy,
        Self::DeleteBucket,
        Self::GetObject,
        Self::GetObjectVersion,
        Self::GetObjectAcl,
        Self::PutObjectAcl,
        Self::PutObject,
        Self::DeleteObject,
        Self::DeleteObjectVersion,
        Self::GetObjectTagging,
        Self::GetObjectVersionTagging,
        Self::PutObjectTagging,
        Self::PutObjectVersionTagging,
        Self::DeleteObjectTagging,
        Self::DeleteObjectVersionTagging,
        Self::GetObjectRetention,
        Self::PutObjectRetention,
        Self::GetObjectLegalHold,
        Self::PutObjectLegalHold,
        Self::BypassGovernanceRetention,
        Self::AbortMultipartUpload,
        Self::ListMultipartUploadParts,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ListAllMyBuckets => "s3:ListAllMyBuckets",
            Self::CreateBucket => "s3:CreateBucket",
            Self::ListBucket => "s3:ListBucket",
            Self::ListBucketVersions => "s3:ListBucketVersions",
            Self::ListBucketMultipartUploads => "s3:ListBucketMultipartUploads",
            Self::GetBucketLocation => "s3:GetBucketLocation",
            Self::GetBucketVersioning => "s3:GetBucketVersioning",
            Self::PutBucketVersioning => "s3:PutBucketVersioning",
            Self::GetLifecycleConfiguration => "s3:GetLifecycleConfiguration",
            Self::PutLifecycleConfiguration => "s3:PutLifecycleConfiguration",
            Self::DeleteLifecycleConfiguration => "s3:DeleteLifecycleConfiguration",
            Self::GetBucketObjectLockConfiguration => "s3:GetBucketObjectLockConfiguration",
            Self::PutBucketObjectLockConfiguration => "s3:PutBucketObjectLockConfiguration",
            Self::GetBucketPolicy => "s3:GetBucketPolicy",
            Self::GetBucketPolicyStatus => "s3:GetBucketPolicyStatus",
            Self::PutBucketPolicy => "s3:PutBucketPolicy",
            Self::DeleteBucketPolicy => "s3:DeleteBucketPolicy",
            Self::DeleteBucket => "s3:DeleteBucket",
            Self::GetObject => "s3:GetObject",
            Self::GetObjectVersion => "s3:GetObjectVersion",
            Self::GetObjectAcl => "s3:GetObjectAcl",
            Self::PutObjectAcl => "s3:PutObjectAcl",
            Self::PutObject => "s3:PutObject",
            Self::DeleteObject => "s3:DeleteObject",
            Self::DeleteObjectVersion => "s3:DeleteObjectVersion",
            Self::GetObjectTagging => "s3:GetObjectTagging",
            Self::GetObjectVersionTagging => "s3:GetObjectVersionTagging",
            Self::PutObjectTagging => "s3:PutObjectTagging",
            Self::PutObjectVersionTagging => "s3:PutObjectVersionTagging",
            Self::DeleteObjectTagging => "s3:DeleteObjectTagging",
            Self::DeleteObjectVersionTagging => "s3:DeleteObjectVersionTagging",
            Self::GetObjectRetention => "s3:GetObjectRetention",
            Self::PutObjectRetention => "s3:PutObjectRetention",
            Self::GetObjectLegalHold => "s3:GetObjectLegalHold",
            Self::PutObjectLegalHold => "s3:PutObjectLegalHold",
            Self::BypassGovernanceRetention => "s3:BypassGovernanceRetention",
            Self::AbortMultipartUpload => "s3:AbortMultipartUpload",
            Self::ListMultipartUploadParts => "s3:ListMultipartUploadParts",
        }
    }

    pub const fn scope(self) -> S3PolicyResourceScope {
        match self {
            Self::ListAllMyBuckets | Self::CreateBucket => S3PolicyResourceScope::Account,
            Self::ListBucket
            | Self::ListBucketVersions
            | Self::ListBucketMultipartUploads
            | Self::GetBucketLocation
            | Self::GetBucketVersioning
            | Self::PutBucketVersioning
            | Self::GetLifecycleConfiguration
            | Self::PutLifecycleConfiguration
            | Self::DeleteLifecycleConfiguration
            | Self::GetBucketObjectLockConfiguration
            | Self::PutBucketObjectLockConfiguration
            | Self::GetBucketPolicy
            | Self::GetBucketPolicyStatus
            | Self::PutBucketPolicy
            | Self::DeleteBucketPolicy
            | Self::DeleteBucket => S3PolicyResourceScope::Bucket,
            _ => S3PolicyResourceScope::Object,
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|action| action.as_str().eq_ignore_ascii_case(name))
    }
}

const S3_BUCKET_POLICY_ACTIONS: [S3PolicyAction; 36] = [
    S3PolicyAction::ListBucket,
    S3PolicyAction::ListBucketVersions,
    S3PolicyAction::ListBucketMultipartUploads,
    S3PolicyAction::GetBucketLocation,
    S3PolicyAction::GetBucketVersioning,
    S3PolicyAction::PutBucketVersioning,
    S3PolicyAction::GetLifecycleConfiguration,
    S3PolicyAction::PutLifecycleConfiguration,
    S3PolicyAction::DeleteLifecycleConfiguration,
    S3PolicyAction::GetBucketObjectLockConfiguration,
    S3PolicyAction::PutBucketObjectLockConfiguration,
    S3PolicyAction::GetBucketPolicy,
    S3PolicyAction::GetBucketPolicyStatus,
    S3PolicyAction::PutBucketPolicy,
    S3PolicyAction::DeleteBucketPolicy,
    S3PolicyAction::DeleteBucket,
    S3PolicyAction::GetObject,
    S3PolicyAction::GetObjectVersion,
    S3PolicyAction::GetObjectAcl,
    S3PolicyAction::PutObjectAcl,
    S3PolicyAction::PutObject,
    S3PolicyAction::DeleteObject,
    S3PolicyAction::DeleteObjectVersion,
    S3PolicyAction::GetObjectTagging,
    S3PolicyAction::GetObjectVersionTagging,
    S3PolicyAction::PutObjectTagging,
    S3PolicyAction::PutObjectVersionTagging,
    S3PolicyAction::DeleteObjectTagging,
    S3PolicyAction::DeleteObjectVersionTagging,
    S3PolicyAction::GetObjectRetention,
    S3PolicyAction::PutObjectRetention,
    S3PolicyAction::GetObjectLegalHold,
    S3PolicyAction::PutObjectLegalHold,
    S3PolicyAction::BypassGovernanceRetention,
    S3PolicyAction::AbortMultipartUpload,
    S3PolicyAction::ListMultipartUploadParts,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3AwsPrincipal {
    Any,
    Account(String),
    Arn(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3PolicyPrincipal {
    Any,
    Aws(Vec<S3AwsPrincipal>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3PrincipalClause {
    Principal(S3PolicyPrincipal),
    NotPrincipal(S3PolicyPrincipal),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3ActionClause {
    Action(Vec<String>),
    NotAction(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3ResourceClause {
    Resource(Vec<String>),
    NotResource(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3ConditionOperator {
    StringEquals,
    StringNotEquals,
    StringLike,
    StringNotLike,
    Bool,
    IpAddress,
    NotIpAddress,
    NumericLessThanEquals,
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3ConditionKey {
    SecureTransport,
    SourceIp,
    PrincipalArn,
    UserId,
    Prefix,
    Delimiter,
    MaxKeys,
    VersionId,
}

#[derive(Clone, Debug, PartialEq)]
enum S3ConditionValue {
    String(String),
    Bool(bool),
    Ip(IpNetwork),
    Unsigned(u64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct S3PolicyCondition {
    operator: S3ConditionOperator,
    key: S3ConditionKey,
    values: Vec<S3ConditionValue>,
}

impl S3PolicyCondition {
    pub const fn operator(&self) -> S3ConditionOperator {
        self.operator
    }

    pub const fn key(&self) -> S3ConditionKey {
        self.key
    }

    pub fn value_count(&self) -> usize {
        self.values.len()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct S3PolicyStatement {
    sid: Option<String>,
    effect: S3PolicyEffect,
    principal: S3PrincipalClause,
    actions: S3ActionClause,
    resources: S3ResourceClause,
    conditions: Vec<S3PolicyCondition>,
}

impl S3PolicyStatement {
    pub fn sid(&self) -> Option<&str> {
        self.sid.as_deref()
    }

    pub const fn effect(&self) -> S3PolicyEffect {
        self.effect
    }

    pub const fn principal(&self) -> &S3PrincipalClause {
        &self.principal
    }

    pub const fn actions(&self) -> &S3ActionClause {
        &self.actions
    }

    pub const fn resources(&self) -> &S3ResourceClause {
        &self.resources
    }

    pub fn conditions(&self) -> &[S3PolicyCondition] {
        &self.conditions
    }
}

/// A statement attached to an authenticated S3 identity.
///
/// Identity statements deliberately have no principal clause: the policy is
/// already scoped by the identity it is attached to. All other clauses share
/// the bucket-policy parser, validators, matcher, and stable serializer.
#[derive(Clone, Debug, PartialEq)]
pub struct S3IdentityPolicyStatement {
    sid: Option<String>,
    effect: S3PolicyEffect,
    actions: S3ActionClause,
    resources: S3ResourceClause,
    conditions: Vec<S3PolicyCondition>,
}

impl S3IdentityPolicyStatement {
    pub fn sid(&self) -> Option<&str> {
        self.sid.as_deref()
    }

    pub const fn effect(&self) -> S3PolicyEffect {
        self.effect
    }

    pub const fn actions(&self) -> &S3ActionClause {
        &self.actions
    }

    pub const fn resources(&self) -> &S3ResourceClause {
        &self.resources
    }

    pub fn conditions(&self) -> &[S3PolicyCondition] {
        &self.conditions
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct S3IdentityPolicy {
    version: &'static str,
    id: Option<String>,
    statements: Vec<S3IdentityPolicyStatement>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct S3BucketPolicy {
    version: &'static str,
    id: Option<String>,
    target_bucket: String,
    statements: Vec<S3PolicyStatement>,
}

impl S3BucketPolicy {
    pub fn parse(document: &[u8], target_bucket: &str) -> Result<Self, S3PolicyError> {
        if !valid_bucket_name(target_bucket) {
            return Err(S3PolicyError::validation(
                S3PolicyErrorCode::InvalidResource,
            ));
        }
        let parsed = parse_policy_document(document, |value| {
            parse_statement(value, S3PolicyKind::Bucket(target_bucket))
        })?;
        let statements = parsed
            .statements
            .into_iter()
            .map(|statement| S3PolicyStatement {
                sid: statement.sid,
                effect: statement.effect,
                principal: statement
                    .principal
                    .expect("bucket-policy parser requires a principal"),
                actions: statement.actions,
                resources: statement.resources,
                conditions: statement.conditions,
            })
            .collect();
        Ok(Self {
            version: parsed.version,
            id: parsed.id,
            target_bucket: target_bucket.to_owned(),
            statements,
        })
    }

    pub fn version(&self) -> &'static str {
        self.version
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn statements(&self) -> &[S3PolicyStatement] {
        &self.statements
    }

    pub fn target_bucket(&self) -> &str {
        &self.target_bucket
    }

    /// Emits a compact, deterministic JSON policy document.
    ///
    /// Object fields, set-like clauses, condition operators, condition keys,
    /// and condition values use stable ordering. Statements retain their
    /// authored order. Scalar-or-array policy fields are normalized to a
    /// scalar when they contain one value and to an array otherwise. The
    /// returned UTF-8 bytes are therefore suitable for both `GetBucketPolicy`
    /// responses and stable content hashing.
    pub fn to_stable_json(&self) -> String {
        write_policy_json(
            self.version,
            self.id.as_deref(),
            &self.statements,
            write_bucket_statement_json,
        )
    }

    /// Computes the conservative public/trusted status used by
    /// `GetBucketPolicyStatus`.
    ///
    /// `owning_account_id` must be the bucket owner's 12-digit S3 account ID.
    /// An invalid account context fails closed whenever the policy contains an
    /// `Allow` statement. Deny-only policies are never public.
    pub fn policy_status(&self, owning_account_id: &str) -> S3BucketPolicyStatus {
        let allow_statements = self
            .statements
            .iter()
            .filter(|statement| statement.effect == S3PolicyEffect::Allow)
            .collect::<Vec<_>>();
        if allow_statements.is_empty() {
            return S3BucketPolicyStatus { is_public: false };
        }
        if !valid_account_id(owning_account_id) {
            return S3BucketPolicyStatus { is_public: true };
        }

        S3BucketPolicyStatus {
            is_public: allow_statements
                .into_iter()
                .any(|statement| statement_may_grant_public_access(statement, owning_account_id)),
        }
    }

    pub fn evaluate(&self, request: &S3PolicyRequest<'_>) -> S3PolicyDecision {
        if request.bucket != self.target_bucket {
            return S3PolicyDecision::ImplicitDeny;
        }
        let Some(request) = request.evaluation_request() else {
            return S3PolicyDecision::ImplicitDeny;
        };
        evaluate_statements(
            self.statements.iter(),
            |statement| statement.matches(&request),
            |statement| statement.effect,
        )
    }
}

impl S3IdentityPolicy {
    pub fn parse(document: &[u8]) -> Result<Self, S3PolicyError> {
        let parsed = parse_policy_document(document, |value| {
            parse_statement(value, S3PolicyKind::Identity)
        })?;
        let statements = parsed
            .statements
            .into_iter()
            .map(|statement| S3IdentityPolicyStatement {
                sid: statement.sid,
                effect: statement.effect,
                actions: statement.actions,
                resources: statement.resources,
                conditions: statement.conditions,
            })
            .collect();
        Ok(Self {
            version: parsed.version,
            id: parsed.id,
            statements,
        })
    }

    pub fn version(&self) -> &'static str {
        self.version
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn statements(&self) -> &[S3IdentityPolicyStatement] {
        &self.statements
    }

    pub fn to_stable_json(&self) -> String {
        write_policy_json(
            self.version,
            self.id.as_deref(),
            &self.statements,
            write_identity_statement_json,
        )
    }

    pub fn evaluate(&self, request: &S3IdentityPolicyRequest<'_>) -> S3PolicyDecision {
        let Some(request) = request.evaluation_request() else {
            return S3PolicyDecision::ImplicitDeny;
        };
        evaluate_statements(
            self.statements.iter(),
            |statement| statement.matches(&request),
            |statement| statement.effect,
        )
    }
}

fn write_policy_json<T>(
    version: &str,
    id: Option<&str>,
    statements: &[T],
    write_statement: fn(&mut String, &T),
) -> String {
    let mut output = String::new();
    output.push('{');
    push_json_name(&mut output, "Version");
    push_json_string(&mut output, version);
    if let Some(id) = id {
        output.push(',');
        push_json_name(&mut output, "Id");
        push_json_string(&mut output, id);
    }
    output.push(',');
    push_json_name(&mut output, "Statement");
    output.push('[');
    for (index, statement) in statements.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write_statement(&mut output, statement);
    }
    output.push(']');
    output.push('}');
    output
}

fn write_bucket_statement_json(output: &mut String, statement: &S3PolicyStatement) {
    write_statement_json(
        output,
        statement.sid.as_deref(),
        statement.effect,
        Some(&statement.principal),
        &statement.actions,
        &statement.resources,
        &statement.conditions,
    );
}

fn write_identity_statement_json(output: &mut String, statement: &S3IdentityPolicyStatement) {
    write_statement_json(
        output,
        statement.sid.as_deref(),
        statement.effect,
        None,
        &statement.actions,
        &statement.resources,
        &statement.conditions,
    );
}

fn write_statement_json(
    output: &mut String,
    sid: Option<&str>,
    effect: S3PolicyEffect,
    principal: Option<&S3PrincipalClause>,
    actions: &S3ActionClause,
    resources: &S3ResourceClause,
    conditions: &[S3PolicyCondition],
) {
    output.push('{');
    let mut has_field = false;
    if let Some(sid) = sid {
        push_json_field_separator(output, &mut has_field);
        push_json_name(output, "Sid");
        push_json_string(output, sid);
    }
    push_json_field_separator(output, &mut has_field);
    push_json_name(output, "Effect");
    push_json_string(
        output,
        match effect {
            S3PolicyEffect::Allow => "Allow",
            S3PolicyEffect::Deny => "Deny",
        },
    );

    if let Some(principal) = principal {
        push_json_field_separator(output, &mut has_field);
        match principal {
            S3PrincipalClause::Principal(principal) => {
                push_json_name(output, "Principal");
                write_principal_json(output, principal);
            }
            S3PrincipalClause::NotPrincipal(principal) => {
                push_json_name(output, "NotPrincipal");
                write_principal_json(output, principal);
            }
        }
    }

    push_json_field_separator(output, &mut has_field);
    match actions {
        S3ActionClause::Action(patterns) => {
            push_json_name(output, "Action");
            write_sorted_string_values(output, patterns);
        }
        S3ActionClause::NotAction(patterns) => {
            push_json_name(output, "NotAction");
            write_sorted_string_values(output, patterns);
        }
    }

    push_json_field_separator(output, &mut has_field);
    match resources {
        S3ResourceClause::Resource(patterns) => {
            push_json_name(output, "Resource");
            write_sorted_string_values(output, patterns);
        }
        S3ResourceClause::NotResource(patterns) => {
            push_json_name(output, "NotResource");
            write_sorted_string_values(output, patterns);
        }
    }

    if !conditions.is_empty() {
        push_json_field_separator(output, &mut has_field);
        push_json_name(output, "Condition");
        write_conditions_json(output, conditions);
    }
    output.push('}');
}

fn write_principal_json(output: &mut String, principal: &S3PolicyPrincipal) {
    match principal {
        S3PolicyPrincipal::Any => push_json_string(output, "*"),
        S3PolicyPrincipal::Aws(principals) => {
            output.push('{');
            push_json_name(output, "AWS");
            let mut values = principals
                .iter()
                .map(|principal| match principal {
                    S3AwsPrincipal::Any => "*",
                    S3AwsPrincipal::Account(account) | S3AwsPrincipal::Arn(account) => account,
                })
                .collect::<Vec<_>>();
            values.sort_unstable();
            write_string_values(output, &values);
            output.push('}');
        }
    }
}

fn write_sorted_string_values(output: &mut String, values: &[String]) {
    let mut values = values.iter().map(String::as_str).collect::<Vec<_>>();
    values.sort_unstable();
    write_string_values(output, &values);
}

fn write_string_values(output: &mut String, values: &[&str]) {
    if values.len() == 1 {
        push_json_string(output, values[0]);
        return;
    }
    output.push('[');
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        push_json_string(output, value);
    }
    output.push(']');
}

fn write_conditions_json(output: &mut String, conditions: &[S3PolicyCondition]) {
    let mut conditions = conditions.iter().collect::<Vec<_>>();
    conditions.sort_by_key(|condition| {
        (
            condition_operator_order(condition.operator),
            condition_key_order(condition.key),
        )
    });

    output.push('{');
    let mut index = 0;
    let mut first_operator = true;
    while index < conditions.len() {
        let operator = conditions[index].operator;
        if !first_operator {
            output.push(',');
        }
        first_operator = false;
        push_json_name(output, condition_operator_name(operator));
        output.push('{');

        let mut first_key = true;
        while index < conditions.len() && conditions[index].operator == operator {
            if !first_key {
                output.push(',');
            }
            first_key = false;
            push_json_name(output, condition_key_name(conditions[index].key));
            write_condition_values(output, &conditions[index].values);
            index += 1;
        }
        output.push('}');
    }
    output.push('}');
}

fn write_condition_values(output: &mut String, values: &[S3ConditionValue]) {
    let mut values = values.iter().collect::<Vec<_>>();
    values.sort_by(|left, right| {
        condition_value_sort_key(left).cmp(&condition_value_sort_key(right))
    });
    if values.len() == 1 {
        write_condition_value(output, values[0]);
        return;
    }
    output.push('[');
    for (index, value) in values.into_iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        write_condition_value(output, value);
    }
    output.push(']');
}

fn write_condition_value(output: &mut String, value: &S3ConditionValue) {
    match value {
        S3ConditionValue::String(value) => push_json_string(output, value),
        S3ConditionValue::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        S3ConditionValue::Ip(network) => {
            push_json_string(output, &format!("{}/{}", network.network, network.prefix));
        }
        S3ConditionValue::Unsigned(value) => output.push_str(&value.to_string()),
    }
}

fn condition_value_sort_key(value: &S3ConditionValue) -> String {
    match value {
        S3ConditionValue::String(value) => format!("0:{value}"),
        S3ConditionValue::Bool(value) => format!("1:{value}"),
        S3ConditionValue::Ip(network) => format!("2:{}/{}", network.network, network.prefix),
        S3ConditionValue::Unsigned(value) => format!("3:{value:020}"),
    }
}

const fn condition_operator_order(operator: S3ConditionOperator) -> u8 {
    match operator {
        S3ConditionOperator::StringEquals => 0,
        S3ConditionOperator::StringNotEquals => 1,
        S3ConditionOperator::StringLike => 2,
        S3ConditionOperator::StringNotLike => 3,
        S3ConditionOperator::Bool => 4,
        S3ConditionOperator::IpAddress => 5,
        S3ConditionOperator::NotIpAddress => 6,
        S3ConditionOperator::NumericLessThanEquals => 7,
        S3ConditionOperator::Null => 8,
    }
}

const fn condition_operator_name(operator: S3ConditionOperator) -> &'static str {
    match operator {
        S3ConditionOperator::StringEquals => "StringEquals",
        S3ConditionOperator::StringNotEquals => "StringNotEquals",
        S3ConditionOperator::StringLike => "StringLike",
        S3ConditionOperator::StringNotLike => "StringNotLike",
        S3ConditionOperator::Bool => "Bool",
        S3ConditionOperator::IpAddress => "IpAddress",
        S3ConditionOperator::NotIpAddress => "NotIpAddress",
        S3ConditionOperator::NumericLessThanEquals => "NumericLessThanEquals",
        S3ConditionOperator::Null => "Null",
    }
}

const fn condition_key_order(key: S3ConditionKey) -> u8 {
    match key {
        S3ConditionKey::SecureTransport => 0,
        S3ConditionKey::SourceIp => 1,
        S3ConditionKey::PrincipalArn => 2,
        S3ConditionKey::UserId => 3,
        S3ConditionKey::Prefix => 4,
        S3ConditionKey::Delimiter => 5,
        S3ConditionKey::MaxKeys => 6,
        S3ConditionKey::VersionId => 7,
    }
}

const fn condition_key_name(key: S3ConditionKey) -> &'static str {
    match key {
        S3ConditionKey::SecureTransport => "aws:SecureTransport",
        S3ConditionKey::SourceIp => "aws:SourceIp",
        S3ConditionKey::PrincipalArn => "aws:PrincipalArn",
        S3ConditionKey::UserId => "aws:userid",
        S3ConditionKey::Prefix => "s3:prefix",
        S3ConditionKey::Delimiter => "s3:delimiter",
        S3ConditionKey::MaxKeys => "s3:max-keys",
        S3ConditionKey::VersionId => "s3:VersionId",
    }
}

fn push_json_field_separator(output: &mut String, has_field: &mut bool) {
    if *has_field {
        output.push(',');
    }
    *has_field = true;
}

fn push_json_name(output: &mut String, name: &str) {
    push_json_string(output, name);
    output.push(':');
}

fn push_json_string(output: &mut String, value: &str) {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{0008}' => output.push_str("\\b"),
            '\u{000c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character <= '\u{001f}' => {
                let code = character as usize;
                output.push_str("\\u00");
                output.push(HEX[(code >> 4) & 0x0f] as char);
                output.push(HEX[code & 0x0f] as char);
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

fn statement_may_grant_public_access(
    statement: &S3PolicyStatement,
    owning_account_id: &str,
) -> bool {
    debug_assert_eq!(statement.effect, S3PolicyEffect::Allow);

    if principal_clause_matches_nobody(&statement.principal) {
        return false;
    }
    if principal_clause_is_trusted(&statement.principal, owning_account_id) {
        return false;
    }
    if statement
        .conditions
        .iter()
        .any(|condition| condition_restricts_to_trusted_principals(condition, owning_account_id))
    {
        return false;
    }
    if statement
        .conditions
        .iter()
        .any(condition_restricts_to_exact_source_ips)
    {
        return false;
    }
    true
}

fn principal_clause_matches_nobody(clause: &S3PrincipalClause) -> bool {
    matches!(
        clause,
        S3PrincipalClause::NotPrincipal(principal) if principal_matches_everybody(principal)
    )
}

fn principal_matches_everybody(principal: &S3PolicyPrincipal) -> bool {
    match principal {
        S3PolicyPrincipal::Any => true,
        S3PolicyPrincipal::Aws(principals) => principals
            .iter()
            .any(|principal| principal == &S3AwsPrincipal::Any),
    }
}

fn principal_clause_is_trusted(clause: &S3PrincipalClause, owning_account_id: &str) -> bool {
    let S3PrincipalClause::Principal(principal) = clause else {
        return false;
    };
    match principal {
        S3PolicyPrincipal::Any => false,
        S3PolicyPrincipal::Aws(principals) => principals
            .iter()
            .all(|principal| principal_is_trusted(principal, owning_account_id)),
    }
}

fn principal_is_trusted(principal: &S3AwsPrincipal, owning_account_id: &str) -> bool {
    match principal {
        S3AwsPrincipal::Any => false,
        S3AwsPrincipal::Account(account) => account == owning_account_id,
        S3AwsPrincipal::Arn(arn) => principal_arn_account(arn) == Some(owning_account_id),
    }
}

fn principal_arn_account(arn: &str) -> Option<&str> {
    let mut parts = arn.splitn(6, ':');
    (parts.next()? == "arn").then_some(())?;
    (parts.next()? == "aws").then_some(())?;
    let service = parts.next()?;
    matches!(service, "iam" | "sts").then_some(())?;
    parts.next()?.is_empty().then_some(())?;
    let account = parts.next()?;
    valid_account_id(account).then_some(account)
}

fn condition_restricts_to_trusted_principals(
    condition: &S3PolicyCondition,
    owning_account_id: &str,
) -> bool {
    if condition.key != S3ConditionKey::PrincipalArn
        || !matches!(
            condition.operator,
            S3ConditionOperator::StringEquals | S3ConditionOperator::StringLike
        )
    {
        return false;
    }
    condition.values.iter().all(|value| {
        let S3ConditionValue::String(arn) = value else {
            return false;
        };
        !arn.contains('*')
            && !arn.contains('?')
            && valid_runtime_principal_arn(arn)
            && principal_arn_account(arn) == Some(owning_account_id)
    })
}

fn condition_restricts_to_exact_source_ips(condition: &S3PolicyCondition) -> bool {
    condition.key == S3ConditionKey::SourceIp
        && condition.operator == S3ConditionOperator::IpAddress
        && condition.values.iter().all(|value| {
            matches!(
                value,
                S3ConditionValue::Ip(IpNetwork {
                    network: IpAddr::V4(_),
                    prefix: 32
                }) | S3ConditionValue::Ip(IpNetwork {
                    network: IpAddr::V6(_),
                    prefix: 128
                })
            )
        })
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct S3PolicyQuery<'a> {
    pub prefix: Option<&'a str>,
    pub delimiter: Option<&'a str>,
    pub max_keys: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct S3PolicyRequest<'a> {
    pub action: S3PolicyAction,
    pub bucket: &'a str,
    pub key: Option<&'a str>,
    pub version_id: Option<&'a str>,
    pub principal_arn: Option<&'a str>,
    pub principal_account: Option<&'a str>,
    pub userid: Option<&'a str>,
    pub secure_transport: bool,
    pub source_ip: Option<IpAddr>,
    pub query: S3PolicyQuery<'a>,
}

/// Explicit resource addressed by an identity-policy request.
///
/// Account actions do not carry a bucket. This keeps `ListAllMyBuckets` and
/// `CreateBucket` distinct from malformed bucket requests without relying on
/// a magic empty string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3IdentityPolicyResource<'a> {
    Account,
    Bucket(&'a str),
    Object { bucket: &'a str, key: &'a str },
}

impl S3IdentityPolicyResource<'_> {
    pub const fn scope(self) -> S3PolicyResourceScope {
        match self {
            Self::Account => S3PolicyResourceScope::Account,
            Self::Bucket(_) => S3PolicyResourceScope::Bucket,
            Self::Object { .. } => S3PolicyResourceScope::Object,
        }
    }
}

/// Runtime request evaluated against an identity policy.
///
/// The three constructors make the addressed scope explicit. Additional
/// fields are public so protocol adapters can supply the same condition
/// context already used by bucket policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct S3IdentityPolicyRequest<'a> {
    pub action: S3PolicyAction,
    pub resource: S3IdentityPolicyResource<'a>,
    pub version_id: Option<&'a str>,
    pub principal_arn: Option<&'a str>,
    pub principal_account: Option<&'a str>,
    pub userid: Option<&'a str>,
    pub secure_transport: bool,
    pub source_ip: Option<IpAddr>,
    pub query: S3PolicyQuery<'a>,
}

impl<'a> S3IdentityPolicyRequest<'a> {
    pub const fn account(action: S3PolicyAction) -> Self {
        Self::new(action, S3IdentityPolicyResource::Account)
    }

    pub const fn bucket(action: S3PolicyAction, bucket: &'a str) -> Self {
        Self::new(action, S3IdentityPolicyResource::Bucket(bucket))
    }

    pub const fn object(action: S3PolicyAction, bucket: &'a str, key: &'a str) -> Self {
        Self::new(action, S3IdentityPolicyResource::Object { bucket, key })
    }

    const fn new(action: S3PolicyAction, resource: S3IdentityPolicyResource<'a>) -> Self {
        Self {
            action,
            resource,
            version_id: None,
            principal_arn: None,
            principal_account: None,
            userid: None,
            secure_transport: false,
            source_ip: None,
            query: S3PolicyQuery {
                prefix: None,
                delimiter: None,
                max_keys: None,
            },
        }
    }

    fn evaluation_request(&self) -> Option<S3PolicyEvaluationRequest<'_>> {
        if self.action.scope() != self.resource.scope() || !request_context_is_well_formed(self) {
            return None;
        }
        let resource = match self.resource {
            S3IdentityPolicyResource::Account => "*".to_owned(),
            S3IdentityPolicyResource::Bucket(bucket) if valid_bucket_name(bucket) => {
                format!("arn:aws:s3:::{bucket}")
            }
            S3IdentityPolicyResource::Object { bucket, key }
                if valid_bucket_name(bucket)
                    && !key.is_empty()
                    && key.len() <= MAX_S3_POLICY_RUNTIME_KEY_BYTES =>
            {
                format!("arn:aws:s3:::{bucket}/{key}")
            }
            _ => return None,
        };
        Some(S3PolicyEvaluationRequest::new(
            self.action,
            resource,
            self.version_id,
            self.principal_arn,
            self.principal_account,
            self.userid,
            self.secure_transport,
            self.source_ip,
            self.query,
        ))
    }
}

impl S3PolicyRequest<'_> {
    fn is_well_formed(&self) -> bool {
        if !valid_bucket_name(self.bucket) {
            return false;
        }
        match (self.action.scope(), self.key) {
            (S3PolicyResourceScope::Bucket, None) => {}
            (S3PolicyResourceScope::Object, Some(key))
                if !key.is_empty() && key.len() <= MAX_S3_POLICY_RUNTIME_KEY_BYTES => {}
            _ => return false,
        }
        request_context_is_well_formed(self)
    }

    fn evaluation_request(&self) -> Option<S3PolicyEvaluationRequest<'_>> {
        if !self.is_well_formed() {
            return None;
        }
        let resource = match self.key {
            Some(key) => format!("arn:aws:s3:::{}/{key}", self.bucket),
            None => format!("arn:aws:s3:::{}", self.bucket),
        };
        Some(S3PolicyEvaluationRequest::new(
            self.action,
            resource,
            self.version_id,
            self.principal_arn,
            self.principal_account,
            self.userid,
            self.secure_transport,
            self.source_ip,
            self.query,
        ))
    }
}

trait S3PolicyRequestContext {
    fn version_id(&self) -> Option<&str>;
    fn principal_arn(&self) -> Option<&str>;
    fn principal_account(&self) -> Option<&str>;
    fn userid(&self) -> Option<&str>;
    fn query(&self) -> S3PolicyQuery<'_>;
}

impl S3PolicyRequestContext for S3PolicyRequest<'_> {
    fn version_id(&self) -> Option<&str> {
        self.version_id
    }

    fn principal_arn(&self) -> Option<&str> {
        self.principal_arn
    }

    fn principal_account(&self) -> Option<&str> {
        self.principal_account
    }

    fn userid(&self) -> Option<&str> {
        self.userid
    }

    fn query(&self) -> S3PolicyQuery<'_> {
        self.query
    }
}

impl S3PolicyRequestContext for S3IdentityPolicyRequest<'_> {
    fn version_id(&self) -> Option<&str> {
        self.version_id
    }

    fn principal_arn(&self) -> Option<&str> {
        self.principal_arn
    }

    fn principal_account(&self) -> Option<&str> {
        self.principal_account
    }

    fn userid(&self) -> Option<&str> {
        self.userid
    }

    fn query(&self) -> S3PolicyQuery<'_> {
        self.query
    }
}

fn request_context_is_well_formed(request: &impl S3PolicyRequestContext) -> bool {
    request
        .principal_arn()
        .is_none_or(valid_runtime_principal_arn)
        && request.principal_account().is_none_or(valid_account_id)
        && request.userid().is_none_or(|value| {
            !value.is_empty()
                && value.len() <= MAX_S3_POLICY_STRING_BYTES
                && !value.chars().any(char::is_control)
        })
        && request
            .version_id()
            .is_none_or(|value| value.len() <= MAX_S3_POLICY_STRING_BYTES)
        && request
            .query()
            .prefix
            .is_none_or(|value| value.len() <= MAX_S3_POLICY_RUNTIME_KEY_BYTES)
        && request
            .query()
            .delimiter
            .is_none_or(|value| value.len() <= MAX_S3_POLICY_STRING_BYTES)
}

struct S3PolicyEvaluationRequest<'a> {
    action: S3PolicyAction,
    resource: String,
    version_id: Option<&'a str>,
    principal_arn: Option<&'a str>,
    principal_account: Option<&'a str>,
    userid: Option<&'a str>,
    secure_transport: bool,
    source_ip: Option<IpAddr>,
    query: S3PolicyQuery<'a>,
}

impl<'a> S3PolicyEvaluationRequest<'a> {
    #[allow(clippy::too_many_arguments)]
    fn new(
        action: S3PolicyAction,
        resource: String,
        version_id: Option<&'a str>,
        principal_arn: Option<&'a str>,
        principal_account: Option<&'a str>,
        userid: Option<&'a str>,
        secure_transport: bool,
        source_ip: Option<IpAddr>,
        query: S3PolicyQuery<'a>,
    ) -> Self {
        Self {
            action,
            resource,
            version_id,
            principal_arn,
            principal_account,
            userid,
            secure_transport,
            source_ip,
            query,
        }
    }
}

impl S3PolicyStatement {
    fn matches(&self, request: &S3PolicyEvaluationRequest<'_>) -> bool {
        principal_matches(&self.principal, request)
            && action_clause_matches(&self.actions, request.action)
            && resource_clause_matches(&self.resources, request)
            && self
                .conditions
                .iter()
                .all(|condition| condition.matches(request))
    }
}

impl S3IdentityPolicyStatement {
    fn matches(&self, request: &S3PolicyEvaluationRequest<'_>) -> bool {
        action_clause_matches(&self.actions, request.action)
            && resource_clause_matches(&self.resources, request)
            && self
                .conditions
                .iter()
                .all(|condition| condition.matches(request))
    }
}

fn evaluate_statements<'a, T: 'a>(
    statements: impl Iterator<Item = &'a T>,
    matches: impl Fn(&T) -> bool,
    effect: impl Fn(&T) -> S3PolicyEffect,
) -> S3PolicyDecision {
    let mut allowed = false;
    for statement in statements {
        if !matches(statement) {
            continue;
        }
        match effect(statement) {
            S3PolicyEffect::Deny => return S3PolicyDecision::ExplicitDeny,
            S3PolicyEffect::Allow => allowed = true,
        }
    }
    if allowed {
        S3PolicyDecision::Allow
    } else {
        S3PolicyDecision::ImplicitDeny
    }
}

impl S3PolicyCondition {
    fn matches(&self, request: &S3PolicyEvaluationRequest<'_>) -> bool {
        if self.operator == S3ConditionOperator::Null {
            let expected_absent = self
                .values
                .iter()
                .any(|value| matches!(value, S3ConditionValue::Bool(true)));
            return !condition_key_is_present(self.key, request) == expected_absent;
        }

        match self.operator {
            S3ConditionOperator::StringEquals => condition_string_value(self.key, request)
                .is_some_and(|actual| self.string_values_any(|expected| actual == expected)),
            S3ConditionOperator::StringNotEquals => condition_string_value(self.key, request)
                .is_none_or(|actual| self.string_values_all(|expected| actual != expected)),
            S3ConditionOperator::StringLike => condition_string_value(self.key, request)
                .is_some_and(|actual| self.string_values_any(|pattern| glob_matches(pattern, actual, false))),
            S3ConditionOperator::StringNotLike => condition_string_value(self.key, request)
                .is_none_or(|actual| self.string_values_all(|pattern| !glob_matches(pattern, actual, false))),
            S3ConditionOperator::Bool => condition_bool_value(self.key, request).is_some_and(
                |actual| {
                    self.values.iter().any(
                        |expected| matches!(expected, S3ConditionValue::Bool(value) if *value == actual),
                    )
                },
            ),
            S3ConditionOperator::IpAddress => request.source_ip.is_some_and(|actual| {
                self.values.iter().any(
                    |expected| matches!(expected, S3ConditionValue::Ip(network) if network.contains(actual)),
                )
            }),
            S3ConditionOperator::NotIpAddress => request.source_ip.is_none_or(|actual| {
                self.values.iter().all(
                    |expected| matches!(expected, S3ConditionValue::Ip(network) if !network.contains(actual)),
                )
            }),
            S3ConditionOperator::NumericLessThanEquals => request.query.max_keys.is_some_and(
                |actual| {
                    self.values.iter().any(
                        |expected| matches!(expected, S3ConditionValue::Unsigned(limit) if actual <= *limit),
                    )
                },
            ),
            S3ConditionOperator::Null => unreachable!("handled above"),
        }
    }

    fn string_values_any(&self, predicate: impl Fn(&str) -> bool) -> bool {
        self.values.iter().any(|value| match value {
            S3ConditionValue::String(value) => predicate(value),
            _ => false,
        })
    }

    fn string_values_all(&self, predicate: impl Fn(&str) -> bool) -> bool {
        self.values.iter().all(|value| match value {
            S3ConditionValue::String(value) => predicate(value),
            _ => false,
        })
    }
}

struct ParsedPolicyDocument<T> {
    version: &'static str,
    id: Option<String>,
    statements: Vec<T>,
}

fn parse_policy_document<T>(
    document: &[u8],
    parse_statement: impl Fn(&Value) -> Result<T, S3PolicyError>,
) -> Result<ParsedPolicyDocument<T>, S3PolicyError> {
    if document.len() > MAX_S3_POLICY_BYTES {
        return Err(S3PolicyError::limit(S3PolicyErrorCode::PolicyTooLarge));
    }
    if document.is_empty() {
        return Err(S3PolicyError::syntax(S3PolicyErrorCode::MalformedJson));
    }

    let root = parse_unique_json(document)?;
    if value_depth(&root, 1) > MAX_S3_POLICY_DEPTH {
        return Err(S3PolicyError::limit(S3PolicyErrorCode::PolicyTooDeep));
    }
    validate_json_string_limits(&root)?;
    let root = object(&root, S3PolicyErrorCode::InvalidDocument)?;
    reject_unknown_fields(root, &["Version", "Id", "Statement"])?;

    let version = match required_string(root, "Version", S3PolicyErrorCode::UnsupportedVersion)? {
        S3_POLICY_VERSION => S3_POLICY_VERSION,
        S3_POLICY_LEGACY_VERSION => S3_POLICY_LEGACY_VERSION,
        _ => {
            return Err(S3PolicyError::validation(
                S3PolicyErrorCode::UnsupportedVersion,
            ));
        }
    };
    let id = root
        .get("Id")
        .map(|value| {
            value
                .as_str()
                .filter(|value| {
                    !value.is_empty()
                        && value.len() <= MAX_S3_POLICY_SID_BYTES
                        && !value.chars().any(char::is_control)
                })
                .map(ToOwned::to_owned)
                .ok_or_else(|| S3PolicyError::validation(S3PolicyErrorCode::InvalidDocument))
        })
        .transpose()?;

    let statements_value = root
        .get("Statement")
        .ok_or_else(|| S3PolicyError::validation(S3PolicyErrorCode::InvalidDocument))?;
    let statement_values: Vec<&Value> = match statements_value {
        Value::Object(_) => vec![statements_value],
        Value::Array(values) if !values.is_empty() => values.iter().collect(),
        _ => {
            return Err(S3PolicyError::validation(
                S3PolicyErrorCode::InvalidStatement,
            ));
        }
    };
    if statement_values.len() > MAX_S3_POLICY_STATEMENTS {
        return Err(S3PolicyError::limit(S3PolicyErrorCode::LimitExceeded));
    }
    let statements = statement_values
        .into_iter()
        .map(parse_statement)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ParsedPolicyDocument {
        version,
        id,
        statements,
    })
}

#[derive(Clone, Copy)]
enum S3PolicyKind<'a> {
    Bucket(&'a str),
    Identity,
}

impl S3PolicyKind<'_> {
    fn supported_actions(self) -> &'static [S3PolicyAction] {
        match self {
            Self::Bucket(_) => &S3_BUCKET_POLICY_ACTIONS,
            Self::Identity => &S3PolicyAction::ALL,
        }
    }
}

struct ParsedPolicyStatement {
    sid: Option<String>,
    effect: S3PolicyEffect,
    principal: Option<S3PrincipalClause>,
    actions: S3ActionClause,
    resources: S3ResourceClause,
    conditions: Vec<S3PolicyCondition>,
}

fn parse_statement(
    value: &Value,
    policy_kind: S3PolicyKind<'_>,
) -> Result<ParsedPolicyStatement, S3PolicyError> {
    let statement = object(value, S3PolicyErrorCode::InvalidStatement)?;
    let allowed_fields: &[&str] = match policy_kind {
        S3PolicyKind::Bucket(_) => &[
            "Sid",
            "Effect",
            "Principal",
            "NotPrincipal",
            "Action",
            "NotAction",
            "Resource",
            "NotResource",
            "Condition",
        ],
        S3PolicyKind::Identity => &[
            "Sid",
            "Effect",
            "Action",
            "NotAction",
            "Resource",
            "NotResource",
            "Condition",
        ],
    };
    reject_unknown_fields(statement, allowed_fields)?;

    let sid = statement
        .get("Sid")
        .map(|value| {
            let sid = value
                .as_str()
                .ok_or_else(|| S3PolicyError::validation(S3PolicyErrorCode::InvalidStatement))?;
            if sid.len() > MAX_S3_POLICY_SID_BYTES
                || !sid
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
            {
                return Err(S3PolicyError::validation(
                    S3PolicyErrorCode::InvalidStatement,
                ));
            }
            Ok(sid.to_owned())
        })
        .transpose()?;

    let effect = match required_string(statement, "Effect", S3PolicyErrorCode::InvalidStatement)? {
        "Allow" => S3PolicyEffect::Allow,
        "Deny" => S3PolicyEffect::Deny,
        _ => {
            return Err(S3PolicyError::validation(
                S3PolicyErrorCode::InvalidStatement,
            ));
        }
    };

    let principal = match policy_kind {
        S3PolicyKind::Bucket(_) => {
            let principal =
                exclusive_clause(statement, "Principal", "NotPrincipal", parse_principal)?;
            Some(match principal {
                Clause::Positive(value) => S3PrincipalClause::Principal(value),
                Clause::Negative(value) => S3PrincipalClause::NotPrincipal(value),
            })
        }
        S3PolicyKind::Identity => None,
    };

    let actions = exclusive_clause(statement, "Action", "NotAction", |value| {
        parse_patterns(
            value,
            MAX_S3_POLICY_ACTIONS,
            S3PolicyErrorCode::UnsupportedAction,
        )
    })?;
    let actions = match actions {
        Clause::Positive(patterns) => {
            validate_action_patterns(&patterns, false, policy_kind.supported_actions())?;
            S3ActionClause::Action(patterns)
        }
        Clause::Negative(patterns) => {
            validate_action_patterns(&patterns, true, policy_kind.supported_actions())?;
            S3ActionClause::NotAction(patterns)
        }
    };

    let resources = exclusive_clause(statement, "Resource", "NotResource", |value| {
        parse_patterns(
            value,
            MAX_S3_POLICY_RESOURCES,
            S3PolicyErrorCode::InvalidResource,
        )
    })?;
    let resources = match resources {
        Clause::Positive(patterns) => {
            validate_resources(&patterns, policy_kind)?;
            S3ResourceClause::Resource(patterns)
        }
        Clause::Negative(patterns) => {
            validate_resources(&patterns, policy_kind)?;
            S3ResourceClause::NotResource(patterns)
        }
    };

    validate_action_resource_compatibility(&actions, &resources, policy_kind)?;
    let conditions = statement
        .get("Condition")
        .map(parse_conditions)
        .transpose()?
        .unwrap_or_default();

    Ok(ParsedPolicyStatement {
        sid,
        effect,
        principal,
        actions,
        resources,
        conditions,
    })
}

enum Clause<T> {
    Positive(T),
    Negative(T),
}

fn exclusive_clause<T>(
    object: &Map<String, Value>,
    positive: &str,
    negative: &str,
    parse: impl Fn(&Value) -> Result<T, S3PolicyError>,
) -> Result<Clause<T>, S3PolicyError> {
    match (object.get(positive), object.get(negative)) {
        (Some(_), Some(_)) | (None, None) => Err(S3PolicyError::validation(
            S3PolicyErrorCode::InvalidStatement,
        )),
        (Some(value), None) => parse(value).map(Clause::Positive),
        (None, Some(value)) => parse(value).map(Clause::Negative),
    }
}

fn parse_principal(value: &Value) -> Result<S3PolicyPrincipal, S3PolicyError> {
    if value.as_str() == Some("*") {
        return Ok(S3PolicyPrincipal::Any);
    }
    let object = object(value, S3PolicyErrorCode::InvalidPrincipal)?;
    reject_unknown_fields(object, &["AWS"])?;
    if object.len() != 1 {
        return Err(S3PolicyError::validation(
            S3PolicyErrorCode::InvalidPrincipal,
        ));
    }
    let values = parse_string_values(
        object.get("AWS").expect("field checked"),
        MAX_S3_POLICY_PRINCIPALS,
        S3PolicyErrorCode::InvalidPrincipal,
    )?;
    let principals = values
        .into_iter()
        .map(|value| parse_aws_principal(&value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(S3PolicyPrincipal::Aws(principals))
}

fn parse_aws_principal(value: &str) -> Result<S3AwsPrincipal, S3PolicyError> {
    if value == "*" {
        return Ok(S3AwsPrincipal::Any);
    }
    if valid_account_id(value) {
        return Ok(S3AwsPrincipal::Account(value.to_owned()));
    }
    let parts = value.splitn(6, ':').collect::<Vec<_>>();
    let valid = parts.len() == 6
        && parts[0] == "arn"
        && parts[1] == "aws"
        && matches!(parts[2], "iam" | "sts")
        && parts[3].is_empty()
        && valid_account_id(parts[4])
        && match parts[2] {
            "iam" => {
                parts[5] == "root"
                    || ["user/", "role/"]
                        .iter()
                        .any(|prefix| valid_principal_path(parts[5], prefix))
            }
            "sts" => valid_principal_path(parts[5], "assumed-role/"),
            _ => false,
        };
    if !valid || value.len() > MAX_S3_POLICY_STRING_BYTES {
        return Err(S3PolicyError::validation(
            S3PolicyErrorCode::InvalidPrincipal,
        ));
    }
    Ok(S3AwsPrincipal::Arn(value.to_owned()))
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

fn parse_patterns(
    value: &Value,
    limit: usize,
    error_code: S3PolicyErrorCode,
) -> Result<Vec<String>, S3PolicyError> {
    let values = parse_string_values(value, limit, error_code)?;
    for pattern in &values {
        if pattern.len() > MAX_S3_POLICY_STRING_BYTES
            || pattern.chars().any(char::is_control)
            || pattern.contains("${")
            || pattern.contains('\\')
        {
            return Err(S3PolicyError::validation(error_code));
        }
    }
    Ok(values)
}

fn parse_string_values(
    value: &Value,
    limit: usize,
    error_code: S3PolicyErrorCode,
) -> Result<Vec<String>, S3PolicyError> {
    let values = match value {
        Value::String(value) => vec![value.clone()],
        Value::Array(values) if !values.is_empty() => values
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(ToOwned::to_owned)
                    .ok_or_else(|| S3PolicyError::validation(error_code))
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(S3PolicyError::validation(error_code)),
    };
    if values.len() > limit
        || values
            .iter()
            .any(|value| value.is_empty() || value.len() > MAX_S3_POLICY_STRING_BYTES)
    {
        return Err(S3PolicyError::limit(S3PolicyErrorCode::LimitExceeded));
    }
    let unique = values.iter().collect::<BTreeSet<_>>();
    if unique.len() != values.len() {
        return Err(S3PolicyError::validation(error_code));
    }
    Ok(values)
}

fn validate_action_patterns(
    patterns: &[String],
    negative: bool,
    supported_actions: &[S3PolicyAction],
) -> Result<(), S3PolicyError> {
    for pattern in patterns {
        if !pattern
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("s3:"))
            || pattern.len() <= 3
            || pattern[3..].chars().any(|character| {
                !(character.is_ascii_alphanumeric() || matches!(character, '*' | '?'))
            })
        {
            return Err(S3PolicyError::validation(
                S3PolicyErrorCode::UnsupportedAction,
            ));
        }
        let matches_supported = supported_actions
            .iter()
            .any(|action| glob_matches(pattern, action.as_str(), true));
        if !matches_supported {
            return Err(S3PolicyError::validation(
                S3PolicyErrorCode::UnsupportedAction,
            ));
        }
    }
    if negative
        && supported_actions.iter().all(|action| {
            patterns
                .iter()
                .any(|pattern| glob_matches(pattern, action.as_str(), true))
        })
    {
        return Err(S3PolicyError::validation(
            S3PolicyErrorCode::UnsupportedAction,
        ));
    }
    Ok(())
}

fn validate_resources(
    patterns: &[String],
    policy_kind: S3PolicyKind<'_>,
) -> Result<(), S3PolicyError> {
    match policy_kind {
        S3PolicyKind::Bucket(target_bucket) => {
            let bucket_arn = format!("arn:aws:s3:::{target_bucket}");
            for pattern in patterns {
                let valid = pattern == &bucket_arn
                    || pattern.strip_prefix(&bucket_arn).is_some_and(|suffix| {
                        suffix.starts_with('/')
                            && suffix.len() > 1
                            && !suffix.chars().any(char::is_control)
                    });
                if !valid {
                    return Err(S3PolicyError::validation(
                        S3PolicyErrorCode::InvalidResource,
                    ));
                }
            }
        }
        S3PolicyKind::Identity => {
            if patterns
                .iter()
                .any(|pattern| !valid_identity_resource(pattern))
            {
                return Err(S3PolicyError::validation(
                    S3PolicyErrorCode::InvalidResource,
                ));
            }
        }
    }
    Ok(())
}

fn valid_identity_resource(pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    let Some(resource) = pattern.strip_prefix("arn:aws:s3:::") else {
        return false;
    };
    let (bucket, object) = match resource.split_once('/') {
        Some((bucket, object)) => (bucket, Some(object)),
        None => (resource, None),
    };
    if !valid_bucket_pattern(bucket) {
        return false;
    }
    object.is_none_or(|object| {
        !object.is_empty()
            && object.len() <= MAX_S3_POLICY_RUNTIME_KEY_BYTES
            && !object.chars().any(char::is_control)
    })
}

fn valid_bucket_pattern(pattern: &str) -> bool {
    if !pattern.contains(['*', '?']) {
        return valid_bucket_name(pattern);
    }
    (1..=63).contains(&pattern.len())
        && pattern.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'-' | b'*' | b'?')
        })
        && pattern
            .as_bytes()
            .first()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'*' | b'?'))
        && pattern
            .as_bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'*' | b'?'))
        && !pattern.contains("..")
        && !pattern.contains(".-")
        && !pattern.contains("-.")
}

fn validate_action_resource_compatibility(
    actions: &S3ActionClause,
    resources: &S3ResourceClause,
    policy_kind: S3PolicyKind<'_>,
) -> Result<(), S3PolicyError> {
    let scopes = resource_clause_scopes(resources, policy_kind);
    let applicable = policy_kind.supported_actions().iter().filter(|action| {
        let matched = match actions {
            S3ActionClause::Action(patterns) => patterns
                .iter()
                .any(|pattern| glob_matches(pattern, action.as_str(), true)),
            S3ActionClause::NotAction(patterns) => !patterns
                .iter()
                .any(|pattern| glob_matches(pattern, action.as_str(), true)),
        };
        matched && scopes.contains(&action.scope())
    });
    if applicable.count() == 0 {
        return Err(S3PolicyError::validation(
            S3PolicyErrorCode::InvalidResource,
        ));
    }
    Ok(())
}

fn resource_clause_scopes(
    resources: &S3ResourceClause,
    policy_kind: S3PolicyKind<'_>,
) -> BTreeSet<S3PolicyResourceScope> {
    let patterns = match resources {
        S3ResourceClause::Resource(patterns) => patterns,
        S3ResourceClause::NotResource(patterns)
            if matches!(policy_kind, S3PolicyKind::Identity) =>
        {
            if patterns.iter().any(|pattern| pattern == "*") {
                return BTreeSet::new();
            }
            return [
                S3PolicyResourceScope::Account,
                S3PolicyResourceScope::Bucket,
                S3PolicyResourceScope::Object,
            ]
            .into_iter()
            .collect();
        }
        S3ResourceClause::NotResource(patterns) => patterns,
    };
    patterns
        .iter()
        .flat_map(|resource| {
            if resource == "*" {
                return vec![
                    S3PolicyResourceScope::Account,
                    S3PolicyResourceScope::Bucket,
                    S3PolicyResourceScope::Object,
                ];
            }
            if resource["arn:aws:s3:::".len()..].contains('/') {
                vec![S3PolicyResourceScope::Object]
            } else {
                vec![S3PolicyResourceScope::Bucket]
            }
        })
        .collect()
}

fn parse_conditions(value: &Value) -> Result<Vec<S3PolicyCondition>, S3PolicyError> {
    let operators = object(value, S3PolicyErrorCode::UnsupportedCondition)?;
    if operators.is_empty() {
        return Err(S3PolicyError::validation(
            S3PolicyErrorCode::UnsupportedCondition,
        ));
    }
    if operators.len() > MAX_S3_POLICY_CONDITION_OPERATORS {
        return Err(S3PolicyError::limit(S3PolicyErrorCode::LimitExceeded));
    }

    let mut condition_count = 0_usize;
    let mut conditions = Vec::new();
    for (operator_name, entries) in operators {
        let operator = parse_condition_operator(operator_name)?;
        let entries = object(entries, S3PolicyErrorCode::UnsupportedCondition)?;
        if entries.is_empty() {
            return Err(S3PolicyError::validation(
                S3PolicyErrorCode::UnsupportedCondition,
            ));
        }
        condition_count = condition_count.saturating_add(entries.len());
        if condition_count > MAX_S3_POLICY_CONDITION_KEYS {
            return Err(S3PolicyError::limit(S3PolicyErrorCode::LimitExceeded));
        }
        for (key_name, values) in entries {
            let key = parse_condition_key(key_name)?;
            conditions.push(parse_condition(operator, key, values)?);
        }
    }
    Ok(conditions)
}

fn parse_condition_operator(value: &str) -> Result<S3ConditionOperator, S3PolicyError> {
    match value {
        "StringEquals" => Ok(S3ConditionOperator::StringEquals),
        "StringNotEquals" => Ok(S3ConditionOperator::StringNotEquals),
        "StringLike" => Ok(S3ConditionOperator::StringLike),
        "StringNotLike" => Ok(S3ConditionOperator::StringNotLike),
        "Bool" => Ok(S3ConditionOperator::Bool),
        "IpAddress" => Ok(S3ConditionOperator::IpAddress),
        "NotIpAddress" => Ok(S3ConditionOperator::NotIpAddress),
        "NumericLessThanEquals" => Ok(S3ConditionOperator::NumericLessThanEquals),
        "Null" => Ok(S3ConditionOperator::Null),
        _ => Err(S3PolicyError::validation(
            S3PolicyErrorCode::UnsupportedCondition,
        )),
    }
}

fn parse_condition_key(value: &str) -> Result<S3ConditionKey, S3PolicyError> {
    if value.eq_ignore_ascii_case("aws:SecureTransport") {
        Ok(S3ConditionKey::SecureTransport)
    } else if value.eq_ignore_ascii_case("aws:SourceIp") {
        Ok(S3ConditionKey::SourceIp)
    } else if value.eq_ignore_ascii_case("aws:PrincipalArn") {
        Ok(S3ConditionKey::PrincipalArn)
    } else if value.eq_ignore_ascii_case("aws:userid") {
        Ok(S3ConditionKey::UserId)
    } else if value.eq_ignore_ascii_case("s3:prefix") {
        Ok(S3ConditionKey::Prefix)
    } else if value.eq_ignore_ascii_case("s3:delimiter") {
        Ok(S3ConditionKey::Delimiter)
    } else if value.eq_ignore_ascii_case("s3:max-keys") {
        Ok(S3ConditionKey::MaxKeys)
    } else if value.eq_ignore_ascii_case("s3:VersionId") {
        Ok(S3ConditionKey::VersionId)
    } else {
        Err(S3PolicyError::validation(
            S3PolicyErrorCode::UnsupportedCondition,
        ))
    }
}

fn parse_condition(
    operator: S3ConditionOperator,
    key: S3ConditionKey,
    value: &Value,
) -> Result<S3PolicyCondition, S3PolicyError> {
    let valid_pair = match operator {
        S3ConditionOperator::StringEquals
        | S3ConditionOperator::StringNotEquals
        | S3ConditionOperator::StringLike
        | S3ConditionOperator::StringNotLike => matches!(
            key,
            S3ConditionKey::PrincipalArn
                | S3ConditionKey::UserId
                | S3ConditionKey::Prefix
                | S3ConditionKey::Delimiter
                | S3ConditionKey::VersionId
        ),
        S3ConditionOperator::Bool => key == S3ConditionKey::SecureTransport,
        S3ConditionOperator::IpAddress | S3ConditionOperator::NotIpAddress => {
            key == S3ConditionKey::SourceIp
        }
        S3ConditionOperator::NumericLessThanEquals => key == S3ConditionKey::MaxKeys,
        S3ConditionOperator::Null => true,
    };
    if !valid_pair {
        return Err(S3PolicyError::validation(
            S3PolicyErrorCode::UnsupportedCondition,
        ));
    }

    let raw_values = scalar_or_array(value)?;
    if raw_values.is_empty() || raw_values.len() > MAX_S3_POLICY_CONDITION_VALUES {
        return Err(S3PolicyError::limit(S3PolicyErrorCode::LimitExceeded));
    }
    if matches!(
        operator,
        S3ConditionOperator::Bool | S3ConditionOperator::Null
    ) && raw_values.len() != 1
    {
        return Err(S3PolicyError::validation(
            S3PolicyErrorCode::UnsupportedCondition,
        ));
    }

    let values = raw_values
        .into_iter()
        .map(|value| parse_condition_value(operator, value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(S3PolicyCondition {
        operator,
        key,
        values,
    })
}

fn scalar_or_array(value: &Value) -> Result<Vec<&Value>, S3PolicyError> {
    match value {
        Value::Array(values) if !values.is_empty() => Ok(values.iter().collect()),
        Value::String(_) | Value::Bool(_) | Value::Number(_) => Ok(vec![value]),
        _ => Err(S3PolicyError::validation(
            S3PolicyErrorCode::UnsupportedCondition,
        )),
    }
}

fn parse_condition_value(
    operator: S3ConditionOperator,
    value: &Value,
) -> Result<S3ConditionValue, S3PolicyError> {
    match operator {
        S3ConditionOperator::StringEquals
        | S3ConditionOperator::StringNotEquals
        | S3ConditionOperator::StringLike
        | S3ConditionOperator::StringNotLike => {
            let value = condition_string(value)?;
            if matches!(
                operator,
                S3ConditionOperator::StringLike | S3ConditionOperator::StringNotLike
            ) && (value.contains("${") || value.contains('\\'))
            {
                return Err(S3PolicyError::validation(
                    S3PolicyErrorCode::UnsupportedCondition,
                ));
            }
            Ok(S3ConditionValue::String(value.to_owned()))
        }
        S3ConditionOperator::Bool | S3ConditionOperator::Null => {
            let parsed = match value {
                Value::Bool(value) => Some(*value),
                Value::String(value) if value.eq_ignore_ascii_case("true") => Some(true),
                Value::String(value) if value.eq_ignore_ascii_case("false") => Some(false),
                _ => None,
            }
            .ok_or_else(|| S3PolicyError::validation(S3PolicyErrorCode::UnsupportedCondition))?;
            Ok(S3ConditionValue::Bool(parsed))
        }
        S3ConditionOperator::IpAddress | S3ConditionOperator::NotIpAddress => {
            let value = condition_string(value)?;
            let network = IpNetwork::parse(value).ok_or_else(|| {
                S3PolicyError::validation(S3PolicyErrorCode::UnsupportedCondition)
            })?;
            Ok(S3ConditionValue::Ip(network))
        }
        S3ConditionOperator::NumericLessThanEquals => {
            let parsed = match value {
                Value::Number(number) => number.as_u64(),
                Value::String(value) => value.parse::<u64>().ok(),
                _ => None,
            }
            .ok_or_else(|| S3PolicyError::validation(S3PolicyErrorCode::UnsupportedCondition))?;
            Ok(S3ConditionValue::Unsigned(parsed))
        }
    }
}

fn condition_string(value: &Value) -> Result<&str, S3PolicyError> {
    let value = value
        .as_str()
        .ok_or_else(|| S3PolicyError::validation(S3PolicyErrorCode::UnsupportedCondition))?;
    if value.is_empty()
        || value.len() > MAX_S3_POLICY_STRING_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(S3PolicyError::validation(
            S3PolicyErrorCode::UnsupportedCondition,
        ));
    }
    Ok(value)
}

fn principal_matches(clause: &S3PrincipalClause, request: &S3PolicyEvaluationRequest<'_>) -> bool {
    match clause {
        S3PrincipalClause::Principal(principal) => principal_matches_value(principal, request),
        S3PrincipalClause::NotPrincipal(principal) => !principal_matches_value(principal, request),
    }
}

fn principal_matches_value(
    principal: &S3PolicyPrincipal,
    request: &S3PolicyEvaluationRequest<'_>,
) -> bool {
    match principal {
        S3PolicyPrincipal::Any => true,
        S3PolicyPrincipal::Aws(principals) => principals.iter().any(|principal| match principal {
            S3AwsPrincipal::Any => true,
            S3AwsPrincipal::Account(account) => request.principal_account == Some(account.as_str()),
            S3AwsPrincipal::Arn(arn) => {
                request.principal_arn == Some(arn.as_str())
                    || root_arn_account(arn)
                        .is_some_and(|account| request.principal_account == Some(account))
            }
        }),
    }
}

fn root_arn_account(arn: &str) -> Option<&str> {
    let prefix = "arn:aws:iam::";
    arn.strip_prefix(prefix)?.strip_suffix(":root")
}

fn action_clause_matches(clause: &S3ActionClause, action: S3PolicyAction) -> bool {
    match clause {
        S3ActionClause::Action(patterns) => patterns
            .iter()
            .any(|pattern| glob_matches(pattern, action.as_str(), true)),
        S3ActionClause::NotAction(patterns) => !patterns
            .iter()
            .any(|pattern| glob_matches(pattern, action.as_str(), true)),
    }
}

fn resource_clause_matches(
    clause: &S3ResourceClause,
    request: &S3PolicyEvaluationRequest<'_>,
) -> bool {
    match clause {
        S3ResourceClause::Resource(patterns) => patterns
            .iter()
            .any(|pattern| glob_matches(pattern, &request.resource, false)),
        S3ResourceClause::NotResource(patterns) => !patterns
            .iter()
            .any(|pattern| glob_matches(pattern, &request.resource, false)),
    }
}

fn condition_string_value<'a>(
    key: S3ConditionKey,
    request: &'a S3PolicyEvaluationRequest<'a>,
) -> Option<&'a str> {
    match key {
        S3ConditionKey::PrincipalArn => request.principal_arn,
        S3ConditionKey::UserId => request.userid,
        S3ConditionKey::Prefix => request.query.prefix,
        S3ConditionKey::Delimiter => request.query.delimiter,
        S3ConditionKey::VersionId => request.version_id,
        _ => None,
    }
}

fn condition_bool_value(
    key: S3ConditionKey,
    request: &S3PolicyEvaluationRequest<'_>,
) -> Option<bool> {
    (key == S3ConditionKey::SecureTransport).then_some(request.secure_transport)
}

fn condition_key_is_present(key: S3ConditionKey, request: &S3PolicyEvaluationRequest<'_>) -> bool {
    match key {
        S3ConditionKey::SecureTransport => true,
        S3ConditionKey::SourceIp => request.source_ip.is_some(),
        S3ConditionKey::PrincipalArn => request.principal_arn.is_some(),
        S3ConditionKey::UserId => request.userid.is_some(),
        S3ConditionKey::Prefix => request.query.prefix.is_some(),
        S3ConditionKey::Delimiter => request.query.delimiter.is_some(),
        S3ConditionKey::MaxKeys => request.query.max_keys.is_some(),
        S3ConditionKey::VersionId => request.version_id.is_some(),
    }
}

/// Dynamic-programming glob matching with O(pattern * value) time and bounded
/// memory. Only `*` and `?` have special meaning.
fn glob_matches(pattern: &str, value: &str, ascii_case_insensitive: bool) -> bool {
    let pattern = pattern.chars().collect::<Vec<_>>();
    let value = value.chars().collect::<Vec<_>>();
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;
    for pattern_character in pattern {
        let mut current = vec![false; value.len() + 1];
        if pattern_character == '*' {
            current[0] = previous[0];
        }
        for (index, value_character) in value.iter().enumerate() {
            current[index + 1] = if pattern_character == '*' {
                previous[index + 1] || current[index]
            } else {
                previous[index]
                    && (pattern_character == '?'
                        || characters_equal(
                            pattern_character,
                            *value_character,
                            ascii_case_insensitive,
                        ))
            };
        }
        previous = current;
    }
    previous[value.len()]
}

fn characters_equal(left: char, right: char, ascii_case_insensitive: bool) -> bool {
    if ascii_case_insensitive {
        left.eq_ignore_ascii_case(&right)
    } else {
        left == right
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct IpNetwork {
    network: IpAddr,
    prefix: u8,
}

impl IpNetwork {
    fn parse(value: &str) -> Option<Self> {
        let (address, prefix) = match value.split_once('/') {
            Some((address, prefix)) => {
                (address.parse::<IpAddr>().ok()?, prefix.parse::<u8>().ok()?)
            }
            None => {
                let address = value.parse::<IpAddr>().ok()?;
                let prefix = if address.is_ipv4() { 32 } else { 128 };
                return Some(Self {
                    network: address,
                    prefix,
                });
            }
        };
        let max_prefix = if address.is_ipv4() { 32 } else { 128 };
        if prefix > max_prefix {
            return None;
        }
        let network = mask_ip(address, prefix);
        if network != address {
            return None;
        }
        Some(Self { network, prefix })
    }

    fn contains(self, address: IpAddr) -> bool {
        match (self.network, address) {
            (IpAddr::V4(network), IpAddr::V4(address)) => mask_v4(address, self.prefix) == network,
            (IpAddr::V6(network), IpAddr::V6(address)) => mask_v6(address, self.prefix) == network,
            _ => false,
        }
    }
}

fn mask_ip(address: IpAddr, prefix: u8) -> IpAddr {
    match address {
        IpAddr::V4(address) => IpAddr::V4(mask_v4(address, prefix)),
        IpAddr::V6(address) => IpAddr::V6(mask_v6(address, prefix)),
    }
}

fn mask_v4(address: Ipv4Addr, prefix: u8) -> Ipv4Addr {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - prefix)
    };
    Ipv4Addr::from(u32::from(address) & mask)
}

fn mask_v6(address: Ipv6Addr, prefix: u8) -> Ipv6Addr {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - prefix)
    };
    Ipv6Addr::from(u128::from(address) & mask)
}

fn valid_account_id(value: &str) -> bool {
    value.len() == 12 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_runtime_principal_arn(value: &str) -> bool {
    matches!(parse_aws_principal(value), Ok(S3AwsPrincipal::Arn(_)))
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
        && value.parse::<Ipv4Addr>().is_err()
}

fn object(
    value: &Value,
    error_code: S3PolicyErrorCode,
) -> Result<&Map<String, Value>, S3PolicyError> {
    value
        .as_object()
        .ok_or_else(|| S3PolicyError::validation(error_code))
}

fn required_string<'a>(
    object: &'a Map<String, Value>,
    name: &str,
    error_code: S3PolicyErrorCode,
) -> Result<&'a str, S3PolicyError> {
    object
        .get(name)
        .and_then(Value::as_str)
        .filter(|value| value.len() <= MAX_S3_POLICY_STRING_BYTES)
        .ok_or_else(|| S3PolicyError::validation(error_code))
}

fn reject_unknown_fields(
    object: &Map<String, Value>,
    allowed: &[&str],
) -> Result<(), S3PolicyError> {
    if object
        .keys()
        .any(|field| !allowed.contains(&field.as_str()))
    {
        return Err(S3PolicyError::validation(
            S3PolicyErrorCode::UnsupportedField,
        ));
    }
    Ok(())
}

fn value_depth(value: &Value, depth: usize) -> usize {
    match value {
        Value::Array(values) => values
            .iter()
            .map(|value| value_depth(value, depth + 1))
            .max()
            .unwrap_or(depth),
        Value::Object(values) => values
            .values()
            .map(|value| value_depth(value, depth + 1))
            .max()
            .unwrap_or(depth),
        _ => depth,
    }
}

fn validate_json_string_limits(value: &Value) -> Result<(), S3PolicyError> {
    match value {
        Value::String(value) if value.len() > MAX_S3_POLICY_STRING_BYTES => {
            Err(S3PolicyError::limit(S3PolicyErrorCode::LimitExceeded))
        }
        Value::Array(values) => values.iter().try_for_each(validate_json_string_limits),
        Value::Object(values) => {
            if values
                .keys()
                .any(|key| key.len() > MAX_S3_POLICY_STRING_BYTES)
            {
                return Err(S3PolicyError::limit(S3PolicyErrorCode::LimitExceeded));
            }
            values.values().try_for_each(validate_json_string_limits)
        }
        _ => Ok(()),
    }
}

fn parse_unique_json(document: &[u8]) -> Result<Value, S3PolicyError> {
    let mut deserializer = serde_json::Deserializer::from_slice(document);
    let value = UniqueValue::deserialize(&mut deserializer)
        .map_err(|_| S3PolicyError::syntax(S3PolicyErrorCode::MalformedJson))?;
    deserializer
        .end()
        .map_err(|_| S3PolicyError::syntax(S3PolicyErrorCode::MalformedJson))?;
    Ok(value.0)
}

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueValueVisitor)
    }
}

struct UniqueValueVisitor;

impl<'de> Visitor<'de> for UniqueValueVisitor {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(Number::from(value))))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueValue)
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        self.visit_string(value.to_owned())
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(UniqueValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueValue>()? {
            values.push(value.0);
        }
        Ok(UniqueValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut object: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = Map::new();
        while let Some((key, value)) = object.next_entry::<String, UniqueValue>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            values.insert(key, value.0);
        }
        Ok(UniqueValue(Value::Object(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BUCKET: &str = "photos-prod";
    const ALICE_ARN: &str = "arn:aws:iam::123456789012:user/alice";

    fn policy(statement: &str) -> S3BucketPolicy {
        S3BucketPolicy::parse(
            format!(r#"{{"Version":"2012-10-17","Statement":{statement}}}"#).as_bytes(),
            BUCKET,
        )
        .expect("valid policy")
    }

    fn statement(effect: &str, action: &str, resource: &str) -> String {
        format!(
            r#"{{"Effect":"{effect}","Principal":"*","Action":"{action}","Resource":"{resource}"}}"#
        )
    }

    fn request(action: S3PolicyAction, key: Option<&str>) -> S3PolicyRequest<'_> {
        S3PolicyRequest {
            action,
            bucket: BUCKET,
            key,
            version_id: None,
            principal_arn: Some(ALICE_ARN),
            principal_account: Some("123456789012"),
            userid: Some("AIDAALICE"),
            secure_transport: true,
            source_ip: Some("203.0.113.10".parse().expect("IP")),
            query: S3PolicyQuery::default(),
        }
    }

    fn assert_invalid(document: &str, code: S3PolicyErrorCode) {
        let error = S3BucketPolicy::parse(document.as_bytes(), BUCKET).expect_err("must reject");
        assert_eq!(error.code(), code, "document: {document}");
        assert_ne!(error.to_string(), document);
    }

    #[test]
    fn parses_single_or_array_statement_and_preserves_structured_principal() {
        let single = policy(&format!(
            r#"{{"Sid":"Read1","Effect":"Allow","Principal":{{"AWS":["123456789012","{ALICE_ARN}"]}},"Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
        ));
        assert_eq!(single.version(), S3_POLICY_VERSION);
        assert_eq!(single.statements().len(), 1);
        assert_eq!(single.statements()[0].sid(), Some("Read1"));
        assert!(matches!(
            single.statements()[0].principal(),
            S3PrincipalClause::Principal(S3PolicyPrincipal::Aws(values)) if values.len() == 2
        ));

        let legacy_document = format!(
            r#"{{"Version":"2008-10-17","Id":"legacy-policy","Statement":{}}}"#,
            statement("Allow", "s3:GetObject", &format!("arn:aws:s3:::{BUCKET}/*"))
        );
        let legacy = S3BucketPolicy::parse(legacy_document.as_bytes(), BUCKET)
            .expect("legacy AWS policy version and Id");
        assert_eq!(legacy.version(), S3_POLICY_LEGACY_VERSION);
        assert_eq!(legacy.id(), Some("legacy-policy"));

        let array = policy(&format!(
            "[{},{}]",
            statement("Allow", "s3:ListBucket", &format!("arn:aws:s3:::{BUCKET}")),
            statement("Allow", "s3:GetObject", &format!("arn:aws:s3:::{BUCKET}/*"))
        ));
        assert_eq!(array.statements().len(), 2);
    }

    #[test]
    fn explicit_deny_wins_regardless_of_statement_order() {
        let allow = statement("Allow", "s3:GetObject", &format!("arn:aws:s3:::{BUCKET}/*"));
        let deny = statement(
            "Deny",
            "s3:GetObject",
            &format!("arn:aws:s3:::{BUCKET}/private/*"),
        );
        for statements in [format!("[{allow},{deny}]"), format!("[{deny},{allow}]")] {
            let policy = policy(&statements);
            assert_eq!(
                policy.evaluate(&request(S3PolicyAction::GetObject, Some("private/a.jpg"))),
                S3PolicyDecision::ExplicitDeny
            );
            assert_eq!(
                policy.evaluate(&request(S3PolicyAction::GetObject, Some("public/a.jpg"))),
                S3PolicyDecision::Allow
            );
        }
    }

    #[test]
    fn principal_account_root_arn_exact_arn_and_not_principal_are_distinct() {
        let cases = [
            (r#""*""#, true),
            (r#"{"AWS":"123456789012"}"#, true),
            (r#"{"AWS":"arn:aws:iam::123456789012:root"}"#, true),
            (r#"{"AWS":"arn:aws:iam::123456789012:user/alice"}"#, true),
            (r#"{"AWS":"arn:aws:iam::123456789012:user/bob"}"#, false),
        ];
        for (principal, expected) in cases {
            let policy = policy(&format!(
                r#"{{"Effect":"Allow","Principal":{principal},"Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
            ));
            assert_eq!(
                policy
                    .evaluate(&request(S3PolicyAction::GetObject, Some("a")))
                    .is_allowed(),
                expected,
                "principal: {principal}"
            );
        }

        let negative_policy = policy(&format!(
            r#"{{"Effect":"Allow","NotPrincipal":{{"AWS":"arn:aws:iam::123456789012:user/bob"}},"Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
        ));
        assert_eq!(
            negative_policy.evaluate(&request(S3PolicyAction::GetObject, Some("a"))),
            S3PolicyDecision::Allow
        );
    }

    #[test]
    fn bucket_and_object_resources_do_not_cross_scopes() {
        let policy = policy(&format!(
            r#"[{},{}]"#,
            statement("Allow", "s3:ListBucket", &format!("arn:aws:s3:::{BUCKET}")),
            statement(
                "Allow",
                "s3:GetObject",
                &format!("arn:aws:s3:::{BUCKET}/public/*")
            )
        ));
        assert_eq!(
            policy.evaluate(&request(S3PolicyAction::ListBucket, None)),
            S3PolicyDecision::Allow
        );
        assert_eq!(
            policy.evaluate(&request(S3PolicyAction::GetObject, Some("public/a"))),
            S3PolicyDecision::Allow
        );
        assert_eq!(
            policy.evaluate(&request(S3PolicyAction::GetObject, Some("private/a"))),
            S3PolicyDecision::ImplicitDeny
        );
    }

    #[test]
    fn action_wildcards_not_action_and_not_resource_are_bounded_and_correct() {
        let wildcard = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject*","Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
        ));
        assert!(
            wildcard
                .evaluate(&request(S3PolicyAction::GetObjectVersion, Some("x")))
                .is_allowed()
        );
        assert!(
            !wildcard
                .evaluate(&request(S3PolicyAction::PutObject, Some("x")))
                .is_allowed()
        );

        let not_action = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","NotAction":"s3:DeleteObject*","Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
        ));
        assert!(
            not_action
                .evaluate(&request(S3PolicyAction::GetObject, Some("x")))
                .is_allowed()
        );
        assert!(
            !not_action
                .evaluate(&request(S3PolicyAction::DeleteObject, Some("x")))
                .is_allowed()
        );

        let not_resource = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","NotResource":"arn:aws:s3:::{BUCKET}/private/*"}}"#
        ));
        assert!(
            not_resource
                .evaluate(&request(S3PolicyAction::GetObject, Some("public/x")))
                .is_allowed()
        );
        assert!(
            !not_resource
                .evaluate(&request(S3PolicyAction::GetObject, Some("private/x")))
                .is_allowed()
        );

        assert!(glob_matches(&"*a".repeat(500), &"a".repeat(500), false));
    }

    #[test]
    fn list_prefix_delimiter_and_max_keys_conditions_match_request_query_only() {
        let policy = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":"s3:ListBucket","Resource":"arn:aws:s3:::{BUCKET}","Condition":{{"StringLike":{{"s3:prefix":"users/alice/*"}},"StringEquals":{{"s3:delimiter":"/"}},"NumericLessThanEquals":{{"s3:max-keys":100}}}}}}"#
        ));
        let mut request = request(S3PolicyAction::ListBucket, None);
        request.query = S3PolicyQuery {
            prefix: Some("users/alice/photos/"),
            delimiter: Some("/"),
            max_keys: Some(100),
        };
        assert_eq!(policy.evaluate(&request), S3PolicyDecision::Allow);
        request.query.max_keys = Some(101);
        assert_eq!(policy.evaluate(&request), S3PolicyDecision::ImplicitDeny);
        request.query.max_keys = Some(1);
        request.query.prefix = Some("users/bob/");
        assert_eq!(policy.evaluate(&request), S3PolicyDecision::ImplicitDeny);
    }

    #[test]
    fn transport_ip_identity_and_version_conditions_are_supported() {
        let policy = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":"s3:GetObjectVersion","Resource":"arn:aws:s3:::{BUCKET}/*","Condition":{{"Bool":{{"aws:SecureTransport":true}},"IpAddress":{{"aws:SourceIp":["203.0.113.0/24","2001:db8::/32"]}},"StringEquals":{{"aws:PrincipalArn":"{ALICE_ARN}","aws:userid":"AIDAALICE","s3:VersionId":"v1"}}}}}}"#
        ));
        let mut request = request(S3PolicyAction::GetObjectVersion, Some("a"));
        request.version_id = Some("v1");
        assert_eq!(policy.evaluate(&request), S3PolicyDecision::Allow);
        request.secure_transport = false;
        assert_eq!(policy.evaluate(&request), S3PolicyDecision::ImplicitDeny);
        request.secure_transport = true;
        request.source_ip = Some("198.51.100.1".parse().expect("IP"));
        assert_eq!(policy.evaluate(&request), S3PolicyDecision::ImplicitDeny);
    }

    #[test]
    fn negative_and_null_conditions_observe_absence() {
        let negative_policy = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*","Condition":{{"StringNotEquals":{{"s3:VersionId":"blocked"}},"NotIpAddress":{{"aws:SourceIp":"10.0.0.0/8"}},"Null":{{"s3:delimiter":true}}}}}}"#
        ));
        let base_request = request(S3PolicyAction::GetObject, Some("a"));
        assert_eq!(
            negative_policy.evaluate(&base_request),
            S3PolicyDecision::Allow
        );
        let mut blocked = base_request;
        blocked.source_ip = Some("10.1.2.3".parse().expect("IP"));
        assert_eq!(
            negative_policy.evaluate(&blocked),
            S3PolicyDecision::ImplicitDeny
        );

        let present_policy = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":"s3:ListBucket","Resource":"arn:aws:s3:::{BUCKET}","Condition":{{"Null":{{"aws:SecureTransport":false,"aws:SourceIp":false,"s3:max-keys":false}}}}}}"#
        ));
        let mut present = request(S3PolicyAction::ListBucket, None);
        present.query.max_keys = Some(10);
        assert_eq!(present_policy.evaluate(&present), S3PolicyDecision::Allow);
        present.source_ip = None;
        assert_eq!(
            present_policy.evaluate(&present),
            S3PolicyDecision::ImplicitDeny
        );
    }

    #[test]
    fn parsed_policy_is_fenced_to_its_target_bucket_even_with_not_resource() {
        let policy = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","NotResource":"arn:aws:s3:::{BUCKET}/private/*"}}"#
        ));
        assert_eq!(policy.target_bucket(), BUCKET);
        let mut foreign = request(S3PolicyAction::GetObject, Some("public/a"));
        foreign.bucket = "other-bucket";
        assert_eq!(policy.evaluate(&foreign), S3PolicyDecision::ImplicitDeny);
    }

    #[test]
    fn s3_star_is_limited_to_supported_actions_and_resource_scope() {
        let policy = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":"s3:*","Resource":["arn:aws:s3:::{BUCKET}","arn:aws:s3:::{BUCKET}/*"]}}"#
        ));
        assert!(
            policy
                .evaluate(&request(S3PolicyAction::ListBucket, None))
                .is_allowed()
        );
        assert!(
            policy
                .evaluate(&request(S3PolicyAction::PutObject, Some("a")))
                .is_allowed()
        );
    }

    #[test]
    fn acl_policy_status_and_governance_bypass_actions_have_correct_scopes() {
        let object_policy = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":["s3:GetObjectAcl","s3:PutObjectAcl","s3:BypassGovernanceRetention"],"Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
        ));
        for action in [
            S3PolicyAction::GetObjectAcl,
            S3PolicyAction::PutObjectAcl,
            S3PolicyAction::BypassGovernanceRetention,
        ] {
            assert_eq!(
                object_policy.evaluate(&request(action, Some("locked/object"))),
                S3PolicyDecision::Allow
            );
            assert_eq!(S3PolicyAction::from_name(action.as_str()), Some(action));
            assert_eq!(action.scope(), S3PolicyResourceScope::Object);
        }

        let status_policy = policy(&statement(
            "Allow",
            "s3:GetBucketPolicyStatus",
            &format!("arn:aws:s3:::{BUCKET}"),
        ));
        assert_eq!(
            status_policy.evaluate(&request(S3PolicyAction::GetBucketPolicyStatus, None)),
            S3PolicyDecision::Allow
        );
        assert_eq!(
            S3PolicyAction::GetBucketPolicyStatus.scope(),
            S3PolicyResourceScope::Bucket
        );
    }

    #[test]
    fn malformed_unknown_conflicting_and_empty_documents_are_rejected() {
        let valid_statement =
            statement("Allow", "s3:GetObject", &format!("arn:aws:s3:::{BUCKET}/*"));
        let cases = [
            ("{", S3PolicyErrorCode::MalformedJson),
            (
                &format!(
                    r#"{{"Version":"2012-10-17","Version":"2012-10-17","Statement":{valid_statement}}}"#
                ),
                S3PolicyErrorCode::MalformedJson,
            ),
            (
                r#"{"Version":"2012-10-17","Statement":[]}"#,
                S3PolicyErrorCode::InvalidStatement,
            ),
            (
                &format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","NotAction":"s3:PutObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#
                ),
                S3PolicyErrorCode::InvalidStatement,
            ),
            (
                r#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":[]}}"#,
                S3PolicyErrorCode::InvalidResource,
            ),
        ];
        for (document, code) in cases {
            assert_invalid(document, code);
        }
    }

    #[test]
    fn invalid_principals_actions_resources_and_conditions_are_rejected() {
        let cases = [
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":{{"Service":"ec2.amazonaws.com"}},"Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#
                ),
                S3PolicyErrorCode::UnsupportedField,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":{{"AWS":"arn:aws:iam::123456789012:user/al*"}},"Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#
                ),
                S3PolicyErrorCode::InvalidPrincipal,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:SelectObjectContent","Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#
                ),
                S3PolicyErrorCode::UnsupportedAction,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:ListAllMyBuckets","Resource":"arn:aws:s3:::{BUCKET}"}}}}"#
                ),
                S3PolicyErrorCode::UnsupportedAction,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:CreateBucket","Resource":"arn:aws:s3:::{BUCKET}"}}}}"#
                ),
                S3PolicyErrorCode::UnsupportedAction,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"ec2:*","Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#
                ),
                S3PolicyErrorCode::UnsupportedAction,
            ),
            (
                r#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::other-bucket/*"}}"#.to_owned(),
                S3PolicyErrorCode::InvalidResource,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}"}}}}"#
                ),
                S3PolicyErrorCode::InvalidResource,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:ListBucket","Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#
                ),
                S3PolicyErrorCode::InvalidResource,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*","Condition":{{"DateEquals":{{"aws:CurrentTime":"now"}}}}}}}}"#
                ),
                S3PolicyErrorCode::UnsupportedCondition,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*","Condition":{{"Bool":{{"aws:SecureTransport":"yes"}}}}}}}}"#
                ),
                S3PolicyErrorCode::UnsupportedCondition,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*","Condition":{{"IpAddress":{{"aws:SourceIp":"203.0.113.1/24"}}}}}}}}"#
                ),
                S3PolicyErrorCode::UnsupportedCondition,
            ),
            (
                format!(
                    r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:ListBucket","Resource":"arn:aws:s3:::{BUCKET}","Condition":{{"NumericLessThanEquals":{{"s3:max-keys":-1}}}}}}}}"#
                ),
                S3PolicyErrorCode::UnsupportedCondition,
            ),
        ];
        for (document, code) in cases {
            assert_invalid(&document, code);
        }
    }

    #[test]
    fn duplicate_nested_keys_depth_size_and_runtime_shape_fail_closed() {
        let duplicate_condition = format!(
            r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*","Condition":{{"StringEquals":{{"s3:VersionId":"a","s3:VersionId":"b"}}}}}}}}"#
        );
        assert_invalid(&duplicate_condition, S3PolicyErrorCode::MalformedJson);

        let mut deep = String::from("[");
        deep.push_str(&"[".repeat(MAX_S3_POLICY_DEPTH + 1));
        deep.push_str("null");
        deep.push_str(&"]".repeat(MAX_S3_POLICY_DEPTH + 1));
        deep.push(']');
        assert_eq!(
            S3BucketPolicy::parse(deep.as_bytes(), BUCKET)
                .expect_err("deep policy")
                .code(),
            S3PolicyErrorCode::PolicyTooDeep
        );

        let oversized = vec![b' '; MAX_S3_POLICY_BYTES + 1];
        assert_eq!(
            S3BucketPolicy::parse(&oversized, BUCKET)
                .expect_err("large policy")
                .code(),
            S3PolicyErrorCode::PolicyTooLarge
        );

        let policy = policy(&statement(
            "Allow",
            "s3:GetObject",
            &format!("arn:aws:s3:::{BUCKET}/*"),
        ));
        assert_eq!(
            policy.evaluate(&request(S3PolicyAction::GetObject, None)),
            S3PolicyDecision::ImplicitDeny
        );
        let huge_key = "x".repeat(MAX_S3_POLICY_RUNTIME_KEY_BYTES + 1);
        assert_eq!(
            policy.evaluate(&request(S3PolicyAction::GetObject, Some(&huge_key))),
            S3PolicyDecision::ImplicitDeny
        );
    }

    #[test]
    fn collection_and_string_limits_are_enforced_before_evaluation() {
        let base = statement("Allow", "s3:GetObject", &format!("arn:aws:s3:::{BUCKET}/*"));
        let too_many_statements = format!(
            r#"{{"Version":"2012-10-17","Statement":[{}]}}"#,
            std::iter::repeat_n(base.as_str(), MAX_S3_POLICY_STATEMENTS + 1)
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_invalid(&too_many_statements, S3PolicyErrorCode::LimitExceeded);

        let repeated = |value: &str, count: usize| {
            std::iter::repeat_n(format!(r#""{value}""#), count)
                .collect::<Vec<_>>()
                .join(",")
        };
        let cases = [
            format!(
                r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":[{}],"Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#,
                repeated("s3:GetObject", MAX_S3_POLICY_ACTIONS + 1)
            ),
            format!(
                r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":[{}]}}}}"#,
                repeated(
                    &format!("arn:aws:s3:::{BUCKET}/*"),
                    MAX_S3_POLICY_RESOURCES + 1
                )
            ),
            format!(
                r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":{{"AWS":[{}]}},"Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#,
                repeated("123456789012", MAX_S3_POLICY_PRINCIPALS + 1)
            ),
            format!(
                r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*","Condition":{{"StringEquals":{{"s3:VersionId":[{}]}}}}}}}}"#,
                repeated("v1", MAX_S3_POLICY_CONDITION_VALUES + 1)
            ),
        ];
        for document in cases {
            assert_invalid(&document, S3PolicyErrorCode::LimitExceeded);
        }

        let long_sid = "x".repeat(MAX_S3_POLICY_STRING_BYTES + 1);
        let long_string = format!(
            r#"{{"Version":"2012-10-17","Statement":{{"Sid":"{long_sid}","Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#
        );
        assert_invalid(&long_string, S3PolicyErrorCode::LimitExceeded);
    }

    #[test]
    fn action_names_are_case_insensitive_but_resources_and_string_conditions_are_not() {
        let policy = policy(&format!(
            r#"{{"Effect":"Allow","Principal":"*","Action":"S3:gEtObJeCt","Resource":"arn:aws:s3:::{BUCKET}/Case/*","Condition":{{"StringEquals":{{"s3:VersionId":"ABC"}}}}}}"#
        ));
        let mut request = request(S3PolicyAction::GetObject, Some("Case/file"));
        request.version_id = Some("ABC");
        assert!(policy.evaluate(&request).is_allowed());
        request.key = Some("case/file");
        assert!(!policy.evaluate(&request).is_allowed());
        request.key = Some("Case/file");
        request.version_id = Some("abc");
        assert!(!policy.evaluate(&request).is_allowed());
    }

    #[test]
    fn stable_json_is_canonical_and_round_trips_without_semantic_drift() {
        let document = format!(
            r#"{{"Statement":{{"Condition":{{"IpAddress":{{"aws:SourceIp":["203.0.113.10","2001:db8::1"]}},"StringEquals":{{"s3:VersionId":["v2","v1"],"aws:PrincipalArn":"{ALICE_ARN}"}}}},"Resource":["arn:aws:s3:::{BUCKET}/b/*","arn:aws:s3:::{BUCKET}/a/*"],"Action":["s3:GetObjectVersion","s3:GetObject"],"Principal":{{"AWS":["arn:aws:iam::123456789012:role/editor","123456789012"]}},"Effect":"Allow","Sid":"Stable1"}},"Id":"stable-policy","Version":"2008-10-17"}}"#
        );
        let parsed = S3BucketPolicy::parse(document.as_bytes(), BUCKET).expect("stable policy");
        let stable = parsed.to_stable_json();
        assert_eq!(
            stable,
            format!(
                r#"{{"Version":"2008-10-17","Id":"stable-policy","Statement":[{{"Sid":"Stable1","Effect":"Allow","Principal":{{"AWS":["123456789012","arn:aws:iam::123456789012:role/editor"]}},"Action":["s3:GetObject","s3:GetObjectVersion"],"Resource":["arn:aws:s3:::{BUCKET}/a/*","arn:aws:s3:::{BUCKET}/b/*"],"Condition":{{"StringEquals":{{"aws:PrincipalArn":"{ALICE_ARN}","s3:VersionId":["v1","v2"]}},"IpAddress":{{"aws:SourceIp":["2001:db8::1/128","203.0.113.10/32"]}}}}}}]}}"#
            )
        );

        let reparsed = S3BucketPolicy::parse(stable.as_bytes(), BUCKET).expect("stable round trip");
        assert_eq!(reparsed.to_stable_json(), stable);
        let mut matching = request(S3PolicyAction::GetObjectVersion, Some("a/photo.jpg"));
        matching.version_id = Some("v1");
        assert_eq!(parsed.evaluate(&matching), reparsed.evaluate(&matching));
        assert_eq!(reparsed.evaluate(&matching), S3PolicyDecision::Allow);
    }

    #[test]
    fn policy_status_classifies_account_root_user_role_and_session_principals() {
        let cases = [
            ("any", r#""*""#, true),
            ("owner account", r#"{"AWS":"123456789012"}"#, false),
            (
                "owner root",
                r#"{"AWS":"arn:aws:iam::123456789012:root"}"#,
                false,
            ),
            (
                "owner user",
                r#"{"AWS":"arn:aws:iam::123456789012:user/alice"}"#,
                false,
            ),
            (
                "owner role",
                r#"{"AWS":"arn:aws:iam::123456789012:role/media/reader"}"#,
                false,
            ),
            (
                "owner assumed role",
                r#"{"AWS":"arn:aws:sts::123456789012:assumed-role/media/session"}"#,
                false,
            ),
            ("external account", r#"{"AWS":"210987654321"}"#, true),
            (
                "external root",
                r#"{"AWS":"arn:aws:iam::210987654321:root"}"#,
                true,
            ),
            (
                "mixed accounts",
                r#"{"AWS":["123456789012","arn:aws:iam::210987654321:user/bob"]}"#,
                true,
            ),
        ];

        for (name, principal, expected_public) in cases {
            let policy = policy(&format!(
                r#"{{"Effect":"Allow","Principal":{principal},"Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
            ));
            let status = policy.policy_status("123456789012");
            assert_eq!(status.is_public(), expected_public, "{name}");
            assert_eq!(status.is_trusted(), !expected_public, "{name}");
        }
    }

    #[test]
    fn policy_status_handles_not_principal_and_deny_conservatively() {
        let cases = [
            (
                "not everybody is unreachable",
                r#"{"Effect":"Allow","NotPrincipal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos-prod/*"}"#,
                false,
            ),
            (
                "not owner leaves an open complement",
                r#"{"Effect":"Allow","NotPrincipal":{"AWS":"123456789012"},"Action":"s3:GetObject","Resource":"arn:aws:s3:::photos-prod/*"}"#,
                true,
            ),
            (
                "deny only",
                r#"{"Effect":"Deny","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos-prod/*"}"#,
                false,
            ),
            (
                "deny does not prove a broad allow safe",
                r#"[{"Effect":"Deny","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos-prod/private/*"},{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::photos-prod/*"}]"#,
                true,
            ),
        ];

        for (name, statements, expected_public) in cases {
            assert_eq!(
                policy(statements).policy_status("123456789012").is_public(),
                expected_public,
                "{name}"
            );
        }

        let owner_only = policy(&format!(
            r#"{{"Effect":"Allow","Principal":{{"AWS":"123456789012"}},"Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
        ));
        assert!(owner_only.policy_status("invalid").is_public());
        let deny_only = policy(&statement(
            "Deny",
            "s3:GetObject",
            &format!("arn:aws:s3:::{BUCKET}/*"),
        ));
        assert!(!deny_only.policy_status("invalid").is_public());
    }

    #[test]
    fn policy_status_only_accepts_provable_principal_or_source_ip_narrowing() {
        let cases = [
            (
                "owner principal arn equality",
                format!(r#""StringEquals":{{"aws:PrincipalArn":"{ALICE_ARN}"}}"#),
                false,
            ),
            (
                "owner role exact string-like",
                r#""StringLike":{"aws:PrincipalArn":"arn:aws:iam::123456789012:role/media/reader"}"#.to_owned(),
                false,
            ),
            (
                "principal arn wildcard",
                r#""StringLike":{"aws:PrincipalArn":"arn:aws:iam::123456789012:role/*"}"#.to_owned(),
                true,
            ),
            (
                "external principal arn",
                r#""StringEquals":{"aws:PrincipalArn":"arn:aws:iam::210987654321:user/bob"}"#.to_owned(),
                true,
            ),
            (
                "exact ipv4 host",
                r#""IpAddress":{"aws:SourceIp":"203.0.113.10"}"#.to_owned(),
                false,
            ),
            (
                "exact ipv6 host",
                r#""IpAddress":{"aws:SourceIp":"2001:db8::1/128"}"#.to_owned(),
                false,
            ),
            (
                "ipv4 network remains public",
                r#""IpAddress":{"aws:SourceIp":"203.0.113.0/24"}"#.to_owned(),
                true,
            ),
            (
                "negative ip condition remains public",
                r#""NotIpAddress":{"aws:SourceIp":"203.0.113.10"}"#.to_owned(),
                true,
            ),
            (
                "transport alone remains public",
                r#""Bool":{"aws:SecureTransport":true}"#.to_owned(),
                true,
            ),
        ];

        for (name, condition, expected_public) in cases {
            let policy = policy(&format!(
                r#"{{"Effect":"Allow","Principal":"*","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*","Condition":{{{condition}}}}}"#
            ));
            assert_eq!(
                policy.policy_status("123456789012").is_public(),
                expected_public,
                "{name}"
            );
        }
    }

    #[test]
    fn policy_status_applies_to_every_supported_action_and_resource_surface() {
        let cases = [
            (
                "bucket action",
                statement("Allow", "s3:ListBucket", &format!("arn:aws:s3:::{BUCKET}")),
                true,
            ),
            (
                "single object",
                statement(
                    "Allow",
                    "s3:GetObject",
                    &format!("arn:aws:s3:::{BUCKET}/public/readme.txt"),
                ),
                true,
            ),
            (
                "object not action",
                format!(
                    r#"{{"Effect":"Allow","Principal":"*","NotAction":"s3:DeleteObject*","Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
                ),
                true,
            ),
            (
                "owner all supported actions",
                format!(
                    r#"{{"Effect":"Allow","Principal":{{"AWS":"123456789012"}},"Action":"s3:*","Resource":["arn:aws:s3:::{BUCKET}","arn:aws:s3:::{BUCKET}/*"]}}"#
                ),
                false,
            ),
        ];

        for (name, statement, expected_public) in cases {
            assert_eq!(
                policy(&statement).policy_status("123456789012").is_public(),
                expected_public,
                "{name}"
            );
        }
    }

    fn identity_policy(statements: &str) -> S3IdentityPolicy {
        S3IdentityPolicy::parse(
            format!(r#"{{"Version":"2012-10-17","Statement":{statements}}}"#).as_bytes(),
        )
        .expect("valid identity policy")
    }

    #[test]
    fn identity_policy_models_account_actions_without_a_bucket_sentinel() {
        let policy = identity_policy(
            r#"[
                {"Effect":"Allow","Action":"s3:ListAllMyBuckets","Resource":"*"},
                {"Effect":"Allow","Action":"s3:CreateBucket","Resource":"*"}
            ]"#,
        );

        for action in [
            S3PolicyAction::ListAllMyBuckets,
            S3PolicyAction::CreateBucket,
        ] {
            assert_eq!(action.scope(), S3PolicyResourceScope::Account);
            assert_eq!(
                policy.evaluate(&S3IdentityPolicyRequest::account(action)),
                S3PolicyDecision::Allow,
                "{}",
                action.as_str()
            );
            assert_eq!(
                policy.evaluate(&S3IdentityPolicyRequest::bucket(action, BUCKET)),
                S3PolicyDecision::ImplicitDeny,
                "scope mismatch must fail closed"
            );
        }

        let invalid = format!(
            r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Action":"s3:CreateBucket","Resource":"arn:aws:s3:::{BUCKET}"}}}}"#
        );
        assert_eq!(
            S3IdentityPolicy::parse(invalid.as_bytes())
                .expect_err("account action requires an account resource")
                .code(),
            S3PolicyErrorCode::InvalidResource
        );
    }

    #[test]
    fn identity_policy_cross_bucket_and_bounded_resource_wildcards_match() {
        let policy = identity_policy(
            r#"{
                "Effect":"Allow",
                "Action":"s3:GetObject",
                "Resource":[
                    "arn:aws:s3:::archive-prod/exact.txt",
                    "arn:aws:s3:::team-?/public/*"
                ]
            }"#,
        );
        let cases = [
            ("exact foreign bucket", "archive-prod", "exact.txt", true),
            (
                "single-character bucket wildcard",
                "team-a",
                "public/a.jpg",
                true,
            ),
            (
                "wildcard does not span a missing key prefix",
                "team-a",
                "private/a.jpg",
                false,
            ),
            (
                "question mark is bounded to one character",
                "team-ab",
                "public/a.jpg",
                false,
            ),
        ];
        for (name, bucket, key, allowed) in cases {
            let decision = policy.evaluate(&S3IdentityPolicyRequest::object(
                S3PolicyAction::GetObject,
                bucket,
                key,
            ));
            assert_eq!(decision.is_allowed(), allowed, "{name}");
        }

        let wildcard = identity_policy(r#"{"Effect":"Allow","Action":"s3:*","Resource":"*"}"#);
        let wildcard_cases = [
            S3IdentityPolicyRequest::account(S3PolicyAction::ListAllMyBuckets),
            S3IdentityPolicyRequest::bucket(S3PolicyAction::ListBucket, BUCKET),
            S3IdentityPolicyRequest::object(S3PolicyAction::PutObject, BUCKET, "a.jpg"),
        ];
        for request in wildcard_cases {
            assert_eq!(wildcard.evaluate(&request), S3PolicyDecision::Allow);
        }
    }

    #[test]
    fn identity_not_action_not_resource_and_deny_precedence_are_correct() {
        let policy = identity_policy(&format!(
            r#"[
                {{"Effect":"Allow","NotAction":"s3:DeleteObject*","NotResource":"arn:aws:s3:::{BUCKET}/private/*"}},
                {{"Effect":"Deny","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/blocked/*"}}
            ]"#
        ));
        let cases = [
            (
                "public read",
                S3PolicyAction::GetObject,
                "public/a",
                S3PolicyDecision::Allow,
            ),
            (
                "not resource exclusion",
                S3PolicyAction::GetObject,
                "private/a",
                S3PolicyDecision::ImplicitDeny,
            ),
            (
                "not action exclusion",
                S3PolicyAction::DeleteObject,
                "public/a",
                S3PolicyDecision::ImplicitDeny,
            ),
            (
                "explicit deny wins",
                S3PolicyAction::GetObject,
                "blocked/a",
                S3PolicyDecision::ExplicitDeny,
            ),
        ];
        for (name, action, key, expected) in cases {
            assert_eq!(
                policy.evaluate(&S3IdentityPolicyRequest::object(action, BUCKET, key)),
                expected,
                "{name}"
            );
        }
    }

    #[test]
    fn identity_policy_forbids_principals_and_bucket_policy_forbids_identity_only_actions() {
        for principal_field in [r#""Principal":"*","#, r#""NotPrincipal":"*","#] {
            let document = format!(
                r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow",{principal_field}"Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}}}"#
            );
            assert_eq!(
                S3IdentityPolicy::parse(document.as_bytes())
                    .expect_err("identity statements cannot contain principals")
                    .code(),
                S3PolicyErrorCode::UnsupportedField
            );
        }

        for action in ["s3:ListAllMyBuckets", "s3:CreateBucket"] {
            let document = format!(
                r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Principal":"*","Action":"{action}","Resource":"arn:aws:s3:::{BUCKET}"}}}}"#
            );
            assert_eq!(
                S3BucketPolicy::parse(document.as_bytes(), BUCKET)
                    .expect_err("identity-only action cannot be installed on a bucket")
                    .code(),
                S3PolicyErrorCode::UnsupportedAction,
                "{action}"
            );
        }
    }

    #[test]
    fn identity_policy_reuses_document_limits_and_stable_roundtrip() {
        let document = r#"{"Version":"2008-10-17","Id":"identity-main","Statement":{"Sid":"ReadPublic","Effect":"Allow","Action":["s3:GetObjectVersion","s3:GetObject"],"Resource":["arn:aws:s3:::team-*/*","arn:aws:s3:::archive-prod/exact.txt"],"Condition":{"Bool":{"aws:SecureTransport":true},"StringLike":{"aws:PrincipalArn":"arn:aws:iam::123456789012:user/*"}}}}"#;
        let parsed = S3IdentityPolicy::parse(document.as_bytes()).expect("identity policy");
        assert_eq!(parsed.version(), S3_POLICY_LEGACY_VERSION);
        assert_eq!(parsed.id(), Some("identity-main"));
        assert_eq!(parsed.statements()[0].sid(), Some("ReadPublic"));
        let stable = parsed.to_stable_json();
        assert!(!stable.contains("\"Principal\":"));
        assert!(!stable.contains("\"NotPrincipal\":"));
        let reparsed = S3IdentityPolicy::parse(stable.as_bytes()).expect("stable roundtrip");
        assert_eq!(reparsed.to_stable_json(), stable);

        let mut request =
            S3IdentityPolicyRequest::object(S3PolicyAction::GetObject, "team-blue", "photo.jpg");
        request.secure_transport = true;
        request.principal_arn = Some(ALICE_ARN);
        request.principal_account = Some("123456789012");
        assert_eq!(parsed.evaluate(&request), S3PolicyDecision::Allow);
        assert_eq!(reparsed.evaluate(&request), S3PolicyDecision::Allow);

        let oversized = vec![b' '; MAX_S3_POLICY_BYTES + 1];
        assert_eq!(
            S3IdentityPolicy::parse(&oversized)
                .expect_err("size limit")
                .code(),
            S3PolicyErrorCode::PolicyTooLarge
        );

        let repeated = (0..=MAX_S3_POLICY_STATEMENTS)
            .map(|index| {
                format!(
                    r#"{{"Sid":"S{index}","Effect":"Allow","Action":"s3:GetObject","Resource":"arn:aws:s3:::{BUCKET}/*"}}"#
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let too_many = format!(r#"{{"Version":"2012-10-17","Statement":[{repeated}]}}"#);
        assert_eq!(
            S3IdentityPolicy::parse(too_many.as_bytes())
                .expect_err("statement count limit")
                .code(),
            S3PolicyErrorCode::LimitExceeded
        );
    }
}
