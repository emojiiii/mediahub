use mediahub_core::{
    ApplicationId, BucketId, Checksum, EntityTag, NewStorageGcTask, ObjectId, ObjectRetention,
    ObjectVersion, ObjectVersionId, ObjectVersionPayload, ObjectVersionState, S3Bucket,
    S3ModelError, S3Object, S3VersionId, SourceProtocol, StorageGcReason, StorageGcTaskId,
    StoredObjectVersion, UploadIntent, UploadIntentId, UploadIntentState, VersioningStatus,
};
use serde_json::Value;
use thiserror::Error;
use time::{Duration, OffsetDateTime};

use crate::{
    Clock, DeleteS3ObjectCommand, DeleteS3ObjectOutcome, ObjectStore, ObjectStoreError,
    PutS3ObjectLockCommand, PutS3ObjectLockOutcome, RepositoryError, S3BucketRepository,
    S3DeleteLockReason, S3ObjectCommitTarget, S3ObjectLockMutation, S3ObjectRepository,
    S3ObjectVersionCommit, S3UploadIntentRepository, StreamedObject,
};

pub const DEFAULT_S3_UPLOAD_INTENT_TTL_SECONDS: i64 = 3_600;
pub const DEFAULT_S3_UPLOAD_LEASE_SECONDS: i64 = 120;
pub const DEFAULT_S3_GC_GRACE_SECONDS: i64 = 24 * 60 * 60;
pub const DEFAULT_S3_GC_MAX_ATTEMPTS: u32 = 10;

#[derive(Clone, Debug)]
pub struct BeginPutObjectRequest {
    pub application_id: ApplicationId,
    pub bucket_name: String,
    pub object_key: String,
    pub expected_size_bytes: u64,
    pub content_type: Option<String>,
    pub user_metadata: Value,
    pub expires_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug)]
pub struct BeginPutObjectReceipt {
    pub intent: UploadIntent,
}

#[derive(Clone, Debug)]
pub struct AbortStagedPutRequest {
    pub application_id: ApplicationId,
    pub intent_id: UploadIntentId,
}

#[derive(Clone, Debug)]
pub struct CompletePutObjectRequest {
    pub application_id: ApplicationId,
    pub intent_id: UploadIntentId,
    pub streamed: StreamedObject,
    pub created_by: String,
    pub source_protocol: SourceProtocol,
}

#[derive(Clone, Debug)]
pub struct CompletePutObjectReceipt {
    pub object: S3Object,
    pub version: ObjectVersion,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NewS3ObjectLock {
    pub retention: Option<ObjectRetention>,
    pub legal_hold: Option<bool>,
}

impl NewS3ObjectLock {
    #[must_use]
    pub const fn requested(self) -> bool {
        self.retention.is_some() || self.legal_hold.is_some()
    }
}

#[derive(Clone, Debug)]
pub struct PreparedS3ObjectVersionCommit {
    pub commit: S3ObjectVersionCommit,
    pub version: ObjectVersion,
}

pub struct PrepareClaimedUploadCommitRequest<'a> {
    pub intent: &'a UploadIntent,
    pub lease_token: &'a str,
    pub entity_tag: &'a EntityTag,
    pub checksum: &'a Checksum,
    pub size_bytes: u64,
    pub created_by: &'a str,
    pub source_protocol: SourceProtocol,
}

#[derive(Clone, Debug)]
pub struct S3ObjectRequest {
    pub application_id: ApplicationId,
    pub bucket_name: String,
    pub object_key: String,
    pub version_id: Option<S3VersionId>,
}

