use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{ApplicationId, BucketId, Checksum, EntityTag, RetentionMode, S3ModelError};

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

uuid_id!(ObjectId);
uuid_id!(ObjectVersionId);

/// Opaque S3 version ID. The literal `null` is valid and may appear once per
/// logical object while versioning is suspended.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct S3VersionId(String);

impl S3VersionId {
    pub fn new(value: impl Into<String>) -> Result<Self, S3ModelError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= 1_024
            && !value.bytes().any(|byte| byte.is_ascii_control());
        if valid {
            Ok(Self(value))
        } else {
            Err(S3ModelError::InvalidVersionId)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_null(&self) -> bool {
        self.0 == "null"
    }
}

impl fmt::Display for S3VersionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectVersionState {
    Staged,
    Committed,
    Deleting,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceProtocol {
    S3,
    Dav,
    Json,
    Processor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectRetention {
    mode: RetentionMode,
    retain_until: OffsetDateTime,
}

impl ObjectRetention {
    #[must_use]
    pub const fn new(mode: RetentionMode, retain_until: OffsetDateTime) -> Self {
        Self { mode, retain_until }
    }

    #[must_use]
    pub const fn mode(self) -> RetentionMode {
        self.mode
    }

    #[must_use]
    pub const fn retain_until(self) -> OffsetDateTime {
        self.retain_until
    }
}

/// Immutable content facts for a non-delete-marker version.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredObjectVersion {
    storage_backend: String,
    storage_key: String,
    provider_etag: Option<EntityTag>,
    provider_version: Option<String>,
    etag: EntityTag,
    size_bytes: u64,
    content_type: Option<String>,
    user_metadata: Value,
    checksum: Option<Checksum>,
}

impl StoredObjectVersion {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        storage_backend: impl Into<String>,
        storage_key: impl Into<String>,
        provider_etag: Option<EntityTag>,
        provider_version: Option<String>,
        etag: EntityTag,
        size_bytes: u64,
        content_type: Option<String>,
        user_metadata: Value,
        checksum: Option<Checksum>,
    ) -> Result<Self, S3ModelError> {
        let storage_backend = storage_backend.into();
        let storage_key = storage_key.into();
        if storage_backend.is_empty() || storage_key.is_empty() || !user_metadata.is_object() {
            return Err(S3ModelError::InvalidObjectVersionPayload);
        }
        Ok(Self {
            storage_backend,
            storage_key,
            provider_etag,
            provider_version,
            etag,
            size_bytes,
            content_type,
            user_metadata,
            checksum,
        })
    }

    #[must_use]
    pub fn storage_backend(&self) -> &str {
        &self.storage_backend
    }

    #[must_use]
    pub fn storage_key(&self) -> &str {
        &self.storage_key
    }

    #[must_use]
    pub const fn provider_etag(&self) -> Option<&EntityTag> {
        self.provider_etag.as_ref()
    }

    #[must_use]
    pub fn provider_version(&self) -> Option<&str> {
        self.provider_version.as_deref()
    }

