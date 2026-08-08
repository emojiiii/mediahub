use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{Duration, OffsetDateTime};

use crate::{ApplicationId, BucketId, ObjectRetention, S3LifecycleConfiguration};

const MAX_REGION_BYTES: usize = 63;

/// S3 bucket versioning state. `Unversioned` is an initial state, not a state
/// that can be restored after versioning has been enabled once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VersioningStatus {
    #[default]
    Unversioned,
    Enabled,
    Suspended,
}

impl VersioningStatus {
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Unversioned, Self::Unversioned | Self::Enabled)
                | (Self::Enabled, Self::Enabled | Self::Suspended)
                | (Self::Suspended, Self::Enabled | Self::Suspended)
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionMode {
    Governance,
    Compliance,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultRetentionPeriod {
    Days(u32),
    Years(u32),
}

impl DefaultRetentionPeriod {
    const fn is_valid(self) -> bool {
        match self {
            Self::Days(value) | Self::Years(value) => value > 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultRetention {
    mode: RetentionMode,
    period: DefaultRetentionPeriod,
}

impl DefaultRetention {
    pub fn new(mode: RetentionMode, period: DefaultRetentionPeriod) -> Result<Self, S3ModelError> {
        if !period.is_valid() {
            return Err(S3ModelError::InvalidDefaultRetention);
        }
        Ok(Self { mode, period })
    }

    #[must_use]
    pub const fn mode(self) -> RetentionMode {
        self.mode
    }

    #[must_use]
    pub const fn period(self) -> DefaultRetentionPeriod {
        self.period
    }

    /// Resolves a bucket default into the absolute timestamp frozen on a new
    /// object version. Year periods use calendar years (including leap-day
    /// clamping) instead of treating a year as a fixed number of days.
    pub fn for_object_at(
        self,
        created_at: OffsetDateTime,
    ) -> Result<ObjectRetention, S3ModelError> {
        let retain_until = match self.period {
            DefaultRetentionPeriod::Days(days) => created_at
                .checked_add(Duration::days(i64::from(days)))
                .ok_or(S3ModelError::InvalidObjectRetention)?,
            DefaultRetentionPeriod::Years(years) => {
                let years =
                    i32::try_from(years).map_err(|_| S3ModelError::InvalidObjectRetention)?;
                let year = created_at
                    .year()
                    .checked_add(years)
                    .ok_or(S3ModelError::InvalidObjectRetention)?;
                let date = created_at
                    .date()
                    .replace_year(year)
                    .or_else(|_| {
                        created_at
                            .date()
                            .replace_day(28)
                            .and_then(|date| date.replace_year(year))
                    })
                    .map_err(|_| S3ModelError::InvalidObjectRetention)?;
                created_at.replace_date(date)
            }
        };
        Ok(ObjectRetention::new(self.mode, retain_until))
    }
}

/// Bucket-scoped S3 configuration. Lifecycle XML parsing belongs above this
/// type; the domain stores the normalized JSON document as an opaque boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BucketS3Configuration {
    region: String,
    versioning_status: VersioningStatus,
    object_lock_enabled: bool,
    default_retention: Option<DefaultRetention>,
    lifecycle_configuration: Option<S3LifecycleConfiguration>,
    revision: u64,
    updated_at: OffsetDateTime,
}

impl BucketS3Configuration {
    pub fn new(
        region: impl Into<String>,
        object_lock_enabled: bool,
        default_retention: Option<DefaultRetention>,
        lifecycle_configuration: Option<S3LifecycleConfiguration>,
        now: OffsetDateTime,
    ) -> Result<Self, S3ModelError> {
        let versioning_status = if object_lock_enabled {
            VersioningStatus::Enabled
        } else {
            VersioningStatus::Unversioned
        };
        Self::from_persistence(PersistedBucketS3Configuration {
            region: region.into(),
            versioning_status,
            object_lock_enabled,
            default_retention,
            lifecycle_configuration,
            revision: 1,
            updated_at: now,
        })
    }

    pub fn from_persistence(
        persisted: PersistedBucketS3Configuration,
    ) -> Result<Self, S3ModelError> {
        validate_region(&persisted.region)?;
        if persisted.revision == 0 {
            return Err(S3ModelError::InvalidConfigurationRevision);
        }
        if let Some(configuration) = &persisted.lifecycle_configuration {
            configuration.validate()?;
        }
        if persisted.default_retention.is_some() && !persisted.object_lock_enabled {
            return Err(S3ModelError::RetentionRequiresObjectLock);
        }
        if persisted.object_lock_enabled && persisted.versioning_status != VersioningStatus::Enabled
        {
            return Err(S3ModelError::ObjectLockRequiresEnabledVersioning);
        }

        Ok(Self {
            region: persisted.region,
            versioning_status: persisted.versioning_status,
            object_lock_enabled: persisted.object_lock_enabled,
            default_retention: persisted.default_retention,
            lifecycle_configuration: persisted.lifecycle_configuration,
            revision: persisted.revision,
            updated_at: persisted.updated_at,
        })
    }

    pub fn transition_versioning(
        &mut self,
        next: VersioningStatus,
        now: OffsetDateTime,
    ) -> Result<bool, S3ModelError> {
        if self.object_lock_enabled && next != VersioningStatus::Enabled {
            return Err(S3ModelError::ObjectLockRequiresEnabledVersioning);
        }
        if !self.versioning_status.can_transition_to(next) {
            return Err(S3ModelError::InvalidVersioningTransition {
                from: self.versioning_status,
                to: next,
            });
        }
        if self.versioning_status == next {
            return Ok(false);
        }
        self.versioning_status = next;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(S3ModelError::InvalidConfigurationRevision)?;
        self.updated_at = now;
        Ok(true)
    }

    pub fn replace_lifecycle_configuration(
        &mut self,
        lifecycle_configuration: Option<S3LifecycleConfiguration>,
        now: OffsetDateTime,
    ) -> Result<bool, S3ModelError> {
        if let Some(configuration) = &lifecycle_configuration {
            configuration.validate()?;
        }
        if self.lifecycle_configuration == lifecycle_configuration {
            return Ok(false);
        }
        self.lifecycle_configuration = lifecycle_configuration;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(S3ModelError::InvalidConfigurationRevision)?;
        self.updated_at = now;
        Ok(true)
    }

    /// Enables Object Lock and replaces its optional default retention rule.
    ///
    /// Object Lock is irreversible. First enablement also enables versioning;
    /// subsequent calls may only replace (or clear) the default retention
    /// rule while keeping both Object Lock and versioning enabled.
    pub fn replace_object_lock_configuration(
        &mut self,
        default_retention: Option<DefaultRetention>,
        now: OffsetDateTime,
    ) -> Result<bool, S3ModelError> {
        if self.object_lock_enabled
            && self.versioning_status == VersioningStatus::Enabled
            && self.default_retention == default_retention
        {
            return Ok(false);
        }
        self.object_lock_enabled = true;
        self.versioning_status = VersioningStatus::Enabled;
        self.default_retention = default_retention;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(S3ModelError::InvalidConfigurationRevision)?;
        self.updated_at = now;
        Ok(true)
    }

    #[must_use]
    pub fn region(&self) -> &str {
        &self.region
    }

    #[must_use]
    pub const fn versioning_status(&self) -> VersioningStatus {
        self.versioning_status
    }

    #[must_use]
    pub const fn object_lock_enabled(&self) -> bool {
        self.object_lock_enabled
    }

    #[must_use]
    pub const fn default_retention(&self) -> Option<DefaultRetention> {
        self.default_retention
    }

    #[must_use]
    pub const fn lifecycle_configuration(&self) -> Option<&S3LifecycleConfiguration> {
        self.lifecycle_configuration.as_ref()
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistedBucketS3Configuration {
    pub region: String,
    pub versioning_status: VersioningStatus,
    pub object_lock_enabled: bool,
    pub default_retention: Option<DefaultRetention>,
    pub lifecycle_configuration: Option<S3LifecycleConfiguration>,
    pub revision: u64,
    pub updated_at: OffsetDateTime,
}

/// S3-facing bucket aggregate. Existing media policy fields deliberately do
/// not leak into this model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3Bucket {
    id: BucketId,
    application_id: ApplicationId,
    name: String,
    configuration: BucketS3Configuration,
    created_at: OffsetDateTime,
}

impl S3Bucket {
    pub fn new(
        id: BucketId,
        application_id: ApplicationId,
        name: impl Into<String>,
        region: impl Into<String>,
        object_lock_enabled: bool,
        default_retention: Option<DefaultRetention>,
        now: OffsetDateTime,
    ) -> Result<Self, S3ModelError> {
        let name = name.into();
        validate_s3_bucket_name(&name)?;
        Ok(Self {
            id,
            application_id,
            name,
            configuration: BucketS3Configuration::new(
                region,
                object_lock_enabled,
                default_retention,
                None,
                now,
            )?,
            created_at: now,
        })
    }

    pub fn from_persistence(persisted: PersistedS3Bucket) -> Result<Self, S3ModelError> {
        validate_s3_bucket_name(&persisted.name)?;
        Ok(Self {
            id: persisted.id,
            application_id: persisted.application_id,
            name: persisted.name,
            configuration: BucketS3Configuration::from_persistence(persisted.configuration)?,
            created_at: persisted.created_at,
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
    pub const fn configuration(&self) -> &BucketS3Configuration {
        &self.configuration
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistedS3Bucket {
    pub id: BucketId,
    pub application_id: ApplicationId,
    pub name: String,
    pub configuration: PersistedBucketS3Configuration,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum S3ModelError {
    #[error("S3 bucket name is invalid")]
    InvalidBucketName,
    #[error("S3 region is invalid")]
    InvalidRegion,
    #[error("default retention period must be positive")]
    InvalidDefaultRetention,
    #[error("default retention requires Object Lock")]
    RetentionRequiresObjectLock,
    #[error("lifecycle configuration must be a JSON object")]
    InvalidLifecycleConfiguration,
    #[error("unsupported S3 lifecycle action: {0}")]
    UnsupportedLifecycleAction(String),
    #[error("entity tag is invalid")]
    InvalidEntityTag,
    #[error("checksum is invalid or unsupported")]
    InvalidChecksum,
    #[error("upload intent is invalid")]
    InvalidUploadIntent,
    #[error("storage locator is invalid")]
    InvalidStorageLocator,
    #[error("storage garbage-collection task is invalid")]
    InvalidStorageGcTask,
    #[error("S3 configuration revision must be positive")]
    InvalidConfigurationRevision,
    #[error("Object Lock requires versioning to remain enabled")]
    ObjectLockRequiresEnabledVersioning,
    #[error("versioning cannot transition from {from:?} to {to:?}")]
    InvalidVersioningTransition {
        from: VersioningStatus,
        to: VersioningStatus,
    },
    #[error("object key is invalid")]
    InvalidObjectKey,
    #[error("object version ID is invalid")]
    InvalidVersionId,
    #[error("object version generation is invalid")]
    InvalidObjectGeneration,
    #[error("object version payload is invalid")]
    InvalidObjectVersionPayload,
    #[error("object retention is invalid")]
    InvalidObjectRetention,
    #[error("object version does not belong to the logical object")]
    ObjectVersionOwnershipMismatch,
    #[error("object version creator is invalid")]
    InvalidCreatedBy,
}

fn validate_region(region: &str) -> Result<(), S3ModelError> {
    let valid = !region.is_empty()
        && region.len() <= MAX_REGION_BYTES
        && region
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && !region.starts_with('-')
        && !region.ends_with('-');
    if valid {
        Ok(())
    } else {
        Err(S3ModelError::InvalidRegion)
    }
}

fn validate_s3_bucket_name(name: &str) -> Result<(), S3ModelError> {
    let valid = (3..=63).contains(&name.len())
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'.')
        })
        && name
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && name
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && !name.contains("..")
        && !name.contains(".-")
        && !name.contains("-.");
    if valid {
        Ok(())
    } else {
        Err(S3ModelError::InvalidBucketName)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration(object_lock_enabled: bool) -> BucketS3Configuration {
        BucketS3Configuration::new(
            "us-east-1",
            object_lock_enabled,
            None,
            None,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("valid configuration")
    }

    #[test]
    fn versioning_state_machine_never_returns_to_unversioned() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut configuration = configuration(false);
        assert_eq!(
            configuration.versioning_status(),
            VersioningStatus::Unversioned
        );
        assert!(
            configuration
                .transition_versioning(VersioningStatus::Enabled, now)
                .expect("enable versioning")
        );
        assert!(
            configuration
                .transition_versioning(VersioningStatus::Suspended, now)
                .expect("suspend versioning")
        );
        assert!(
            configuration
                .transition_versioning(VersioningStatus::Enabled, now)
                .expect("resume versioning")
        );
        assert!(matches!(
            configuration.transition_versioning(VersioningStatus::Unversioned, now),
            Err(S3ModelError::InvalidVersioningTransition { .. })
        ));
    }

    #[test]
    fn unversioned_bucket_cannot_jump_to_suspended() {
        let mut configuration = configuration(false);
        assert!(matches!(
            configuration
                .transition_versioning(VersioningStatus::Suspended, OffsetDateTime::UNIX_EPOCH),
            Err(S3ModelError::InvalidVersioningTransition { .. })
        ));
    }

    #[test]
    fn object_lock_is_created_enabled_and_cannot_be_suspended() {
        let mut configuration = configuration(true);
        assert!(configuration.object_lock_enabled());
        assert_eq!(configuration.versioning_status(), VersioningStatus::Enabled);
        assert_eq!(
            configuration
                .transition_versioning(VersioningStatus::Suspended, OffsetDateTime::UNIX_EPOCH),
            Err(S3ModelError::ObjectLockRequiresEnabledVersioning)
        );
    }

    #[test]
    fn retention_requires_object_lock() {
        let retention =
            DefaultRetention::new(RetentionMode::Governance, DefaultRetentionPeriod::Days(30))
                .expect("valid retention");
        assert_eq!(
            BucketS3Configuration::new(
                "us-east-1",
                false,
                Some(retention),
                None,
                OffsetDateTime::UNIX_EPOCH,
            ),
            Err(S3ModelError::RetentionRequiresObjectLock)
        );
    }

    #[test]
    fn object_lock_enablement_is_irreversible_and_revisioned_as_one_change() {
        let mut configuration = configuration(false);
        let retention =
            DefaultRetention::new(RetentionMode::Compliance, DefaultRetentionPeriod::Years(2))
                .expect("valid retention");
        let enabled_at = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(1);
        assert!(
            configuration
                .replace_object_lock_configuration(Some(retention), enabled_at)
                .expect("enable Object Lock")
        );
        assert!(configuration.object_lock_enabled());
        assert_eq!(configuration.versioning_status(), VersioningStatus::Enabled);
        assert_eq!(configuration.default_retention(), Some(retention));
        assert_eq!(configuration.revision(), 2);
        assert_eq!(configuration.updated_at(), enabled_at);

        assert!(
            !configuration
                .replace_object_lock_configuration(Some(retention), enabled_at)
                .expect("idempotent replacement")
        );
        let cleared_at = enabled_at + time::Duration::seconds(1);
        assert!(
            configuration
                .replace_object_lock_configuration(None, cleared_at)
                .expect("clear only the default retention")
        );
        assert!(configuration.object_lock_enabled());
        assert_eq!(configuration.versioning_status(), VersioningStatus::Enabled);
        assert_eq!(configuration.default_retention(), None);
        assert_eq!(configuration.revision(), 3);
        assert_eq!(configuration.updated_at(), cleared_at);
    }
}