#[derive(Clone, Debug)]
pub struct DeleteObjectRequest {
    pub application_id: ApplicationId,
    pub bucket_name: String,
    pub object_key: String,
    pub version_id: Option<S3VersionId>,
    pub bypass_governance: bool,
    pub deleted_by: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeleteObjectReceipt {
    pub version_id: Option<S3VersionId>,
    pub delete_marker: bool,
}

#[derive(Clone, Debug)]
pub struct S3HeadObjectReceipt {
    pub object: S3Object,
    pub version: ObjectVersion,
}

#[derive(Clone, Debug)]
pub struct S3GetObjectReceipt {
    pub head: S3HeadObjectReceipt,
    pub content: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct ListObjectVersionsRequest {
    pub application_id: ApplicationId,
    pub bucket_name: String,
    pub object_key: String,
}

#[derive(Clone, Debug)]
pub struct PutObjectRetentionRequest {
    pub object: S3ObjectRequest,
    pub retention: ObjectRetention,
    pub bypass_governance: bool,
}

#[derive(Clone, Debug)]
pub struct PutObjectLegalHoldRequest {
    pub object: S3ObjectRequest,
    pub legal_hold: bool,
}

#[derive(Debug, Error)]
pub enum S3ObjectServiceError {
    #[error("S3 bucket was not found")]
    BucketNotFound,

    #[error("S3 object was not found")]
    ObjectNotFound,

    #[error("S3 object version was not found")]
    VersionNotFound,

    #[error("the requested version is a delete marker: {version_id}")]
    DeleteMarker {
        version_id: S3VersionId,
        is_current: bool,
    },

    #[error("upload intent was not found")]
    UploadIntentNotFound,

    #[error("upload intent does not belong to the application")]
    UploadIntentOwnershipMismatch,

    #[error("upload intent facts do not match the completed stream")]
    UploadIntentFactsMismatch,

    #[error("upload intent is in invalid state {0:?}")]
    InvalidUploadIntentState(UploadIntentState),

    #[error("upload intent completion is already in progress")]
    CompletionInProgress,

    #[error("S3 GC grace must not be negative")]
    InvalidGcGrace,

    #[error("S3 object deletion is blocked by Object Lock: {0:?}")]
    DeleteLocked(S3DeleteLockReason),

    #[error("Object Lock is not enabled for the bucket")]
    ObjectLockNotEnabled,

    #[error("object retention configuration was not found")]
    ObjectRetentionNotFound,

    #[error("object retention update is denied by Object Lock: {0:?}")]
    RetentionUpdateLocked(S3DeleteLockReason),

    #[error("object retention date must be in the future")]
    InvalidObjectRetention,

    #[error("failed to promote the staged object: {0}")]
    Promotion(ObjectStoreError),

    #[error(
        "failed to release the upload intent after promotion failed; promotion: {promotion}; release: {release}"
    )]
    PromotionRelease {
        promotion: ObjectStoreError,
        release: RepositoryError,
    },

    #[error("promotion result could not be determined; promotion: {promotion}; probe: {probe}")]
    PromotionUncertain {
        promotion: ObjectStoreError,
        probe: ObjectStoreError,
    },

