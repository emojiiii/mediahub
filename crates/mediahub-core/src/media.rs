use serde::Serialize;
use time::OffsetDateTime;

use crate::{
    ApplicationId, BucketId, CURRENT_METADATA_VERSION, ClientMetadata, DomainError, DomainResult,
    MediaId, MediaMetadata, SystemMetadata, Visibility,
};

/// Lifecycle state of an immutable media object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaState {
    Uploading,
    Active,
    ArchivePending,
    Archived,
    DeletePending,
    Deleted,
    Quarantined,
}

impl MediaState {
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Uploading, Self::Active)
                | (Self::Active, Self::ArchivePending)
                | (Self::ArchivePending, Self::Archived)
                | (Self::Active, Self::DeletePending)
                | (Self::DeletePending, Self::Deleted)
                | (Self::Active, Self::Quarantined)
        )
    }

    pub fn ensure_transition_to(self, next: Self) -> DomainResult<()> {
        if self.can_transition_to(next) {
            Ok(())
        } else {
            Err(DomainError::InvalidMediaStateTransition {
                from: self,
                to: next,
            })
        }
    }

    #[must_use]
    pub const fn is_readable(self) -> bool {
        matches!(self, Self::Active)
    }
}

/// Optional image dimensions. Both measurements are always present together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct MediaDimensions {
    width: u32,
    height: u32,
}

impl MediaDimensions {
    pub fn new(width: u32, height: u32) -> DomainResult<Self> {
        if width == 0 || height == 0 {
            return Err(DomainError::InvalidMetadataShape);
        }
        Ok(Self { width, height })
    }

    #[must_use]
    pub const fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub const fn height(self) -> u32 {
        self.height
    }
}

/// Server-side input used to create a new uploading media object.
#[derive(Clone, Debug)]
pub struct NewMedia {
    pub id: MediaId,
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub original_name: Option<String>,
    pub display_name: String,
    pub extension: Option<String>,
    pub storage_backend: String,
    pub storage_key: String,
    pub visibility_override: Option<Visibility>,
    pub expire_at: Option<OffsetDateTime>,
    pub system_metadata: SystemMetadata,
    pub client_metadata: ClientMetadata,
}

/// Stable data-transfer shape for persistence adapters. Deserializing this DTO
/// alone does not create a domain object; pass it to [`Media::from_persistence`]
/// so every current invariant is checked before an aggregate is rehydrated.
#[derive(Clone, Debug, PartialEq, Serialize, serde::Deserialize)]
pub struct PersistedMedia {
    pub id: MediaId,
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub original_name: Option<String>,
    pub display_name: String,
    pub extension: Option<String>,
    pub storage_backend: String,
    pub storage_key: String,
    pub state: MediaState,
    pub visibility_override: Option<Visibility>,
    pub system_metadata: PersistedSystemMetadata,
    pub client_metadata: ClientMetadata,
    pub metadata_version: u32,
    pub revision: u64,
    pub expire_at: Option<OffsetDateTime>,
    pub archived_at: Option<OffsetDateTime>,
    pub deleted_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// Serialization-friendly representation of MediaHub-generated media facts.
/// It is only accepted through [`Media::from_persistence`], not public client
/// metadata parsing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct PersistedSystemMetadata {
    pub mime: String,
    pub size: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_ms: Option<u64>,
    pub sha256: String,
}

