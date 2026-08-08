use async_trait::async_trait;
use mediahub_core::{
    ApplicationId, BucketId, BucketS3Configuration, Checksum, DefaultRetention, EntityTag,
    NewStorageGcTask, ObjectId, ObjectRetention, ObjectVersion, ObjectVersionId,
    ObjectVersionState, S3Bucket, S3LifecycleConfiguration, S3Object, S3ObjectTagSet, S3VersionId,
    StorageGcTask, StorageGcTaskId, UploadIntent, UploadIntentId, VersioningStatus,
};
use time::OffsetDateTime;

use crate::RepositoryError;

pub const MAX_S3_OBJECT_LIST_LIMIT: usize = 1_000;
pub const MAX_S3_VERSION_LIST_LIMIT: usize = 1_000;
pub const MAX_S3_MULTIPART_UPLOAD_LIST_LIMIT: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3ObjectListQuery {
    pub bucket_id: BucketId,
    pub prefix: String,
    pub start_after: Option<String>,
    pub delimiter: bool,
    pub limit: usize,
}

impl S3ObjectListQuery {
    pub fn validate(&self) -> Result<(), RepositoryError> {
        if self.limit > MAX_S3_OBJECT_LIST_LIMIT {
            return Err(RepositoryError::Invariant(
                "S3 object page limit must not exceed 1000".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3ObjectListItem {
    pub key: String,
    pub version: ObjectVersion,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct S3ObjectPage {
    pub items: Vec<S3ObjectListItem>,
    pub common_prefixes: Vec<String>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3ObjectVersionListQuery {
    pub bucket_id: BucketId,
    pub prefix: String,
    pub key_marker: Option<String>,
    pub version_id_marker: Option<S3VersionId>,
    pub delimiter: bool,
    pub limit: usize,
}

impl S3ObjectVersionListQuery {
    pub fn validate(&self) -> Result<(), RepositoryError> {
        if self.limit > MAX_S3_VERSION_LIST_LIMIT {
            return Err(RepositoryError::Invariant(
                "S3 object-version page limit must not exceed 1000".into(),
            ));
        }
        if self.version_id_marker.is_some() && self.key_marker.is_none() {
            return Err(RepositoryError::Invariant(
                "version-id-marker requires key-marker".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3ObjectVersionListItem {
    pub key: String,
    pub version: ObjectVersion,
    pub is_latest: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct S3ObjectVersionPage {
    pub items: Vec<S3ObjectVersionListItem>,
    pub common_prefixes: Vec<String>,
    pub next_key_marker: Option<String>,
    pub next_version_id_marker: Option<S3VersionId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3MultipartUploadListQuery {
    pub bucket_id: BucketId,
    pub prefix: String,
    pub key_marker: Option<String>,
    pub upload_id_marker: Option<String>,
    pub delimiter: bool,
    pub limit: usize,
    pub as_of: OffsetDateTime,
}

impl S3MultipartUploadListQuery {
    pub fn validate(&self) -> Result<(), RepositoryError> {
        if self.limit > MAX_S3_MULTIPART_UPLOAD_LIST_LIMIT {
            return Err(RepositoryError::Invariant(
                "S3 multipart-upload page limit must not exceed 1000".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3MultipartUploadListItem {
    pub key: String,
    pub upload_id: String,
    pub initiated_at: OffsetDateTime,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct S3MultipartUploadPage {
    pub items: Vec<S3MultipartUploadListItem>,
    pub common_prefixes: Vec<String>,
    pub next_key_marker: Option<String>,
    pub next_upload_id_marker: Option<String>,
}

/// Bucket-scoped S3 listings backed only by logical object/version and
/// multipart-upload metadata. Implementations must apply markers and the
/// delimiter before the shared `limit + 1` page window.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3ListingRepository: Send + Sync {
    async fn list_s3_object_versions_page(
        &self,
        application_id: ApplicationId,
        query: &S3ObjectVersionListQuery,
    ) -> Result<S3ObjectVersionPage, RepositoryError>;

    async fn list_s3_multipart_uploads_page(
        &self,
        application_id: ApplicationId,
        query: &S3MultipartUploadListQuery,
    ) -> Result<S3MultipartUploadPage, RepositoryError>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3DeleteLockReason {
    LegalHold,
    ComplianceRetention,
    GovernanceRetention,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteS3ObjectCommand {
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub version_id: Option<S3VersionId>,
    pub bypass_governance: bool,
    pub deleted_by: String,
    pub deleted_at: OffsetDateTime,
    pub delete_marker_id: ObjectVersionId,
    pub delete_marker_version_id: S3VersionId,
    pub gc_task_id: StorageGcTaskId,
    pub gc_not_before: OffsetDateTime,
    pub gc_max_attempts: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletedS3ObjectVersion {
    pub version_id: Option<S3VersionId>,
    pub delete_marker: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeleteS3ObjectOutcome {
    Deleted(DeletedS3ObjectVersion),
    NoOp,
    VersionNotFound,
    Locked(S3DeleteLockReason),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3ObjectLockMutation {
    Retention {
        retention: ObjectRetention,
        bypass_governance: bool,
    },
    LegalHold(bool),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PutS3ObjectLockCommand {
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub version_id: Option<S3VersionId>,
    pub mutation: S3ObjectLockMutation,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PutS3ObjectLockOutcome {
    Updated(ObjectVersion),
    ObjectNotFound,
    VersionNotFound,
    DeleteMarker {
        version_id: S3VersionId,
        is_current: bool,
    },
    ObjectLockNotEnabled,
    RetentionLocked(S3DeleteLockReason),
    InvalidRetention,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3ObjectTaggingCommand {
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub version_id: Option<S3VersionId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3ObjectTaggingOutcome {
    Found {
        object: S3Object,
        version: Box<ObjectVersion>,
        tags: S3ObjectTagSet,
    },
    ObjectNotFound,
    VersionNotFound,
    DeleteMarker {
        version_id: S3VersionId,
        is_current: bool,
    },
}

/// Persistence boundary for S3 bucket identity and configuration.
///
/// Mutations that inspect emptiness or configuration state must lock the
/// bucket row and complete in one transaction.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3BucketRepository: Send + Sync {
    async fn list_s3_buckets(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<S3Bucket>, RepositoryError>;

    async fn find_s3_bucket(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<S3Bucket>, RepositoryError>;

    /// Creates bucket identity and all S3 configuration in one transaction.
    async fn create_s3_bucket(&self, bucket: &S3Bucket) -> Result<(), RepositoryError>;

    /// Locks the bucket, verifies that no logical object or in-flight upload
    /// remains, and deletes it. Missing buckets return `false`.
    async fn delete_s3_bucket(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<bool, RepositoryError>;

    async fn get_s3_bucket_location(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<String>, RepositoryError>;

    async fn get_s3_bucket_configuration(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<BucketS3Configuration>, RepositoryError>;

    async fn get_s3_bucket_versioning(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<VersioningStatus>, RepositoryError>;

    /// Locks the bucket row before applying the irreversible versioning state
    /// machine. Object Lock buckets can only remain `Enabled`.
    async fn set_s3_bucket_versioning(
        &self,
        application_id: ApplicationId,
        name: &str,
        status: VersioningStatus,
        updated_at: OffsetDateTime,
    ) -> Result<BucketS3Configuration, RepositoryError>;

    /// Locks the bucket row, irreversibly enables Object Lock and versioning,
    /// and replaces the optional default retention rule in one transaction.
    async fn replace_s3_bucket_object_lock(
        &self,
        application_id: ApplicationId,
        name: &str,
        default_retention: Option<DefaultRetention>,
        updated_at: OffsetDateTime,
    ) -> Result<BucketS3Configuration, RepositoryError>;

    /// Stores normalized lifecycle configuration as a JSON document. XML
    /// parsing and S3 protocol validation remain outside this port.
    async fn replace_s3_bucket_lifecycle(
        &self,
        application_id: ApplicationId,
        name: &str,
        lifecycle_configuration: Option<S3LifecycleConfiguration>,
        updated_at: OffsetDateTime,
    ) -> Result<BucketS3Configuration, RepositoryError>;
}

/// Persistence boundary for logical objects and immutable S3 versions.
///
/// `objects.current_version_id` is the sole latest-version source of truth.
/// Implementations must lock the logical object row, mark the previous current
/// version's `became_noncurrent_at`, insert the new version, and switch the
/// pointer in one transaction.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3ObjectRepository: Send + Sync {
    async fn find_s3_object(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        key: &str,
    ) -> Result<Option<S3Object>, RepositoryError>;

    /// Lists only the current committed data version for each visible key.
    /// Items and common prefixes share one byte-ordered pagination window.
    async fn list_current_s3_objects(
        &self,
        application_id: ApplicationId,
        query: &S3ObjectListQuery,
    ) -> Result<S3ObjectPage, RepositoryError>;

    async fn find_s3_object_version(
        &self,
        object_id: ObjectId,
        version_id: &S3VersionId,
    ) -> Result<Option<ObjectVersion>, RepositoryError>;

    /// Internal audit lookup used by committed-intent replay. Unlike the S3
    /// versionId lookup, this may return a superseded null-version row.
    ///
    /// Implementations that have not yet adopted superseded audit rows fail
    /// loudly instead of silently pretending the version is absent.
    async fn find_s3_object_version_by_id(
        &self,
        version_id: ObjectVersionId,
    ) -> Result<Option<ObjectVersion>, RepositoryError> {
        let _ = version_id;
        Err(RepositoryError::Unavailable(
            "internal object-version audit lookup is not implemented".into(),
        ))
    }

    /// Resolves one immutable, committed data version inside an Application.
    ///
    /// This is the control-plane read boundary for version-addressed content:
    /// implementations must apply the Application predicate in the storage
    /// query and must not return delete markers or non-committed versions.
    async fn find_committed_s3_object_version_for_application(
        &self,
        application_id: ApplicationId,
        version_id: ObjectVersionId,
    ) -> Result<Option<S3ObjectVersionRead>, RepositoryError>;

    async fn find_current_s3_object_version(
        &self,
        object_id: ObjectId,
    ) -> Result<Option<ObjectVersion>, RepositoryError>;

    async fn list_s3_object_versions(
        &self,
        object_id: ObjectId,
    ) -> Result<Vec<ObjectVersion>, RepositoryError>;

    /// Reads tags for an already resolved committed data version while
    /// preserving Application isolation. `None` means the version is not a
    /// visible data version for that Application; an empty set is a valid hit.
    async fn find_s3_object_version_tags(
        &self,
        application_id: ApplicationId,
        version_id: ObjectVersionId,
    ) -> Result<Option<S3ObjectTagSet>, RepositoryError>;

    async fn get_s3_object_tagging(
        &self,
        command: &S3ObjectTaggingCommand,
    ) -> Result<S3ObjectTaggingOutcome, RepositoryError>;

    /// Replaces all tags after locking the tenant bucket, logical object and
    /// exact committed data version. An empty set implements DeleteObjectTagging.
    async fn replace_s3_object_tagging(
        &self,
        command: &S3ObjectTaggingCommand,
        tags: &S3ObjectTagSet,
    ) -> Result<S3ObjectTaggingOutcome, RepositoryError>;

    /// Applies S3 delete semantics while holding the bucket configuration,
    /// logical object, and affected version rows. Object Lock checks, head
    /// changes, audit visibility, and persistent GC enqueue are atomic.
    async fn delete_s3_object(
        &self,
        command: &DeleteS3ObjectCommand,
    ) -> Result<DeleteS3ObjectOutcome, RepositoryError>;

    /// Updates retention or legal hold while locking the tenant bucket,
    /// logical object and exact immutable version identity in one transaction.
    async fn put_s3_object_lock(
        &self,
        command: &PutS3ObjectLockCommand,
    ) -> Result<PutS3ObjectLockOutcome, RepositoryError>;

    /// Creates an empty logical object, inserts generation one, then points the    /// object at that version in one transaction.
    async fn create_s3_object_with_version(
        &self,
        object: S3Object,
        version: ObjectVersion,
        updated_at: OffsetDateTime,
    ) -> Result<S3Object, RepositoryError>;

    /// Uses `expected_generation` as a compare-and-set fence while holding a
    /// row lock. A stale writer receives `RepositoryError::Conflict`.
    async fn append_s3_object_version(
        &self,
        object_id: ObjectId,
        expected_generation: u64,
        version: ObjectVersion,
        updated_at: OffsetDateTime,
    ) -> Result<S3Object, RepositoryError>;
}

/// Tenant-scoped read model for immutable ObjectVersion content experiences.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3ObjectVersionRead {
    pub object_key: String,
    pub version: ObjectVersion,
}

pub const MAX_UPLOAD_INTENT_EXPIRY_LIMIT: usize = 1_000;
pub const MAX_STORAGE_GC_CLAIM_LIMIT: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3ObjectCommitTarget {
    Create(S3Object),
    Append {
        object_id: ObjectId,
        expected_generation: u64,
    },
}

/// Complete description of one atomic version-history commit. Any GC tasks in
/// this command must be inserted in the same transaction as the head switch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3ObjectVersionCommit {
    pub target: S3ObjectCommitTarget,
    pub version: ObjectVersion,
    pub committed_at: OffsetDateTime,
    pub replaced_null_version_id: Option<ObjectVersionId>,
    pub gc_tasks: Vec<NewStorageGcTask>,
    /// Explicit S3 request values. The repository combines these with the
    /// bucket default while holding the bucket row lock, then freezes the
    /// resulting values on the inserted ObjectVersion.
    pub requested_retention: Option<ObjectRetention>,
    pub requested_legal_hold: Option<bool>,
}

impl S3ObjectVersionCommit {
    pub fn validate(&self) -> Result<(), RepositoryError> {
        if self.version.state() != ObjectVersionState::Committed {
            return Err(RepositoryError::Invariant(
                "only committed versions may enter object history".into(),
            ));
        }
        if self.replaced_null_version_id.is_some()
            && (!self.version.is_null_version()
                || self.replaced_null_version_id == Some(self.version.id()))
        {
            return Err(RepositoryError::Invariant(
                "only a new null version may identify the active null version it replaces".into(),
            ));
        }
        match &self.target {
            S3ObjectCommitTarget::Create(object) => {
                if object.generation() != 0
                    || object.current_version_id().is_some()
                    || self.version.object_id() != object.id()
                    || self.version.application_id() != object.application_id()
                    || self.version.bucket_id() != object.bucket_id()
                    || self.version.generation() != 1
                    || self.replaced_null_version_id.is_some()
                {
                    return Err(RepositoryError::Invariant(
                        "new-object version commit identity is inconsistent".into(),
                    ));
                }
            }
            S3ObjectCommitTarget::Append {
                object_id,
                expected_generation,
            } => {
                let next_generation = expected_generation.checked_add(1).ok_or_else(|| {
                    RepositoryError::Invariant("object generation overflow".into())
                })?;
                if self.version.object_id() != *object_id
                    || self.version.generation() != next_generation
                {
                    return Err(RepositoryError::Invariant(
                        "append version commit generation is inconsistent".into(),
                    ));
                }
            }
        }
        for task in &self.gc_tasks {
            task.validate()
                .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        }
        Ok(())
    }
}

/// Durable staged-upload boundary. Implementations fence claims with a lease;
/// version commit, head switch, multipart terminal state and GC enqueue are
/// transactional and never write Media.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3UploadIntentRepository: Send + Sync {
    async fn create_upload_intent(&self, intent: &UploadIntent) -> Result<(), RepositoryError>;

    async fn find_upload_intent(
        &self,
        intent_id: UploadIntentId,
    ) -> Result<Option<UploadIntent>, RepositoryError>;

    /// Freezes facts produced by the completed stream. This is the only valid
    /// `Staging -> Ready` transition and must verify the expected byte length.
    async fn complete_upload_intent_staging(
        &self,
        intent_id: UploadIntentId,
        entity_tag: &EntityTag,
        checksum: &Checksum,
        size_bytes: u64,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError>;
    async fn claim_upload_intent(
        &self,
        intent_id: UploadIntentId,
        lease_token: &str,
        lease_until: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError>;

    async fn release_upload_intent(
        &self,
        intent_id: UploadIntentId,
        lease_token: &str,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError>;

    async fn commit_upload_intent(
        &self,
        intent_id: UploadIntentId,
        lease_token: &str,
        commit: S3ObjectVersionCommit,
    ) -> Result<S3Object, RepositoryError>;

    async fn abort_upload_intent_and_enqueue_gc(
        &self,
        intent_id: UploadIntentId,
        gc_task: NewStorageGcTask,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError>;

    async fn expire_upload_intents(
        &self,
        now: OffsetDateTime,
        limit: usize,
        gc_max_attempts: u32,
    ) -> Result<usize, RepositoryError>;

    /// Commits the composed payload through the existing 0006 multipart row.
    async fn commit_multipart_object_version(
        &self,
        upload_id: &str,
        completion_token: &str,
        commit: S3ObjectVersionCommit,
        final_entity_tag: &EntityTag,
        final_checksum: &Checksum,
    ) -> Result<S3Object, RepositoryError>;
}

#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait StorageGcRepository: Send + Sync {
    async fn enqueue_storage_gc_task(&self, task: NewStorageGcTask) -> Result<(), RepositoryError>;

    async fn claim_storage_gc_tasks(
        &self,
        lease_token: &str,
        lease_until: OffsetDateTime,
        now: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<StorageGcTask>, RepositoryError>;

    async fn complete_storage_gc_task(
        &self,
        task_id: StorageGcTaskId,
        lease_token: &str,
        completed_at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    async fn fail_storage_gc_task(
        &self,
        task_id: StorageGcTaskId,
        lease_token: &str,
        error: &str,
        retry_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<StorageGcTask, RepositoryError>;
}

#[cfg(test)]
mod phase_one_tests {
    use mediahub_core::{
        ApplicationId, BucketId, Checksum, EntityTag, ObjectId, ObjectVersionId, S3VersionId,
        SourceProtocol, StoredObjectVersion,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn staged_version_cannot_be_committed_to_history() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let object = S3Object::new(
            ObjectId::new(),
            application_id,
            bucket_id,
            "asset.bin",
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("object");
        let payload = StoredObjectVersion::new(
            "filesystem",
            "staging/asset",
            None,
            None,
            EntityTag::new("etag").expect("etag"),
            4,
            None,
            json!({}),
            Some(Checksum::sha256_hex("0".repeat(64)).expect("checksum")),
        )
        .expect("payload");
        let version = ObjectVersion::new_object(
            ObjectVersionId::new(),
            object.id(),
            application_id,
            bucket_id,
            S3VersionId::new("version-1").expect("version"),
            1,
            false,
            ObjectVersionState::Staged,
            payload,
            None,
            false,
            "test",
            SourceProtocol::S3,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("version");
        let commit = S3ObjectVersionCommit {
            target: S3ObjectCommitTarget::Create(object),
            version,
            committed_at: OffsetDateTime::UNIX_EPOCH,
            replaced_null_version_id: None,
            gc_tasks: Vec::new(),
            requested_retention: None,
            requested_legal_hold: None,
        };
        assert!(matches!(
            commit.validate(),
            Err(RepositoryError::Invariant(_))
        ));
    }

    #[test]
    fn replacement_hint_is_only_valid_for_a_new_null_version() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let object_id = ObjectId::new();
        let payload = StoredObjectVersion::new(
            "filesystem",
            "objects/new-null",
            None,
            None,
            EntityTag::new("etag").expect("etag"),
            4,
            None,
            json!({}),
            Some(Checksum::sha256_hex("0".repeat(64)).expect("checksum")),
        )
        .expect("payload");
        let replacement_id = ObjectVersionId::new();
        let target = S3ObjectCommitTarget::Append {
            object_id,
            expected_generation: 1,
        };
        let non_null = ObjectVersion::new_object(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            S3VersionId::new("version-2").expect("version"),
            2,
            false,
            ObjectVersionState::Committed,
            payload.clone(),
            None,
            false,
            "test",
            SourceProtocol::S3,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("version");
        let invalid = S3ObjectVersionCommit {
            target: target.clone(),
            version: non_null,
            committed_at: OffsetDateTime::UNIX_EPOCH,
            replaced_null_version_id: Some(replacement_id),
            gc_tasks: Vec::new(),
            requested_retention: None,
            requested_legal_hold: None,
        };
        assert!(matches!(
            invalid.validate(),
            Err(RepositoryError::Invariant(_))
        ));

        let null = ObjectVersion::new_object(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            S3VersionId::new("null").expect("version"),
            2,
            true,
            ObjectVersionState::Committed,
            payload,
            None,
            false,
            "test",
            SourceProtocol::S3,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("version");
        let valid = S3ObjectVersionCommit {
            target,
            version: null,
            committed_at: OffsetDateTime::UNIX_EPOCH,
            replaced_null_version_id: Some(replacement_id),
            gc_tasks: Vec::new(),
            requested_retention: None,
            requested_legal_hold: None,
        };
        assert_eq!(valid.validate(), Ok(()));
    }

    #[test]
    fn object_list_limit_allows_zero_and_rejects_more_than_one_thousand() {
        let bucket_id = BucketId::new();
        for limit in [0, 1, MAX_S3_OBJECT_LIST_LIMIT] {
            assert_eq!(
                S3ObjectListQuery {
                    bucket_id,
                    prefix: String::new(),
                    start_after: None,
                    delimiter: false,
                    limit,
                }
                .validate(),
                Ok(())
            );
        }
        assert!(matches!(
            S3ObjectListQuery {
                bucket_id,
                prefix: String::new(),
                start_after: None,
                delimiter: false,
                limit: MAX_S3_OBJECT_LIST_LIMIT + 1,
            }
            .validate(),
            Err(RepositoryError::Invariant(_))
        ));
    }
}
