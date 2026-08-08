use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{ApplicationId, BucketId, ObjectId, ObjectVersionId, S3ModelError};

const MAX_OPAQUE_TAG_BYTES: usize = 1_024;
const MAX_STORAGE_NAME_BYTES: usize = 255;
const MAX_STORAGE_KEY_BYTES: usize = 2_048;
const MAX_GC_ERROR_BYTES: usize = 4_096;

macro_rules! uuid_id {
    ($name:ident) => {
        #[derive(
            Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self::from_uuid(value)
            }
        }

        impl From<$name> for Uuid {
            fn from(value: $name) -> Self {
                value.as_uuid()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self::from_uuid)
            }
        }
    };
}

uuid_id!(UploadIntentId);
uuid_id!(StorageGcTaskId);

/// Canonical, unquoted HTTP entity-tag value. Quoting belongs to the protocol
/// adapter so persisted values cannot accidentally be double quoted.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntityTag(String);

impl EntityTag {
    pub fn new(value: impl Into<String>) -> Result<Self, S3ModelError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= MAX_OPAQUE_TAG_BYTES
            && !value.contains('"')
            && !value.bytes().any(|byte| byte.is_ascii_control());
        if valid {
            Ok(Self(value))
        } else {
            Err(S3ModelError::InvalidEntityTag)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EntityTag {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    Sha256,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checksum {
    algorithm: ChecksumAlgorithm,
    value: String,
}

impl Checksum {
    pub fn sha256_hex(value: impl Into<String>) -> Result<Self, S3ModelError> {
        let value = value.into();
        if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(S3ModelError::InvalidChecksum);
        }
        Ok(Self {
            algorithm: ChecksumAlgorithm::Sha256,
            value: value.to_ascii_lowercase(),
        })
    }

    pub fn from_parts(
        algorithm: ChecksumAlgorithm,
        value: impl Into<String>,
    ) -> Result<Self, S3ModelError> {
        match algorithm {
            ChecksumAlgorithm::Sha256 => Self::sha256_hex(value),
        }
    }

    #[must_use]
    pub const fn algorithm(&self) -> ChecksumAlgorithm {
        self.algorithm
    }

    #[must_use]
    pub fn value(&self) -> &str {
        &self.value
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadIntentState {
    Staging,
    Ready,
    Committing,
    Committed,
    Aborted,
    Expired,
}

/// Durable owner of one in-flight single-object payload. The intent is
/// persisted before bytes are streamed. Payload facts become immutable when
/// the state advances from `Staging` to `Ready`; no staged row enters object
/// version history before the final fenced commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadIntent {
    id: UploadIntentId,
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: String,
    proposed_version_id: ObjectVersionId,
    state: UploadIntentState,
    storage_backend: String,
    temporary_storage_key: String,
    final_storage_key: String,
    entity_tag: Option<EntityTag>,
    checksum: Option<Checksum>,
    expected_size_bytes: u64,
    size_bytes: Option<u64>,
    content_type: Option<String>,
    user_metadata: Value,
    lease_token: Option<String>,
    lease_until: Option<OffsetDateTime>,
    committed_object_id: Option<ObjectId>,
    committed_version_id: Option<ObjectVersionId>,
    expires_at: OffsetDateTime,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl UploadIntent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: UploadIntentId,
        application_id: ApplicationId,
        bucket_id: BucketId,
        object_key: impl Into<String>,
        proposed_version_id: ObjectVersionId,
        storage_backend: impl Into<String>,
        temporary_storage_key: impl Into<String>,
        final_storage_key: impl Into<String>,
        expected_size_bytes: u64,
        content_type: Option<String>,
        user_metadata: Value,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<Self, S3ModelError> {
        Self::from_persistence(PersistedUploadIntent {
            id,
            application_id,
            bucket_id,
            object_key: object_key.into(),
            proposed_version_id,
            state: UploadIntentState::Staging,
            storage_backend: storage_backend.into(),
            temporary_storage_key: temporary_storage_key.into(),
            final_storage_key: final_storage_key.into(),
            entity_tag: None,
            checksum: None,
            expected_size_bytes,
            size_bytes: None,
            content_type,
            user_metadata,
            lease_token: None,
            lease_until: None,
            committed_object_id: None,
            committed_version_id: None,
            expires_at,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn from_persistence(persisted: PersistedUploadIntent) -> Result<Self, S3ModelError> {
        validate_object_key(&persisted.object_key)?;
        validate_storage_locator(&persisted.storage_backend, &persisted.temporary_storage_key)?;
        validate_storage_locator(&persisted.storage_backend, &persisted.final_storage_key)?;
        if persisted.temporary_storage_key == persisted.final_storage_key
            || !persisted.user_metadata.is_object()
            || persisted.expires_at <= persisted.created_at
        {
            return Err(S3ModelError::InvalidUploadIntent);
        }

        let fact_count = usize::from(persisted.entity_tag.is_some())
            + usize::from(persisted.checksum.is_some())
            + usize::from(persisted.size_bytes.is_some());
        if fact_count != 0 && fact_count != 3 {
            return Err(S3ModelError::InvalidUploadIntent);
        }
        let has_facts = fact_count == 3;
        if persisted
            .size_bytes
            .is_some_and(|size| size != persisted.expected_size_bytes)
        {
            return Err(S3ModelError::InvalidUploadIntent);
        }
        let has_lease = persisted.lease_token.is_some() && persisted.lease_until.is_some();
        let has_commit =
            persisted.committed_object_id.is_some() && persisted.committed_version_id.is_some();
        let state_is_valid = match persisted.state {
            UploadIntentState::Staging => !has_facts && !has_lease && !has_commit,
            UploadIntentState::Ready => has_facts && !has_lease && !has_commit,
            UploadIntentState::Committing => has_facts && has_lease && !has_commit,
            UploadIntentState::Committed => has_facts && !has_lease && has_commit,
            UploadIntentState::Aborted | UploadIntentState::Expired => !has_lease && !has_commit,
        };
        if !state_is_valid
            || persisted.lease_token.is_some() != persisted.lease_until.is_some()
            || persisted.committed_object_id.is_some() != persisted.committed_version_id.is_some()
            || persisted
                .lease_token
                .as_deref()
                .is_some_and(|token| !valid_token(token))
        {
            return Err(S3ModelError::InvalidUploadIntent);
        }

        Ok(Self {
            id: persisted.id,
            application_id: persisted.application_id,
            bucket_id: persisted.bucket_id,
            object_key: persisted.object_key,
            proposed_version_id: persisted.proposed_version_id,
            state: persisted.state,
            storage_backend: persisted.storage_backend,
            temporary_storage_key: persisted.temporary_storage_key,
            final_storage_key: persisted.final_storage_key,
            entity_tag: persisted.entity_tag,
            checksum: persisted.checksum,
            expected_size_bytes: persisted.expected_size_bytes,
            size_bytes: persisted.size_bytes,
            content_type: persisted.content_type,
            user_metadata: persisted.user_metadata,
            lease_token: persisted.lease_token,
            lease_until: persisted.lease_until,
            committed_object_id: persisted.committed_object_id,
            committed_version_id: persisted.committed_version_id,
            expires_at: persisted.expires_at,
            created_at: persisted.created_at,
            updated_at: persisted.updated_at,
        })
    }

    #[must_use]
    pub const fn id(&self) -> UploadIntentId {
        self.id
    }
    #[must_use]
    pub const fn application_id(&self) -> ApplicationId {
        self.application_id
    }
    #[must_use]
    pub const fn bucket_id(&self) -> BucketId {
        self.bucket_id
    }
    #[must_use]
    pub fn object_key(&self) -> &str {
        &self.object_key
    }
    #[must_use]
    pub const fn proposed_version_id(&self) -> ObjectVersionId {
        self.proposed_version_id
    }
    #[must_use]
    pub const fn state(&self) -> UploadIntentState {
        self.state
    }
    #[must_use]
    pub fn storage_backend(&self) -> &str {
        &self.storage_backend
    }
    #[must_use]
    pub fn temporary_storage_key(&self) -> &str {
        &self.temporary_storage_key
    }
    #[must_use]
    pub fn final_storage_key(&self) -> &str {
        &self.final_storage_key
    }
    #[must_use]
    pub const fn entity_tag(&self) -> Option<&EntityTag> {
        self.entity_tag.as_ref()
    }
    #[must_use]
    pub const fn checksum(&self) -> Option<&Checksum> {
        self.checksum.as_ref()
    }
    #[must_use]
    pub const fn expected_size_bytes(&self) -> u64 {
        self.expected_size_bytes
    }
    #[must_use]
    pub const fn size_bytes(&self) -> Option<u64> {
        self.size_bytes
    }
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }
    #[must_use]
    pub const fn user_metadata(&self) -> &Value {
        &self.user_metadata
    }
    #[must_use]
    pub fn lease_token(&self) -> Option<&str> {
        self.lease_token.as_deref()
    }
    #[must_use]
    pub const fn lease_until(&self) -> Option<OffsetDateTime> {
        self.lease_until
    }
    #[must_use]
    pub const fn committed_object_id(&self) -> Option<ObjectId> {
        self.committed_object_id
    }
    #[must_use]
    pub const fn committed_version_id(&self) -> Option<ObjectVersionId> {
        self.committed_version_id
    }
    #[must_use]
    pub const fn expires_at(&self) -> OffsetDateTime {
        self.expires_at
    }
    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }
    #[must_use]
    pub const fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistedUploadIntent {
    pub id: UploadIntentId,
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub proposed_version_id: ObjectVersionId,
    pub state: UploadIntentState,
    pub storage_backend: String,
    pub temporary_storage_key: String,
    pub final_storage_key: String,
    pub entity_tag: Option<EntityTag>,
    pub checksum: Option<Checksum>,
    pub expected_size_bytes: u64,
    pub size_bytes: Option<u64>,
    pub content_type: Option<String>,
    pub user_metadata: Value,
    pub lease_token: Option<String>,
    pub lease_until: Option<OffsetDateTime>,
    pub committed_object_id: Option<ObjectId>,
    pub committed_version_id: Option<ObjectVersionId>,
    pub expires_at: OffsetDateTime,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageGcReason {
    AbortedUploadIntent,
    MultipartTemporary,
    ReplacedNullVersion,
    LifecycleExpiration,
    ExplicitDelete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageGcTaskState {
    Pending,
    Leased,
    Completed,
    DeadLetter,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewStorageGcTask {
    pub id: StorageGcTaskId,
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_version_id: Option<ObjectVersionId>,
    pub upload_intent_id: Option<UploadIntentId>,
    pub multipart_upload_id: Option<String>,
    pub storage_backend: String,
    pub storage_key: String,
    pub reason: StorageGcReason,
    pub not_before: OffsetDateTime,
    pub max_attempts: u32,
    pub created_at: OffsetDateTime,
}

impl NewStorageGcTask {
    pub fn validate(&self) -> Result<(), S3ModelError> {
        validate_storage_locator(&self.storage_backend, &self.storage_key)?;
        if self.max_attempts == 0
            || self
                .multipart_upload_id
                .as_deref()
                .is_some_and(|value| !valid_token(value))
        {
            return Err(S3ModelError::InvalidStorageGcTask);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StorageGcTask {
    pub task: NewStorageGcTask,
    pub state: StorageGcTaskState,
    pub attempts: u32,
    pub lease_token: Option<String>,
    pub lease_until: Option<OffsetDateTime>,
    pub last_error: Option<String>,
    pub completed_at: Option<OffsetDateTime>,
    pub updated_at: OffsetDateTime,
}

impl StorageGcTask {
    pub fn validate(&self) -> Result<(), S3ModelError> {
        self.task.validate()?;
        if self.attempts > self.task.max_attempts
            || self
                .last_error
                .as_deref()
                .is_some_and(|error| error.len() > MAX_GC_ERROR_BYTES)
        {
            return Err(S3ModelError::InvalidStorageGcTask);
        }
        let has_lease = self.lease_token.is_some() && self.lease_until.is_some();
        let valid_state = match self.state {
            StorageGcTaskState::Pending => !has_lease && self.completed_at.is_none(),
            StorageGcTaskState::Leased => has_lease && self.completed_at.is_none(),
            StorageGcTaskState::Completed => !has_lease && self.completed_at.is_some(),
            StorageGcTaskState::DeadLetter => !has_lease && self.completed_at.is_none(),
        };
        if valid_state {
            Ok(())
        } else {
            Err(S3ModelError::InvalidStorageGcTask)
        }
    }
}

fn validate_storage_locator(backend: &str, key: &str) -> Result<(), S3ModelError> {
    if backend.is_empty()
        || backend.len() > MAX_STORAGE_NAME_BYTES
        || key.is_empty()
        || key.len() > MAX_STORAGE_KEY_BYTES
        || backend.bytes().any(|byte| byte.is_ascii_control())
        || key.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(S3ModelError::InvalidStorageLocator)
    } else {
        Ok(())
    }
}

fn validate_object_key(key: &str) -> Result<(), S3ModelError> {
    if key.is_empty() || key.len() > 1_024 || key.contains('\0') {
        Err(S3ModelError::InvalidObjectKey)
    } else {
        Ok(())
    }
}

fn valid_token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 1_024 && !value.bytes().any(|byte| byte.is_ascii_control())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn entity_tag_is_canonical_and_unquoted() {
        assert!(EntityTag::new("abc-2").is_ok());
        assert_eq!(
            EntityTag::new("\"abc\""),
            Err(S3ModelError::InvalidEntityTag)
        );
    }

    #[test]
    fn sha256_checksum_is_validated_and_normalized() {
        let checksum = Checksum::sha256_hex("A".repeat(64)).expect("checksum");
        assert_eq!(checksum.algorithm(), ChecksumAlgorithm::Sha256);
        assert_eq!(checksum.value(), "a".repeat(64));
        assert_eq!(
            Checksum::sha256_hex("abcd"),
            Err(S3ModelError::InvalidChecksum)
        );
    }

    #[test]
    fn upload_intent_never_requires_a_media_record() {
        let intent = UploadIntent::new(
            UploadIntentId::new(),
            ApplicationId::new(),
            BucketId::new(),
            "asset.png",
            ObjectVersionId::new(),
            "filesystem",
            "staging/asset",
            "objects/version",
            42,
            Some("image/png".into()),
            json!({}),
            OffsetDateTime::UNIX_EPOCH + time::Duration::hours(1),
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("intent");
        assert_eq!(intent.state(), UploadIntentState::Staging);
        assert!(intent.entity_tag().is_none());
        assert!(intent.checksum().is_none());
        assert!(intent.size_bytes().is_none());
        assert!(intent.committed_object_id().is_none());
    }
}