    #[error(transparent)]
    Model(#[from] S3ModelError),

    #[error(transparent)]
    Repository(#[from] RepositoryError),

    #[error(transparent)]
    Storage(#[from] ObjectStoreError),
}

pub struct S3ObjectService<B, R, U, S, C> {
    bucket_repository: B,
    object_repository: R,
    upload_intent_repository: U,
    object_store: S,
    clock: C,
    gc_grace: Duration,
}

impl<B, R, U, S, C> S3ObjectService<B, R, U, S, C>
where
    B: S3BucketRepository,
    R: S3ObjectRepository,
    U: S3UploadIntentRepository,
    S: ObjectStore,
    C: Clock,
{
    #[must_use]
    pub fn new(
        bucket_repository: B,
        object_repository: R,
        upload_intent_repository: U,
        object_store: S,
        clock: C,
    ) -> Self {
        Self {
            bucket_repository,
            object_repository,
            upload_intent_repository,
            object_store,
            clock,
            gc_grace: Duration::seconds(DEFAULT_S3_GC_GRACE_SECONDS),
        }
    }

    pub fn with_gc_grace(mut self, gc_grace: Duration) -> Result<Self, S3ObjectServiceError> {
        if gc_grace.is_negative() {
            return Err(S3ObjectServiceError::InvalidGcGrace);
        }
        self.gc_grace = gc_grace;
        Ok(self)
    }

    /// Persists the durable upload owner before the caller learns the staging
    /// key. The transport may only start streaming after this method returns.
    pub async fn begin_put(
        &self,
        request: &BeginPutObjectRequest,
    ) -> Result<BeginPutObjectReceipt, S3ObjectServiceError> {
        let bucket = self
            .bucket_repository
            .find_s3_bucket(request.application_id, &request.bucket_name)
            .await?
            .ok_or(S3ObjectServiceError::BucketNotFound)?;
        let now = self.clock.now();
        let intent_id = UploadIntentId::new();
        let proposed_version_id = ObjectVersionId::new();
        let temporary_storage_key = temporary_storage_key(intent_id, proposed_version_id);
        let final_storage_key =
            final_storage_key(request.application_id, bucket.id(), proposed_version_id);
        let expires_at = request
            .expires_at
            .unwrap_or(now + Duration::seconds(DEFAULT_S3_UPLOAD_INTENT_TTL_SECONDS));
        let intent = UploadIntent::new(
            intent_id,
            request.application_id,
            bucket.id(),
            &request.object_key,
            proposed_version_id,
            self.object_store.backend_name(),
            temporary_storage_key,
            final_storage_key,
            request.expected_size_bytes,
            request.content_type.clone(),
            request.user_metadata.clone(),
            expires_at,
            now,
        )?;

        self.upload_intent_repository
            .create_upload_intent(&intent)
            .await?;
        Ok(BeginPutObjectReceipt { intent })
    }

    /// Aborts a request that failed before promotion and durably schedules its
    /// deterministic staging key for idempotent cleanup.
    pub async fn abort_staged_put(
        &self,
        request: &AbortStagedPutRequest,
    ) -> Result<UploadIntent, S3ObjectServiceError> {
        let intent = self
            .upload_intent_repository
            .find_upload_intent(request.intent_id)
            .await?
            .ok_or(S3ObjectServiceError::UploadIntentNotFound)?;
        if intent.application_id() != request.application_id {
            return Err(S3ObjectServiceError::UploadIntentOwnershipMismatch);
        }
        match intent.state() {
            UploadIntentState::Aborted | UploadIntentState::Expired => return Ok(intent),
            UploadIntentState::Staging | UploadIntentState::Ready => {}
            state => return Err(S3ObjectServiceError::InvalidUploadIntentState(state)),
        }
        let now = self.clock.now();
        let task = NewStorageGcTask {
            id: StorageGcTaskId::new(),
            application_id: intent.application_id(),
            bucket_id: intent.bucket_id(),
            object_version_id: None,
            upload_intent_id: Some(intent.id()),
            multipart_upload_id: None,
            storage_backend: intent.storage_backend().to_owned(),
            storage_key: intent.temporary_storage_key().to_owned(),
            reason: StorageGcReason::AbortedUploadIntent,
            not_before: now,
            max_attempts: DEFAULT_S3_GC_MAX_ATTEMPTS,
            created_at: now,
        };
        self.upload_intent_repository
            .abort_upload_intent_and_enqueue_gc(intent.id(), task, now)
            .await
            .map_err(Into::into)
    }

    /// Freezes streaming facts, claims a fenced lease, promotes the immutable
    /// payload, then atomically commits object history and the intent.
    ///
    /// A failed promotion releases the lease when the final key is known not
    /// to exist. Any failure after a successful/observed promotion deliberately
    /// leaves the intent in `Committing` for reconciliation.
    pub async fn complete_put(
        &self,
        request: &CompletePutObjectRequest,
    ) -> Result<CompletePutObjectReceipt, S3ObjectServiceError> {
        self.complete_put_with_object_lock(request, NewS3ObjectLock::default())
            .await
    }

    pub async fn complete_put_with_object_lock(
        &self,
        request: &CompletePutObjectRequest,
        object_lock: NewS3ObjectLock,
    ) -> Result<CompletePutObjectReceipt, S3ObjectServiceError> {
        let checksum = Checksum::sha256_hex(&request.streamed.sha256)?;
        let entity_tag = EntityTag::new(&request.streamed.md5)?;
        let intent = self
            .upload_intent_repository
            .find_upload_intent(request.intent_id)
            .await?
            .ok_or(S3ObjectServiceError::UploadIntentNotFound)?;
        if intent.application_id() != request.application_id {
            return Err(S3ObjectServiceError::UploadIntentOwnershipMismatch);
        }
        if object_lock.requested()
            && !self
                .find_bucket_by_id(&intent)
                .await?
                .configuration()
                .object_lock_enabled()
        {
            return Err(S3ObjectServiceError::ObjectLockNotEnabled);
        }

        let ready = match intent.state() {
            UploadIntentState::Staging => {
                self.upload_intent_repository
                    .complete_upload_intent_staging(
                        intent.id(),
                        &entity_tag,
                        &checksum,
                        request.streamed.size,
                        self.clock.now(),
                    )
                    .await?
            }
            UploadIntentState::Ready => {
                ensure_ready_facts(&intent, &entity_tag, &checksum, request.streamed.size)?;
                intent
            }
            UploadIntentState::Committed => {
                ensure_ready_facts(&intent, &entity_tag, &checksum, request.streamed.size)?;
                return self.committed_receipt(&intent).await;
            }
            UploadIntentState::Committing => {
                return Err(S3ObjectServiceError::CompletionInProgress);
            }
            state => return Err(S3ObjectServiceError::InvalidUploadIntentState(state)),
        };

        let lease_token = format!("s3-put-{}", UploadIntentId::new());
        let now = self.clock.now();
        let committing = self
            .upload_intent_repository
            .claim_upload_intent(
                ready.id(),
                &lease_token,
                now + Duration::seconds(DEFAULT_S3_UPLOAD_LEASE_SECONDS),
                now,
            )
            .await?;

        self.promote_claimed_upload_intent(&committing, &lease_token)
            .await?;
        let prepared = self
            .prepare_claimed_upload_commit_with_object_lock(
                PrepareClaimedUploadCommitRequest {
                    intent: &committing,
                    lease_token: &lease_token,
                    entity_tag: &entity_tag,
                    checksum: &checksum,
                    size_bytes: request.streamed.size,
                    created_by: &request.created_by,
                    source_protocol: request.source_protocol,
                },
                object_lock,
            )
            .await?;
        let version_id = prepared.version.id();
        let object = self
            .upload_intent_repository
            .commit_upload_intent(committing.id(), &lease_token, prepared.commit)
            .await?;
        let version = self
            .object_repository
            .find_s3_object_version_by_id(version_id)
            .await?
            .filter(|version| {
                version.object_id() == object.id()
                    && version.application_id() == request.application_id
                    && version.bucket_id() == object.bucket_id()
            })
            .ok_or_else(|| {
                RepositoryError::Invariant("committed object-lock version is missing".into())
            })?;
        Ok(CompletePutObjectReceipt { object, version })
    }

    /// Promotes a leased UploadIntent without changing its durable ownership.
    /// Multipart completion uses this after attaching the same fencing token to
    /// the multipart row.
    pub async fn promote_claimed_upload_intent(
        &self,
        intent: &UploadIntent,
        lease_token: &str,
    ) -> Result<(), S3ObjectServiceError> {
        if intent.state() != UploadIntentState::Committing
            || intent.lease_token() != Some(lease_token)
        {
            return Err(S3ObjectServiceError::InvalidUploadIntentState(
                intent.state(),
            ));
        }
        self.promote(intent, lease_token).await
    }

    /// Builds the single canonical object-version commit used by regular Put
    /// and multipart completion. Promotion must already have made the final
    /// storage key observable.
    pub async fn prepare_claimed_upload_commit(
        &self,
        request: PrepareClaimedUploadCommitRequest<'_>,
    ) -> Result<PreparedS3ObjectVersionCommit, S3ObjectServiceError> {
        self.prepare_claimed_upload_commit_with_object_lock(request, NewS3ObjectLock::default())
            .await
    }

    async fn prepare_claimed_upload_commit_with_object_lock(
        &self,
        request: PrepareClaimedUploadCommitRequest<'_>,
        object_lock: NewS3ObjectLock,
    ) -> Result<PreparedS3ObjectVersionCommit, S3ObjectServiceError> {
        let PrepareClaimedUploadCommitRequest {
            intent,
            lease_token,
            entity_tag,
            checksum,
            size_bytes,
            created_by,
            source_protocol,
        } = request;
        if intent.state() != UploadIntentState::Committing
            || intent.lease_token() != Some(lease_token)
        {
            return Err(S3ObjectServiceError::InvalidUploadIntentState(
                intent.state(),
            ));
        }
        ensure_ready_facts(intent, entity_tag, checksum, size_bytes)?;

        let bucket = self.find_bucket_by_id(intent).await?;
        if object_lock.requested() && !bucket.configuration().object_lock_enabled() {
            return Err(S3ObjectServiceError::ObjectLockNotEnabled);
        }
        let existing_object = self
            .object_repository
            .find_s3_object(
                intent.application_id(),
                intent.bucket_id(),
                intent.object_key(),
            )
            .await?;
        let versioning_status = bucket.configuration().versioning_status();
        let replaced_null = if versioning_status == VersioningStatus::Enabled {
            None
        } else if let Some(object) = &existing_object {
            self.object_repository
                .list_s3_object_versions(object.id())
                .await?
                .into_iter()
                .find(ObjectVersion::is_null_version)
        } else {
            None
        };
        let (target, object_id, generation) = match existing_object {
            Some(object) => {
                let generation = object.generation().checked_add(1).ok_or_else(|| {
                    RepositoryError::Invariant("object generation overflow".into())
                })?;
                (
                    S3ObjectCommitTarget::Append {
                        object_id: object.id(),
                        expected_generation: object.generation(),
                    },
                    object.id(),
                    generation,
                )
            }
            None => {
                let object = S3Object::new(
                    ObjectId::new(),
                    intent.application_id(),
                    intent.bucket_id(),
                    intent.object_key(),
                    self.clock.now(),
                )?;
                let object_id = object.id();
                (S3ObjectCommitTarget::Create(object), object_id, 1)
            }
        };
        let (external_version_id, is_null_version) =
            external_version_id(versioning_status, intent.proposed_version_id())?;
        let provider = self.object_store.head(intent.final_storage_key()).await?;
        let stored = StoredObjectVersion::new(
            intent.storage_backend(),
            intent.final_storage_key(),
            provider.etag.map(EntityTag::new).transpose()?,
            provider.version,
            entity_tag.clone(),
            size_bytes,
            intent.content_type().map(str::to_owned),
            intent.user_metadata().clone(),
            Some(checksum.clone()),
        )?;
        let committed_at = self.clock.now();
        let version = ObjectVersion::new_object(
            intent.proposed_version_id(),
            object_id,
            intent.application_id(),
            intent.bucket_id(),
            external_version_id,
            generation,
            is_null_version,
            ObjectVersionState::Committed,
            stored,
            None,
            false,
            created_by,
            source_protocol,
            committed_at,
        )?;
        let commit = S3ObjectVersionCommit {
            target,
            version: version.clone(),
            committed_at,
            replaced_null_version_id: replaced_null.as_ref().map(ObjectVersion::id),
            gc_tasks: null_replacement_gc_tasks(
                replaced_null.as_ref(),
                committed_at,
                self.gc_grace,
            ),
            requested_retention: object_lock.retention,
            requested_legal_hold: object_lock.legal_hold,
        };
        Ok(PreparedS3ObjectVersionCommit { commit, version })
    }
    pub async fn get(
        &self,
        request: &S3ObjectRequest,
    ) -> Result<S3GetObjectReceipt, S3ObjectServiceError> {
        let head = self.resolve_visible_version(request).await?;
        let ObjectVersionPayload::Object(payload) = head.version.payload() else {
            unreachable!("delete markers are rejected by resolve_visible_version")
        };
        let content = self.object_store.read(payload.storage_key()).await?;
        Ok(S3GetObjectReceipt { head, content })
    }

    pub async fn head(
        &self,
        request: &S3ObjectRequest,
    ) -> Result<S3HeadObjectReceipt, S3ObjectServiceError> {
        self.resolve_visible_version(request).await
    }

    pub async fn get_object_lock(
        &self,
        request: &S3ObjectRequest,
    ) -> Result<S3HeadObjectReceipt, S3ObjectServiceError> {
        let bucket = self
            .bucket_repository
            .find_s3_bucket(request.application_id, &request.bucket_name)
            .await?
            .ok_or(S3ObjectServiceError::BucketNotFound)?;
        if !bucket.configuration().object_lock_enabled() {
            return Err(S3ObjectServiceError::ObjectLockNotEnabled);
        }
        self.resolve_visible_version(request).await
    }

    pub async fn put_object_retention(
        &self,
        request: &PutObjectRetentionRequest,
    ) -> Result<ObjectVersion, S3ObjectServiceError> {
        self.put_object_lock(
            &request.object,
            S3ObjectLockMutation::Retention {
                retention: request.retention,
                bypass_governance: request.bypass_governance,
            },
        )
        .await
    }

    pub async fn put_object_legal_hold(
        &self,
        request: &PutObjectLegalHoldRequest,
    ) -> Result<ObjectVersion, S3ObjectServiceError> {
        self.put_object_lock(
            &request.object,
            S3ObjectLockMutation::LegalHold(request.legal_hold),
        )
        .await
    }

    async fn put_object_lock(
        &self,
        request: &S3ObjectRequest,
        mutation: S3ObjectLockMutation,
    ) -> Result<ObjectVersion, S3ObjectServiceError> {
        let bucket = self
            .bucket_repository
            .find_s3_bucket(request.application_id, &request.bucket_name)
            .await?
            .ok_or(S3ObjectServiceError::BucketNotFound)?;
        let outcome = self
            .object_repository
            .put_s3_object_lock(&PutS3ObjectLockCommand {
                application_id: request.application_id,
                bucket_id: bucket.id(),
                object_key: request.object_key.clone(),
                version_id: request.version_id.clone(),
                mutation,
                updated_at: self.clock.now(),
            })
            .await?;
        match outcome {
            PutS3ObjectLockOutcome::Updated(version) => Ok(version),
            PutS3ObjectLockOutcome::ObjectNotFound => Err(S3ObjectServiceError::ObjectNotFound),
            PutS3ObjectLockOutcome::VersionNotFound => Err(S3ObjectServiceError::VersionNotFound),
            PutS3ObjectLockOutcome::DeleteMarker {
                version_id,
                is_current,
            } => Err(S3ObjectServiceError::DeleteMarker {
                version_id,
                is_current,
            }),
            PutS3ObjectLockOutcome::ObjectLockNotEnabled => {
                Err(S3ObjectServiceError::ObjectLockNotEnabled)
            }
            PutS3ObjectLockOutcome::RetentionLocked(reason) => {
                Err(S3ObjectServiceError::RetentionUpdateLocked(reason))
            }
            PutS3ObjectLockOutcome::InvalidRetention => {
                Err(S3ObjectServiceError::InvalidObjectRetention)
            }
        }
    }

    pub async fn delete(
        &self,
        request: &DeleteObjectRequest,
    ) -> Result<DeleteObjectReceipt, S3ObjectServiceError> {
        let bucket = self
            .bucket_repository
            .find_s3_bucket(request.application_id, &request.bucket_name)
            .await?
            .ok_or(S3ObjectServiceError::BucketNotFound)?;
        let now = self.clock.now();
        let delete_marker_id = ObjectVersionId::new();
        let outcome = self
            .object_repository
            .delete_s3_object(&DeleteS3ObjectCommand {
                application_id: request.application_id,
                bucket_id: bucket.id(),
                object_key: request.object_key.clone(),
                version_id: request.version_id.clone(),
                bypass_governance: request.bypass_governance,
                deleted_by: request.deleted_by.clone(),
                deleted_at: now,
                delete_marker_id,
                delete_marker_version_id: S3VersionId::new(delete_marker_id.to_string())?,
                gc_task_id: StorageGcTaskId::new(),
                gc_not_before: now + self.gc_grace,
                gc_max_attempts: DEFAULT_S3_GC_MAX_ATTEMPTS,
            })
            .await?;
        match outcome {
            DeleteS3ObjectOutcome::Deleted(deleted) => Ok(DeleteObjectReceipt {
                version_id: deleted.version_id,
                delete_marker: deleted.delete_marker,
            }),
            DeleteS3ObjectOutcome::NoOp => Ok(DeleteObjectReceipt {
                version_id: None,
                delete_marker: false,
            }),
            DeleteS3ObjectOutcome::VersionNotFound => Err(S3ObjectServiceError::VersionNotFound),
            DeleteS3ObjectOutcome::Locked(reason) => {
                Err(S3ObjectServiceError::DeleteLocked(reason))
            }
        }
    }
    pub async fn list_versions(
        &self,
        request: &ListObjectVersionsRequest,
    ) -> Result<Vec<ObjectVersion>, S3ObjectServiceError> {
        let bucket = self
            .bucket_repository
            .find_s3_bucket(request.application_id, &request.bucket_name)
            .await?
            .ok_or(S3ObjectServiceError::BucketNotFound)?;
        let Some(object) = self
            .object_repository
            .find_s3_object(request.application_id, bucket.id(), &request.object_key)
            .await?
        else {
            return Ok(Vec::new());
        };
        self.object_repository
            .list_s3_object_versions(object.id())
            .await
            .map_err(Into::into)
    }

    async fn find_bucket_by_id(
        &self,
        intent: &UploadIntent,
    ) -> Result<S3Bucket, S3ObjectServiceError> {
        self.bucket_repository
            .list_s3_buckets(intent.application_id())
            .await?
            .into_iter()
            .find(|bucket| bucket.id() == intent.bucket_id())
            .ok_or(S3ObjectServiceError::BucketNotFound)
    }

    async fn committed_receipt(
        &self,
        intent: &UploadIntent,
    ) -> Result<CompletePutObjectReceipt, S3ObjectServiceError> {
        let object_id = intent.committed_object_id().ok_or_else(|| {
            RepositoryError::Invariant("committed intent has no object ID".into())
        })?;
        let version_id = intent.committed_version_id().ok_or_else(|| {
            RepositoryError::Invariant("committed intent has no object version ID".into())
        })?;
        let object = self
            .object_repository
            .find_s3_object(
                intent.application_id(),
                intent.bucket_id(),
                intent.object_key(),
            )
            .await?
            .filter(|object| object.id() == object_id)
            .ok_or_else(|| {
                RepositoryError::Invariant("committed intent object is missing".into())
            })?;
        let version = self
            .object_repository
            .find_s3_object_version_by_id(version_id)
            .await?
            .filter(|version| {
                version.object_id() == object_id
                    && version.application_id() == intent.application_id()
                    && version.bucket_id() == intent.bucket_id()
            })
            .ok_or_else(|| {
                RepositoryError::Invariant("committed intent version is missing".into())
            })?;
        Ok(CompletePutObjectReceipt { object, version })
    }

    async fn promote(
        &self,
        intent: &UploadIntent,
        lease_token: &str,
    ) -> Result<(), S3ObjectServiceError> {
        match self.object_store.exists(intent.final_storage_key()).await {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => return Err(S3ObjectServiceError::Storage(error)),
        }

        let promotion = self
            .object_store
            .commit_temporary(intent.temporary_storage_key(), intent.final_storage_key())
            .await;
        let Err(promotion) = promotion else {
            return Ok(());
        };

        match self.object_store.exists(intent.final_storage_key()).await {
            Ok(true) => Ok(()),
            Ok(false) => {
                if let Err(release) = self
                    .upload_intent_repository
                    .release_upload_intent(intent.id(), lease_token, self.clock.now())
                    .await
                {
                    return Err(S3ObjectServiceError::PromotionRelease { promotion, release });
                }
                Err(S3ObjectServiceError::Promotion(promotion))
            }
            Err(probe) => Err(S3ObjectServiceError::PromotionUncertain { promotion, probe }),
        }
    }

    async fn resolve_visible_version(
        &self,
        request: &S3ObjectRequest,
    ) -> Result<S3HeadObjectReceipt, S3ObjectServiceError> {
        let bucket = self
            .bucket_repository
            .find_s3_bucket(request.application_id, &request.bucket_name)
            .await?
            .ok_or(S3ObjectServiceError::BucketNotFound)?;
        let object = self
            .object_repository
            .find_s3_object(request.application_id, bucket.id(), &request.object_key)
            .await?
            .ok_or(S3ObjectServiceError::ObjectNotFound)?;
        let exact = request.version_id.is_some();
        let version = match &request.version_id {
            Some(version_id) => self
                .object_repository
                .find_s3_object_version(object.id(), version_id)
                .await?
                .ok_or(S3ObjectServiceError::VersionNotFound)?,
            None => self
                .object_repository
                .find_current_s3_object_version(object.id())
                .await?
                .ok_or(S3ObjectServiceError::ObjectNotFound)?,
        };
        if version.state() != ObjectVersionState::Committed {
            return Err(if exact {
                S3ObjectServiceError::VersionNotFound
            } else {
                S3ObjectServiceError::ObjectNotFound
            });
        }
        if version.is_delete_marker() {
            return Err(S3ObjectServiceError::DeleteMarker {
                version_id: version.external_version_id().clone(),
                is_current: object.current_version_id() == Some(version.id()),
            });
        }
        Ok(S3HeadObjectReceipt { object, version })
    }
}

fn null_replacement_gc_tasks(
    replaced: Option<&ObjectVersion>,
    committed_at: OffsetDateTime,
    gc_grace: Duration,
) -> Vec<NewStorageGcTask> {
    let Some(replaced) = replaced else {
        return Vec::new();
    };
    let ObjectVersionPayload::Object(payload) = replaced.payload() else {
        return Vec::new();
    };
    vec![NewStorageGcTask {
        id: StorageGcTaskId::new(),
        application_id: replaced.application_id(),
        bucket_id: replaced.bucket_id(),
        object_version_id: Some(replaced.id()),
        upload_intent_id: None,
        multipart_upload_id: None,
        storage_backend: payload.storage_backend().to_owned(),
        storage_key: payload.storage_key().to_owned(),
        reason: StorageGcReason::ReplacedNullVersion,
        not_before: committed_at + gc_grace,
        max_attempts: DEFAULT_S3_GC_MAX_ATTEMPTS,
        created_at: committed_at,
    }]
}
fn ensure_ready_facts(
    intent: &UploadIntent,
    entity_tag: &EntityTag,
    checksum: &Checksum,
    size: u64,
) -> Result<(), S3ObjectServiceError> {
    if intent.entity_tag() == Some(entity_tag)
        && intent.checksum() == Some(checksum)
        && intent.size_bytes() == Some(size)
    {
        Ok(())
    } else {
        Err(S3ObjectServiceError::UploadIntentFactsMismatch)
    }
}

fn external_version_id(
    status: VersioningStatus,
    proposed_version_id: ObjectVersionId,
) -> Result<(S3VersionId, bool), S3ModelError> {
    match status {
        VersioningStatus::Enabled => {
            Ok((S3VersionId::new(proposed_version_id.to_string())?, false))
        }
        VersioningStatus::Unversioned | VersioningStatus::Suspended => {
            Ok((S3VersionId::new("null")?, true))
        }
    }
}

fn temporary_storage_key(
    intent_id: UploadIntentId,
    proposed_version_id: ObjectVersionId,
) -> String {
    format!("s3/staging/{intent_id}/{proposed_version_id}")
}

fn final_storage_key(
    application_id: ApplicationId,
    bucket_id: BucketId,
    proposed_version_id: ObjectVersionId,
) -> String {
    format!("s3/objects/{application_id}/{bucket_id}/{proposed_version_id}")
}
