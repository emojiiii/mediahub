use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::{ApplicationId, BucketId, DomainError, DomainResult, Visibility};

pub const MAX_LIFECYCLE_RULES: usize = 32;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleRule {
    ExpireAfter {
        id: String,
        #[serde(default = "default_enabled")]
        enabled: bool,
        prefix: String,
        duration_seconds: u64,
    },
    KeepLatest {
        id: String,
        #[serde(default = "default_enabled")]
        enabled: bool,
        prefix: String,
        count: u32,
    },
}

impl LifecycleRule {
    #[must_use]
    pub fn id(&self) -> &str {
        match self {
            Self::ExpireAfter { id, .. } | Self::KeepLatest { id, .. } => id,
        }
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        match self {
            Self::ExpireAfter { enabled, .. } | Self::KeepLatest { enabled, .. } => *enabled,
        }
    }

    #[must_use]
    pub fn prefix(&self) -> &str {
        match self {
            Self::ExpireAfter { prefix, .. } | Self::KeepLatest { prefix, .. } => prefix,
        }
    }

    fn validate(&self) -> DomainResult<()> {
        let valid_common = !self.id().is_empty()
            && self.id().len() <= 128
            && !self.id().bytes().any(|byte| byte.is_ascii_control())
            && self.prefix().len() <= 1024
            && !self.prefix().bytes().any(|byte| byte.is_ascii_control());
        let valid_value = match self {
            Self::ExpireAfter {
                duration_seconds, ..
            } => *duration_seconds > 0 && i64::try_from(*duration_seconds).is_ok(),
            Self::KeepLatest { count, .. } => *count > 0,
        };
        if valid_common && valid_value {
            Ok(())
        } else {
            Err(DomainError::InvalidLifecycleRule)
        }
    }
}

const fn default_enabled() -> bool {
    true
}

/// Upload and access defaults associated with a bucket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketPolicy {
    visibility: Visibility,
    default_ttl_seconds: Option<u64>,
    max_object_size: Option<u64>,
    allowed_mime_types: BTreeSet<String>,
    #[serde(default)]
    lifecycle_rules: Vec<LifecycleRule>,
}

impl BucketPolicy {
    pub fn new(
        visibility: Visibility,
        default_ttl_seconds: Option<u64>,
        max_object_size: Option<u64>,
        allowed_mime_types: impl IntoIterator<Item = String>,
    ) -> DomainResult<Self> {
        if default_ttl_seconds == Some(0) || max_object_size == Some(0) {
            return Err(DomainError::InvalidBucketPolicy);
        }
        let mut mime_types = BTreeSet::new();
        for mime_type in allowed_mime_types {
            mime_types.insert(canonical_mime_type(&mime_type)?);
        }

        Ok(Self {
            visibility,
            default_ttl_seconds,
            max_object_size,
            allowed_mime_types: mime_types,
            lifecycle_rules: Vec::new(),
        })
    }

    #[must_use]
    pub fn unrestricted(visibility: Visibility) -> Self {
        Self {
            visibility,
            default_ttl_seconds: None,
            max_object_size: None,
            allowed_mime_types: BTreeSet::new(),
            lifecycle_rules: Vec::new(),
        }
    }

    pub fn with_lifecycle_rules(
        mut self,
        lifecycle_rules: Vec<LifecycleRule>,
    ) -> DomainResult<Self> {
        if lifecycle_rules.len() > MAX_LIFECYCLE_RULES {
            return Err(DomainError::InvalidLifecycleRule);
        }
        let mut ids = BTreeSet::new();
        for rule in &lifecycle_rules {
            rule.validate()?;
            if !ids.insert(rule.id()) {
                return Err(DomainError::InvalidLifecycleRule);
            }
        }
        self.lifecycle_rules = lifecycle_rules;
        Ok(self)
    }

    #[must_use]
    pub const fn visibility(&self) -> Visibility {
        self.visibility
    }

    #[must_use]
    pub const fn default_ttl_seconds(&self) -> Option<u64> {
        self.default_ttl_seconds
    }

    #[must_use]
    pub const fn max_object_size(&self) -> Option<u64> {
        self.max_object_size
    }

    pub fn allowed_mime_types(&self) -> impl Iterator<Item = &str> {
        self.allowed_mime_types.iter().map(String::as_str)
    }

    #[must_use]
    pub fn lifecycle_rules(&self) -> &[LifecycleRule] {
        &self.lifecycle_rules
    }

    /// Validates a completed upload against the bucket's policy.
    pub fn validate_upload(&self, mime: &str, size: u64) -> DomainResult<()> {
        let mime = canonical_mime_type(mime)?;

        if let Some(maximum) = self.max_object_size
            && size > maximum
        {
            return Err(DomainError::ObjectTooLarge {
                actual: size,
                maximum,
            });
        }

        if !self.allowed_mime_types.is_empty() && !self.allowed_mime_types.contains(&mime) {
            return Err(DomainError::MimeTypeNotAllowed { mime });
        }

        Ok(())
    }
}