/// An immutable binary object with mutable metadata, access, and lifecycle
/// properties. Content identity and storage location have no mutators.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Media {
    id: MediaId,
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: String,
    original_name: Option<String>,
    display_name: String,
    mime: String,
    extension: Option<String>,
    size: u64,
    dimensions: Option<MediaDimensions>,
    duration_ms: Option<u64>,
    sha256: String,
    etag: String,
    storage_backend: String,
    storage_key: String,
    state: MediaState,
    visibility_override: Option<Visibility>,
    metadata: MediaMetadata,
    metadata_version: u32,
    revision: u64,
    expire_at: Option<OffsetDateTime>,
    archived_at: Option<OffsetDateTime>,
    deleted_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl Media {
    pub fn new(input: NewMedia, now: OffsetDateTime) -> DomainResult<Self> {
        validate_object_key(&input.object_key)?;
        validate_text_field("display name", &input.display_name, 255)?;
        if let Some(original_name) = &input.original_name {
            validate_text_field("original name", original_name, 255)?;
        }
        if let Some(extension) = &input.extension {
            validate_text_field("extension", extension, 32)?;
        }
        validate_text_field("storage backend", &input.storage_backend, 128)?;
        validate_text_field("storage key", &input.storage_key, 1024)?;

        let dimensions = match (
            input.system_metadata.width(),
            input.system_metadata.height(),
        ) {
            (Some(width), Some(height)) => Some(MediaDimensions::new(width, height)?),
            (None, None) => None,
            _ => return Err(DomainError::InvalidMetadataShape),
        };
        let metadata = MediaMetadata::new(input.system_metadata, input.client_metadata)?;

        Ok(Self {
            id: input.id,
            application_id: input.application_id,
            bucket_id: input.bucket_id,
            object_key: input.object_key,
            original_name: input.original_name,
            display_name: input.display_name,
            mime: metadata.system().mime().to_owned(),
            extension: input.extension,
            size: metadata.system().size(),
            dimensions,
            duration_ms: metadata.system().duration_ms(),
            sha256: metadata.system().sha256().to_owned(),
            etag: metadata.system().sha256().to_owned(),
            storage_backend: input.storage_backend,
            storage_key: input.storage_key,
            state: MediaState::Uploading,
            visibility_override: input.visibility_override,
            metadata,
            metadata_version: CURRENT_METADATA_VERSION,
            revision: 0,
            expire_at: input.expire_at,
            archived_at: None,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        })
    }

    /// Rehydrates a media aggregate supplied by a persistence adapter.
    ///
    /// This is intentionally stricter than deserializing `Media` directly:
    /// static content fields, metadata limits, and terminal-state timestamps
    /// are all checked before the value becomes usable by the domain layer.
    pub fn from_persistence(persisted: PersistedMedia) -> DomainResult<Self> {
        validate_object_key(&persisted.object_key)?;
        validate_text_field("display name", &persisted.display_name, 255)?;
        if let Some(original_name) = &persisted.original_name {
            validate_text_field("original name", original_name, 255)?;
        }
        if let Some(extension) = &persisted.extension {
            validate_text_field("extension", extension, 32)?;
        }
        validate_text_field("storage backend", &persisted.storage_backend, 128)?;
        validate_text_field("storage key", &persisted.storage_key, 1024)?;
        if persisted.metadata_version == 0 {
            return Err(DomainError::InvalidMetadataVersion);
        }
        validate_terminal_timestamps(persisted.state, persisted.archived_at, persisted.deleted_at)?;

        let system_metadata = SystemMetadata::new(
            persisted.system_metadata.mime,
            persisted.system_metadata.size,
            persisted.system_metadata.width,
            persisted.system_metadata.height,
            persisted.system_metadata.duration_ms,
            persisted.system_metadata.sha256,
        )?;
        let dimensions = match (system_metadata.width(), system_metadata.height()) {
            (Some(width), Some(height)) => Some(MediaDimensions::new(width, height)?),
            (None, None) => None,
            _ => return Err(DomainError::InvalidMetadataShape),
        };
        let metadata = MediaMetadata::new(system_metadata, persisted.client_metadata)?;
        let sha256 = metadata.system().sha256().to_owned();

        Ok(Self {
            id: persisted.id,
            application_id: persisted.application_id,
            bucket_id: persisted.bucket_id,
            object_key: persisted.object_key,
            original_name: persisted.original_name,
            display_name: persisted.display_name,
            mime: metadata.system().mime().to_owned(),
            extension: persisted.extension,
            size: metadata.system().size(),
            dimensions,
            duration_ms: metadata.system().duration_ms(),
            etag: sha256.clone(),
            sha256,
            storage_backend: persisted.storage_backend,
            storage_key: persisted.storage_key,
            state: persisted.state,
            visibility_override: persisted.visibility_override,
            metadata,
            metadata_version: persisted.metadata_version,
            revision: persisted.revision,
            expire_at: persisted.expire_at,
            archived_at: persisted.archived_at,
            deleted_at: persisted.deleted_at,
            created_at: persisted.created_at,
            updated_at: persisted.updated_at,
        })
    }

    #[must_use]
    pub fn to_persisted(&self) -> PersistedMedia {
        PersistedMedia {
            id: self.id,
            application_id: self.application_id,
            bucket_id: self.bucket_id,
            object_key: self.object_key.clone(),
            original_name: self.original_name.clone(),
            display_name: self.display_name.clone(),
            extension: self.extension.clone(),
            storage_backend: self.storage_backend.clone(),
            storage_key: self.storage_key.clone(),
            state: self.state,
            visibility_override: self.visibility_override,
            system_metadata: PersistedSystemMetadata {
                mime: self.mime.clone(),
                size: self.size,
                width: self.dimensions.map(MediaDimensions::width),
                height: self.dimensions.map(MediaDimensions::height),
                duration_ms: self.duration_ms,
                sha256: self.sha256.clone(),
            },
            client_metadata: ClientMetadata::new(
                self.metadata.user().clone(),
                self.metadata.ai().clone(),
            )
            .expect("existing metadata is validated"),
            metadata_version: self.metadata_version,
            revision: self.revision,
            expire_at: self.expire_at,
            archived_at: self.archived_at,
            deleted_at: self.deleted_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> MediaId {
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
    pub fn original_name(&self) -> Option<&str> {
        self.original_name.as_deref()
    }

    #[must_use]
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    #[must_use]
    pub fn mime(&self) -> &str {
        &self.mime
    }

    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        self.extension.as_deref()
    }

    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    #[must_use]
    pub const fn dimensions(&self) -> Option<MediaDimensions> {
        self.dimensions
    }

    #[must_use]
    pub const fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    #[must_use]
    pub fn etag(&self) -> &str {
        &self.etag
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
    pub const fn state(&self) -> MediaState {
        self.state
    }

    #[must_use]
    pub const fn visibility_override(&self) -> Option<Visibility> {
        self.visibility_override
    }

    #[must_use]
    pub const fn effective_visibility(&self, bucket_visibility: Visibility) -> Visibility {
        match self.visibility_override {
            Some(visibility) => visibility,
            None => bucket_visibility,
        }
    }

    #[must_use]
    pub const fn metadata(&self) -> &MediaMetadata {
        &self.metadata
    }

    #[must_use]
    pub const fn metadata_version(&self) -> u32 {
        self.metadata_version
    }

    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub const fn expire_at(&self) -> Option<OffsetDateTime> {
        self.expire_at
    }

    #[must_use]
    pub const fn archived_at(&self) -> Option<OffsetDateTime> {
        self.archived_at
    }

    #[must_use]
    pub const fn deleted_at(&self) -> Option<OffsetDateTime> {
        self.deleted_at
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    #[must_use]
    pub const fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }

    pub fn ensure_readable(&self) -> DomainResult<()> {
        if self.state.is_readable() {
            Ok(())
        } else {
            Err(DomainError::MediaNotReadable { state: self.state })
        }
    }

    pub fn transition_to(&mut self, next: MediaState, now: OffsetDateTime) -> DomainResult<()> {
        self.state.ensure_transition_to(next)?;
        self.state = next;
        if matches!(next, MediaState::Archived) {
            self.archived_at = Some(now);
        }
        if matches!(next, MediaState::Deleted) {
            self.deleted_at = Some(now);
        }
        self.touch(now);
        Ok(())
    }

    pub fn replace_client_metadata(
        &mut self,
        client_metadata: ClientMetadata,
        expected_revision: u64,
        now: OffsetDateTime,
    ) -> DomainResult<()> {
        self.ensure_expected_revision(expected_revision)?;
        self.metadata.replace_client_metadata(client_metadata)?;
        self.touch(now);
        Ok(())
    }

    pub fn set_display_name(
        &mut self,
        display_name: impl Into<String>,
        expected_revision: u64,
        now: OffsetDateTime,
    ) -> DomainResult<()> {
        self.ensure_expected_revision(expected_revision)?;
        let display_name = display_name.into();
        validate_text_field("display name", &display_name, 255)?;
        self.display_name = display_name;
        self.touch(now);
        Ok(())
    }

    pub fn set_visibility_override(
        &mut self,
        visibility_override: Option<Visibility>,
        expected_revision: u64,
        now: OffsetDateTime,
    ) -> DomainResult<()> {
        self.ensure_expected_revision(expected_revision)?;
        self.visibility_override = visibility_override;
        self.touch(now);
        Ok(())
    }

    pub fn set_expire_at(
        &mut self,
        expire_at: Option<OffsetDateTime>,
        expected_revision: u64,
        now: OffsetDateTime,
    ) -> DomainResult<()> {
        self.ensure_expected_revision(expected_revision)?;
        self.expire_at = expire_at;
        self.touch(now);
        Ok(())
    }

    fn ensure_expected_revision(&self, expected_revision: u64) -> DomainResult<()> {
        if self.revision == expected_revision {
            Ok(())
        } else {
            Err(DomainError::RevisionConflict {
                expected: expected_revision,
                actual: self.revision,
            })
        }
    }

    fn touch(&mut self, now: OffsetDateTime) {
        self.revision = self.revision.saturating_add(1);
        self.updated_at = now;
    }
}

fn validate_object_key(value: &str) -> DomainResult<()> {
    if value.is_empty() || value.len() > 1024 || value.as_bytes().contains(&0) {
        return Err(DomainError::InvalidObjectKey);
    }
    Ok(())
}

fn validate_text_field(field: &'static str, value: &str, max_bytes: usize) -> DomainResult<()> {
    if value.is_empty()
        || value.len() > max_bytes
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(DomainError::InvalidTextField { field, max_bytes });
    }
    Ok(())
}

fn validate_terminal_timestamps(
    state: MediaState,
    archived_at: Option<OffsetDateTime>,
    deleted_at: Option<OffsetDateTime>,
) -> DomainResult<()> {
    let archived_valid = !matches!(state, MediaState::Archived) || archived_at.is_some();
    let deleted_valid = !matches!(state, MediaState::Deleted) || deleted_at.is_some();
    if archived_valid && deleted_valid {
        Ok(())
    } else {
        Err(DomainError::InvalidMetadataShape)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn new_media() -> Media {
        Media::new(
            NewMedia {
                id: MediaId::new(),
                application_id: ApplicationId::new(),
                bucket_id: BucketId::new(),
                object_key: "renders/image.png".to_owned(),
                original_name: Some("image.png".to_owned()),
                display_name: "Final image".to_owned(),
                extension: Some("png".to_owned()),
                storage_backend: "local".to_owned(),
                storage_key: "objects/2026/07/example.png".to_owned(),
                visibility_override: None,
                expire_at: None,
                system_metadata: SystemMetadata::new(
                    "image/png",
                    12,
                    Some(2),
                    Some(3),
                    None,
                    "a".repeat(64),
                )
                .expect("system metadata"),
                client_metadata: ClientMetadata::from_value(json!({
                    "user": {"project": "website-a"},
                    "ai": {"model": "gpt-image-2"}
                }))
                .expect("client metadata"),
            },
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("media")
    }

    #[test]
    fn media_state_machine_accepts_only_documented_transitions() {
        assert!(
            MediaState::Uploading
                .ensure_transition_to(MediaState::Active)
                .is_ok()
        );
        assert!(
            MediaState::Active
                .ensure_transition_to(MediaState::ArchivePending)
                .is_ok()
        );
        assert!(
            MediaState::Active
                .ensure_transition_to(MediaState::DeletePending)
                .is_ok()
        );
        assert!(
            MediaState::Active
                .ensure_transition_to(MediaState::Quarantined)
                .is_ok()
        );
        assert_eq!(
            MediaState::Archived.ensure_transition_to(MediaState::Active),
            Err(DomainError::InvalidMediaStateTransition {
                from: MediaState::Archived,
                to: MediaState::Active
            })
        );
    }

    #[test]
    fn only_active_media_is_readable() {
        let mut media = new_media();
        assert!(matches!(
            media.ensure_readable(),
            Err(DomainError::MediaNotReadable {
                state: MediaState::Uploading
            })
        ));

        media
            .transition_to(MediaState::Active, OffsetDateTime::UNIX_EPOCH)
            .expect("upload completes");
        assert!(media.ensure_readable().is_ok());
    }

    #[test]
    fn mutable_updates_use_revision_and_preserve_content_identity() {
        let mut media = new_media();
        let sha256 = media.sha256().to_owned();
        let storage_key = media.storage_key().to_owned();
        media
            .replace_client_metadata(
                ClientMetadata::from_value(json!({"user": {"project": "two"}}))
                    .expect("client metadata"),
                0,
                OffsetDateTime::UNIX_EPOCH,
            )
            .expect("update succeeds");

        assert_eq!(media.revision(), 1);
        assert_eq!(media.sha256(), sha256);
        assert_eq!(media.storage_key(), storage_key);
        assert_eq!(
            media.set_display_name("Nope", 0, OffsetDateTime::UNIX_EPOCH),
            Err(DomainError::RevisionConflict {
                expected: 0,
                actual: 1
            })
        );
    }

    #[test]
    fn persisted_media_rehydrates_only_after_domain_validation() {
        let media = new_media();
        let persisted = media.to_persisted();
        let rehydrated = Media::from_persistence(persisted.clone()).expect("valid persisted media");

        assert_eq!(rehydrated, media);

        let mut invalid = persisted;
        invalid.system_metadata.sha256 = "not-a-digest".to_owned();
        assert_eq!(
            Media::from_persistence(invalid),
            Err(DomainError::InvalidSha256)
        );
    }
}