    #[must_use]
    pub const fn etag(&self) -> &EntityTag {
        &self.etag
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
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
    pub const fn checksum(&self) -> Option<&Checksum> {
        self.checksum.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ObjectVersionPayload {
    Object(StoredObjectVersion),
    DeleteMarker,
}

/// One immutable S3-visible object version. Only lifecycle metadata such as
/// `became_noncurrent_at` may change after commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectVersion {
    id: ObjectVersionId,
    object_id: ObjectId,
    application_id: ApplicationId,
    bucket_id: BucketId,
    external_version_id: S3VersionId,
    generation: u64,
    is_null_version: bool,
    state: ObjectVersionState,
    payload: ObjectVersionPayload,
    retention: Option<ObjectRetention>,
    legal_hold: bool,
    created_by: String,
    source_protocol: SourceProtocol,
    created_at: OffsetDateTime,
    became_noncurrent_at: Option<OffsetDateTime>,
}

impl ObjectVersion {
    #[allow(clippy::too_many_arguments)]
    pub fn new_object(
        id: ObjectVersionId,
        object_id: ObjectId,
        application_id: ApplicationId,
        bucket_id: BucketId,
        external_version_id: S3VersionId,
        generation: u64,
        is_null_version: bool,
        state: ObjectVersionState,
        object: StoredObjectVersion,
        retention: Option<ObjectRetention>,
        legal_hold: bool,
        created_by: impl Into<String>,
        source_protocol: SourceProtocol,
        created_at: OffsetDateTime,
    ) -> Result<Self, S3ModelError> {
        Self::from_persistence(PersistedObjectVersion {
            id,
            object_id,
            application_id,
            bucket_id,
            external_version_id,
            generation,
            is_null_version,
            state,
            payload: ObjectVersionPayload::Object(object),
            retention,
            legal_hold,
            created_by: created_by.into(),
            source_protocol,
            created_at,
            became_noncurrent_at: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_delete_marker(
        id: ObjectVersionId,
        object_id: ObjectId,
        application_id: ApplicationId,
        bucket_id: BucketId,
        external_version_id: S3VersionId,
        generation: u64,
        is_null_version: bool,
        created_by: impl Into<String>,
        source_protocol: SourceProtocol,
        created_at: OffsetDateTime,
    ) -> Result<Self, S3ModelError> {
        Self::from_persistence(PersistedObjectVersion {
            id,
            object_id,
            application_id,
            bucket_id,
            external_version_id,
            generation,
            is_null_version,
            state: ObjectVersionState::Committed,
            payload: ObjectVersionPayload::DeleteMarker,
            retention: None,
            legal_hold: false,
            created_by: created_by.into(),
            source_protocol,
            created_at,
            became_noncurrent_at: None,
        })
    }

    pub fn from_persistence(persisted: PersistedObjectVersion) -> Result<Self, S3ModelError> {
        if persisted.generation == 0 {
            return Err(S3ModelError::InvalidObjectGeneration);
        }
        if persisted.is_null_version != persisted.external_version_id.is_null() {
            return Err(S3ModelError::InvalidVersionId);
        }
        if persisted.created_by.is_empty()
            || persisted
                .created_by
                .bytes()
                .any(|byte| byte.is_ascii_control())
        {
            return Err(S3ModelError::InvalidCreatedBy);
        }
        if matches!(persisted.payload, ObjectVersionPayload::DeleteMarker)
            && (persisted.retention.is_some() || persisted.legal_hold)
        {
            return Err(S3ModelError::InvalidObjectVersionPayload);
        }

        Ok(Self {
            id: persisted.id,
            object_id: persisted.object_id,
            application_id: persisted.application_id,
            bucket_id: persisted.bucket_id,
            external_version_id: persisted.external_version_id,
            generation: persisted.generation,
            is_null_version: persisted.is_null_version,
            state: persisted.state,
            payload: persisted.payload,
            retention: persisted.retention,
            legal_hold: persisted.legal_hold,
            created_by: persisted.created_by,
            source_protocol: persisted.source_protocol,
            created_at: persisted.created_at,
            became_noncurrent_at: persisted.became_noncurrent_at,
        })
    }

    #[must_use]
    pub const fn id(&self) -> ObjectVersionId {
        self.id
    }

    #[must_use]
    pub const fn object_id(&self) -> ObjectId {
        self.object_id
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
    pub const fn external_version_id(&self) -> &S3VersionId {
        &self.external_version_id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    #[must_use]
    pub const fn is_null_version(&self) -> bool {
        self.is_null_version
    }

    #[must_use]
    pub const fn state(&self) -> ObjectVersionState {
        self.state
    }

    #[must_use]
    pub const fn payload(&self) -> &ObjectVersionPayload {
        &self.payload
    }

    #[must_use]
    pub const fn is_delete_marker(&self) -> bool {
        matches!(self.payload, ObjectVersionPayload::DeleteMarker)
    }

    #[must_use]
    pub const fn retention(&self) -> Option<ObjectRetention> {
        self.retention
    }

    #[must_use]
    pub const fn legal_hold(&self) -> bool {
        self.legal_hold
    }

    #[must_use]
    pub fn created_by(&self) -> &str {
        &self.created_by
    }

    #[must_use]
    pub const fn source_protocol(&self) -> SourceProtocol {
        self.source_protocol
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    #[must_use]
    pub const fn became_noncurrent_at(&self) -> Option<OffsetDateTime> {
        self.became_noncurrent_at
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PersistedObjectVersion {
    pub id: ObjectVersionId,
    pub object_id: ObjectId,
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub external_version_id: S3VersionId,
    pub generation: u64,
    pub is_null_version: bool,
    pub state: ObjectVersionState,
    pub payload: ObjectVersionPayload,
    pub retention: Option<ObjectRetention>,
    pub legal_hold: bool,
    pub created_by: String,
    pub source_protocol: SourceProtocol,
    pub created_at: OffsetDateTime,
    pub became_noncurrent_at: Option<OffsetDateTime>,
}

/// Logical `(bucket, key)` record. A single nullable pointer is the only source
/// of latest-version truth; no `is_latest` flag is persisted on versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3Object {
    id: ObjectId,
    application_id: ApplicationId,
    bucket_id: BucketId,
    key: String,
    current_version_id: Option<ObjectVersionId>,
    generation: u64,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl S3Object {
    pub fn new(
        id: ObjectId,
        application_id: ApplicationId,
        bucket_id: BucketId,
        key: impl Into<String>,
        now: OffsetDateTime,
    ) -> Result<Self, S3ModelError> {
        Self::from_persistence(PersistedS3Object {
            id,
            application_id,
            bucket_id,
            key: key.into(),
            current_version_id: None,
            generation: 0,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn from_persistence(persisted: PersistedS3Object) -> Result<Self, S3ModelError> {
        validate_object_key(&persisted.key)?;
        Ok(Self {
            id: persisted.id,
            application_id: persisted.application_id,
            bucket_id: persisted.bucket_id,
            key: persisted.key,
            current_version_id: persisted.current_version_id,
            generation: persisted.generation,
            created_at: persisted.created_at,
            updated_at: persisted.updated_at,
        })
    }

    pub fn advanced_to(
        &self,
        version: &ObjectVersion,
        now: OffsetDateTime,
    ) -> Result<Self, S3ModelError> {
        if version.object_id != self.id
            || version.application_id != self.application_id
            || version.bucket_id != self.bucket_id
        {
            return Err(S3ModelError::ObjectVersionOwnershipMismatch);
        }
        if version.state != ObjectVersionState::Committed {
            return Err(S3ModelError::InvalidObjectVersionPayload);
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(S3ModelError::InvalidObjectGeneration)?;
        if version.generation != generation {
            return Err(S3ModelError::InvalidObjectGeneration);
        }
        Ok(Self {
            id: self.id,
            application_id: self.application_id,
            bucket_id: self.bucket_id,
            key: self.key.clone(),
            current_version_id: Some(version.id),
            generation,
            created_at: self.created_at,
            updated_at: now,
        })
    }

    #[must_use]
    pub const fn id(&self) -> ObjectId {
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
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub const fn current_version_id(&self) -> Option<ObjectVersionId> {
        self.current_version_id
    }

    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
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
pub struct PersistedS3Object {
    pub id: ObjectId,
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub key: String,
    pub current_version_id: Option<ObjectVersionId>,
    pub generation: u64,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

fn validate_object_key(key: &str) -> Result<(), S3ModelError> {
    if key.is_empty() || key.len() > 1_024 || key.contains('\0') {
        Err(S3ModelError::InvalidObjectKey)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn stored_object() -> StoredObjectVersion {
        StoredObjectVersion::new(
            "filesystem",
            "objects/version-1",
            None,
            None,
            EntityTag::new("etag").expect("etag"),
            4,
            Some("text/plain".into()),
            json!({}),
            Some(Checksum::sha256_hex("0".repeat(64)).expect("checksum")),
        )
        .expect("valid object")
    }

    #[test]
    fn current_version_is_derived_from_one_object_pointer() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let object = S3Object::new(
            ObjectId::new(),
            application_id,
            bucket_id,
            "folder/file.txt",
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("object");
        let version = ObjectVersion::new_object(
            ObjectVersionId::new(),
            object.id(),
            application_id,
            bucket_id,
            S3VersionId::new("version-1").expect("version ID"),
            1,
            false,
            ObjectVersionState::Committed,
            stored_object(),
            None,
            false,
            "test",
            SourceProtocol::S3,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("version");
        let advanced = object
            .advanced_to(&version, OffsetDateTime::UNIX_EPOCH)
            .expect("advance current pointer");

        assert_eq!(advanced.current_version_id(), Some(version.id()));
        assert_eq!(advanced.generation(), 1);
    }

    #[test]
    fn delete_marker_cannot_carry_retention_or_legal_hold() {
        let persisted = PersistedObjectVersion {
            id: ObjectVersionId::new(),
            object_id: ObjectId::new(),
            application_id: ApplicationId::new(),
            bucket_id: BucketId::new(),
            external_version_id: S3VersionId::new("marker").expect("version ID"),
            generation: 1,
            is_null_version: false,
            state: ObjectVersionState::Committed,
            payload: ObjectVersionPayload::DeleteMarker,
            retention: Some(ObjectRetention::new(
                RetentionMode::Governance,
                OffsetDateTime::UNIX_EPOCH,
            )),
            legal_hold: false,
            created_by: "test".into(),
            source_protocol: SourceProtocol::S3,
            created_at: OffsetDateTime::UNIX_EPOCH,
            became_noncurrent_at: None,
        };
        assert_eq!(
            ObjectVersion::from_persistence(persisted),
            Err(S3ModelError::InvalidObjectVersionPayload)
        );
    }

    #[test]
    fn identical_null_version_ids_are_valid_for_different_objects() {
        let left = S3VersionId::new("null").expect("null version");
        let right = S3VersionId::new("null").expect("null version");
        assert_eq!(left, right);
        assert_ne!(ObjectId::new(), ObjectId::new());
    }

    #[test]
    fn null_version_flag_must_match_external_version_id() {
        let base = PersistedObjectVersion {
            id: ObjectVersionId::new(),
            object_id: ObjectId::new(),
            application_id: ApplicationId::new(),
            bucket_id: BucketId::new(),
            external_version_id: S3VersionId::new("null").expect("version ID"),
            generation: 1,
            is_null_version: false,
            state: ObjectVersionState::Committed,
            payload: ObjectVersionPayload::Object(stored_object()),
            retention: None,
            legal_hold: false,
            created_by: "test".into(),
            source_protocol: SourceProtocol::S3,
            created_at: OffsetDateTime::UNIX_EPOCH,
            became_noncurrent_at: None,
        };

        assert_eq!(
            ObjectVersion::from_persistence(base.clone()),
            Err(S3ModelError::InvalidVersionId)
        );
        assert_eq!(
            ObjectVersion::from_persistence(PersistedObjectVersion {
                external_version_id: S3VersionId::new("version-1").expect("version ID"),
                is_null_version: true,
                ..base
            }),
            Err(S3ModelError::InvalidVersionId)
        );
    }
}