/// A logical namespace whose name is immutable after creation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bucket {
    id: BucketId,
    application_id: ApplicationId,
    name: String,
    policy: BucketPolicy,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl Bucket {
    pub fn new(
        id: BucketId,
        application_id: ApplicationId,
        name: impl Into<String>,
        policy: BucketPolicy,
        now: OffsetDateTime,
    ) -> DomainResult<Self> {
        let name = name.into();
        validate_bucket_name(&name)?;

        Ok(Self {
            id,
            application_id,
            name,
            policy,
            created_at: now,
            updated_at: now,
        })
    }

    #[must_use]
    pub const fn id(&self) -> BucketId {
        self.id
    }

    #[must_use]
    pub const fn application_id(&self) -> ApplicationId {
        self.application_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn policy(&self) -> &BucketPolicy {
        &self.policy
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    #[must_use]
    pub const fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }

    /// Changes mutable policy fields while deliberately leaving the bucket name untouched.
    pub fn update_policy(&mut self, policy: BucketPolicy, now: OffsetDateTime) {
        self.policy = policy;
        self.updated_at = now;
    }

    pub fn validate_upload(&self, mime: &str, size: u64) -> DomainResult<()> {
        self.policy.validate_upload(mime, size)
    }
}

pub(crate) fn canonical_mime_type(value: &str) -> DomainResult<String> {
    let value = value.trim();
    let valid = value
        .split_once('/')
        .is_some_and(|(type_part, subtype_part)| {
            !type_part.is_empty()
                && !subtype_part.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii() && !byte.is_ascii_control() && byte != b' ')
        });

    if !valid {
        return Err(DomainError::InvalidMimeType {
            value: value.to_owned(),
        });
    }

    Ok(value.to_ascii_lowercase())
}

fn validate_bucket_name(name: &str) -> DomainResult<()> {
    if name.is_empty() || name.len() > 255 || name.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(DomainError::InvalidBucketName);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_policy_enforces_size_and_mime_limits() {
        let policy = BucketPolicy::new(
            Visibility::Private,
            Some(3600),
            Some(10),
            ["image/png".to_owned()],
        )
        .expect("valid policy");

        assert!(policy.validate_upload("IMAGE/PNG", 10).is_ok());
        assert_eq!(
            policy.validate_upload("image/jpeg", 10),
            Err(DomainError::MimeTypeNotAllowed {
                mime: "image/jpeg".to_owned()
            })
        );
        assert_eq!(
            BucketPolicy::new(Visibility::Private, Some(0), None, Vec::new()),
            Err(DomainError::InvalidBucketPolicy)
        );
        assert_eq!(
            policy.validate_upload("image/png", 11),
            Err(DomainError::ObjectTooLarge {
                actual: 11,
                maximum: 10
            })
        );
    }

    #[test]
    fn bucket_name_is_validated_and_not_mutable() {
        let policy = BucketPolicy::unrestricted(Visibility::Public);
        let bucket = Bucket::new(
            BucketId::new(),
            ApplicationId::new(),
            "assets",
            policy,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("valid bucket");

        assert_eq!(bucket.name(), "assets");
        assert!(
            Bucket::new(
                BucketId::new(),
                ApplicationId::new(),
                "",
                BucketPolicy::unrestricted(Visibility::Private),
                OffsetDateTime::UNIX_EPOCH,
            )
            .is_err()
        );
    }

    #[test]
    fn lifecycle_rules_validate_values_and_unique_ids() {
        let policy = BucketPolicy::unrestricted(Visibility::Private)
            .with_lifecycle_rules(vec![
                LifecycleRule::ExpireAfter {
                    id: "expire-temp".into(),
                    enabled: true,
                    prefix: "temp/".into(),
                    duration_seconds: 3600,
                },
                LifecycleRule::KeepLatest {
                    id: "keep-outputs".into(),
                    enabled: true,
                    prefix: "outputs/".into(),
                    count: 10,
                },
            ])
            .expect("rules");
        assert_eq!(policy.lifecycle_rules().len(), 2);
        assert!(
            BucketPolicy::unrestricted(Visibility::Private)
                .with_lifecycle_rules(vec![
                    LifecycleRule::KeepLatest {
                        id: "same".into(),
                        enabled: true,
                        prefix: String::new(),
                        count: 1,
                    },
                    LifecycleRule::KeepLatest {
                        id: "same".into(),
                        enabled: true,
                        prefix: String::new(),
                        count: 2,
                    },
                ])
                .is_err()
        );
    }
}
