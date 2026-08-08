use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ops::Range,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use futures::executor::block_on;
use mediahub_core::{
    ApplicationId, BucketId, BucketS3Configuration, Checksum, DefaultRetention,
    DefaultRetentionPeriod, EntityTag, NewStorageGcTask, ObjectId, ObjectRetention,
    ObjectRetentionUpdateError, ObjectVersion, ObjectVersionId, ObjectVersionPayload,
    ObjectVersionState, PersistedBucketS3Configuration, PersistedS3Bucket, PersistedS3Object,
    PersistedUploadIntent, RetentionMode, S3Bucket, S3LifecycleConfiguration, S3Object,
    S3VersionId, SourceProtocol, StorageGcReason, StoredObjectVersion, UploadIntent,
    UploadIntentId, UploadIntentState, VersioningStatus,
};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

use crate::{
    BeginPutObjectReceipt, BeginPutObjectRequest, CompletePutObjectRequest, ComposedObject,
    DeleteObjectRequest, DeleteS3ObjectCommand, DeleteS3ObjectOutcome, DeletedS3ObjectVersion,
    FixedClock, InMemoryObjectStore, ListObjectVersionsRequest, NewS3ObjectLock, ObjectMetadata,
    ObjectPage, ObjectStore, ObjectStoreError, PutObjectLegalHoldRequest,
    PutObjectRetentionRequest, PutS3ObjectLockCommand, PutS3ObjectLockOutcome, RepositoryError,
    S3BucketRepository, S3DeleteLockReason, S3ObjectCommitTarget, S3ObjectListItem,
    S3ObjectListQuery, S3ObjectLockMutation, S3ObjectPage, S3ObjectRepository, S3ObjectRequest,
    S3ObjectService, S3ObjectServiceError, S3ObjectVersionCommit, S3ObjectVersionRead,
    S3UploadIntentRepository, StreamedObject,
};

#[derive(Default)]
struct State {
    buckets: Vec<S3Bucket>,
    intents: HashMap<UploadIntentId, UploadIntent>,
    objects: HashMap<ObjectId, S3Object>,
    versions: HashMap<ObjectId, Vec<ObjectVersion>>,
    superseded_versions: HashSet<ObjectVersionId>,
    deleted_versions: HashSet<ObjectVersionId>,
    gc_tasks: Vec<NewStorageGcTask>,
    fail_commit: bool,
}

#[derive(Clone, Default)]
struct MemoryS3Repository {
    state: Arc<Mutex<State>>,
    events: Arc<Mutex<Vec<String>>>,
}

impl MemoryS3Repository {
    fn new(bucket: S3Bucket, events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                buckets: vec![bucket],
                ..State::default()
            })),
            events,
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("memory S3 repository lock")
    }

    fn record(&self, event: &str) {
        self.events
            .lock()
            .expect("memory S3 event lock")
            .push(event.to_owned());
    }

    fn intent(&self, id: UploadIntentId) -> UploadIntent {
        self.state()
            .intents
            .get(&id)
            .cloned()
            .expect("intent exists")
    }

    fn fail_next_commit(&self) {
        self.state().fail_commit = true;
    }

    fn seed_object(&self, object: S3Object, versions: Vec<ObjectVersion>) {
        let mut state = self.state();
        state.versions.insert(object.id(), versions);
        state.objects.insert(object.id(), object);
    }
}

#[async_trait]
impl S3BucketRepository for MemoryS3Repository {
    async fn list_s3_buckets(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<S3Bucket>, RepositoryError> {
        Ok(self
            .state()
            .buckets
            .iter()
            .filter(|bucket| bucket.application_id() == application_id)
            .cloned()
            .collect())
    }

    async fn find_s3_bucket(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<S3Bucket>, RepositoryError> {
        Ok(self
            .state()
            .buckets
            .iter()
            .find(|bucket| bucket.application_id() == application_id && bucket.name() == name)
            .cloned())
    }

    async fn create_s3_bucket(&self, bucket: &S3Bucket) -> Result<(), RepositoryError> {
        self.state().buckets.push(bucket.clone());
        Ok(())
    }

    async fn delete_s3_bucket(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<bool, RepositoryError> {
        let mut state = self.state();
        let before = state.buckets.len();
        state
            .buckets
            .retain(|bucket| bucket.application_id() != application_id || bucket.name() != name);
        Ok(before != state.buckets.len())
    }

    async fn get_s3_bucket_location(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        Ok(self
            .find_s3_bucket(application_id, name)
            .await?
            .map(|bucket| bucket.configuration().region().to_owned()))
    }

    async fn get_s3_bucket_configuration(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<BucketS3Configuration>, RepositoryError> {
        Ok(self
            .find_s3_bucket(application_id, name)
            .await?
            .map(|bucket| bucket.configuration().clone()))
    }

    async fn get_s3_bucket_versioning(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<VersioningStatus>, RepositoryError> {
        Ok(self
            .find_s3_bucket(application_id, name)
            .await?
            .map(|bucket| bucket.configuration().versioning_status()))
    }

    async fn set_s3_bucket_versioning(
        &self,
        application_id: ApplicationId,
        name: &str,
        status: VersioningStatus,
        updated_at: OffsetDateTime,
    ) -> Result<BucketS3Configuration, RepositoryError> {
        let mut state = self.state();
        let bucket = state
            .buckets
            .iter_mut()
            .find(|bucket| bucket.application_id() == application_id && bucket.name() == name)
            .ok_or(RepositoryError::NotFound)?;
        let mut configuration = bucket.configuration().clone();
        configuration
            .transition_versioning(status, updated_at)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        *bucket = S3Bucket::from_persistence(PersistedS3Bucket {
            id: bucket.id(),
            application_id: bucket.application_id(),
            name: bucket.name().to_owned(),
            configuration: PersistedBucketS3Configuration {
                region: configuration.region().to_owned(),
                versioning_status: configuration.versioning_status(),
                object_lock_enabled: configuration.object_lock_enabled(),
                default_retention: configuration.default_retention(),
                lifecycle_configuration: configuration.lifecycle_configuration().cloned(),
                revision: configuration.revision(),
                updated_at: configuration.updated_at(),
            },
            created_at: bucket.created_at(),
        })
        .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        Ok(configuration)
    }

    async fn replace_s3_bucket_object_lock(
        &self,
        application_id: ApplicationId,
        name: &str,
        default_retention: Option<DefaultRetention>,
        updated_at: OffsetDateTime,
    ) -> Result<BucketS3Configuration, RepositoryError> {
        let mut state = self.state();
        let bucket = state
            .buckets
            .iter_mut()
            .find(|bucket| bucket.application_id() == application_id && bucket.name() == name)
            .ok_or(RepositoryError::NotFound)?;
        let mut configuration = bucket.configuration().clone();
        configuration
            .replace_object_lock_configuration(default_retention, updated_at)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        *bucket = S3Bucket::from_persistence(PersistedS3Bucket {
            id: bucket.id(),
            application_id: bucket.application_id(),
            name: bucket.name().to_owned(),
            configuration: PersistedBucketS3Configuration {
                region: configuration.region().to_owned(),
                versioning_status: configuration.versioning_status(),
                object_lock_enabled: configuration.object_lock_enabled(),
                default_retention: configuration.default_retention(),
                lifecycle_configuration: configuration.lifecycle_configuration().cloned(),
                revision: configuration.revision(),
                updated_at: configuration.updated_at(),
            },
            created_at: bucket.created_at(),
        })
        .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        Ok(configuration)
    }

    async fn replace_s3_bucket_lifecycle(
        &self,
        _application_id: ApplicationId,
        _name: &str,
        _lifecycle_configuration: Option<S3LifecycleConfiguration>,
        _updated_at: OffsetDateTime,
    ) -> Result<BucketS3Configuration, RepositoryError> {
        unreachable!("not used by ObjectService tests")
    }
}

#[async_trait]
impl S3ObjectRepository for MemoryS3Repository {
    async fn find_s3_object(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        key: &str,
    ) -> Result<Option<S3Object>, RepositoryError> {
        Ok(self
            .state()
            .objects
            .values()
            .find(|object| {
                object.application_id() == application_id
                    && object.bucket_id() == bucket_id
                    && object.key() == key
            })
            .cloned())
    }

    async fn list_current_s3_objects(
        &self,
        application_id: ApplicationId,
        query: &S3ObjectListQuery,
    ) -> Result<S3ObjectPage, RepositoryError> {
        query.validate()?;
        if query.limit == 0 {
            return Ok(S3ObjectPage::default());
        }

        let state = self.state();
        let mut entries = BTreeMap::<String, Option<S3ObjectListItem>>::new();
        for object in state.objects.values().filter(|object| {
            object.application_id() == application_id
                && object.bucket_id() == query.bucket_id
                && object.key().starts_with(&query.prefix)
        }) {
            let Some(current_version_id) = object.current_version_id() else {
                continue;
            };
            if state.superseded_versions.contains(&current_version_id)
                || state.deleted_versions.contains(&current_version_id)
            {
                continue;
            }
            let Some(version) = state
                .versions
                .get(&object.id())
                .into_iter()
                .flatten()
                .find(|version| version.id() == current_version_id)
                .filter(|version| {
                    version.state() == ObjectVersionState::Committed
                        && matches!(version.payload(), ObjectVersionPayload::Object(_))
                })
                .cloned()
            else {
                continue;
            };

            if query.delimiter {
                let relative_key = &object.key()[query.prefix.len()..];
                if let Some(slash_index) = relative_key.find('/') {
                    let common_prefix =
                        format!("{}{}", query.prefix, &relative_key[..=slash_index]);
                    entries.entry(common_prefix).or_insert(None);
                    continue;
                }
            }
            entries.insert(
                object.key().to_owned(),
                Some(S3ObjectListItem {
                    key: object.key().to_owned(),
                    version,
                }),
            );
        }

        let mut selected = entries
            .into_iter()
            .filter(|(entry_key, _)| {
                query
                    .start_after
                    .as_deref()
                    .is_none_or(|cursor| entry_key.as_bytes() > cursor.as_bytes())
            })
            .take(query.limit + 1)
            .collect::<Vec<_>>();
        let has_more = selected.len() > query.limit;
        selected.truncate(query.limit);
        let next_cursor = has_more.then(|| {
            selected
                .last()
                .expect("a truncated non-zero page has a final entry")
                .0
                .clone()
        });
        let mut page = S3ObjectPage {
            next_cursor,
            ..S3ObjectPage::default()
        };
        for (entry_key, item) in selected {
            match item {
                Some(item) => page.items.push(item),
                None => page.common_prefixes.push(entry_key),
            }
        }
        Ok(page)
    }

    async fn find_s3_object_version(
        &self,
        object_id: ObjectId,
        version_id: &S3VersionId,
    ) -> Result<Option<ObjectVersion>, RepositoryError> {
        let state = self.state();
        Ok(state.versions.get(&object_id).and_then(|versions| {
            versions
                .iter()
                .find(|version| {
                    version.external_version_id() == version_id
                        && !state.superseded_versions.contains(&version.id())
                        && !state.deleted_versions.contains(&version.id())
                        && version.state() == ObjectVersionState::Committed
                })
                .cloned()
        }))
    }

    async fn find_s3_object_version_by_id(
        &self,
        version_id: ObjectVersionId,
    ) -> Result<Option<ObjectVersion>, RepositoryError> {
        Ok(self
            .state()
            .versions
            .values()
            .flat_map(|versions| versions.iter())
            .find(|version| version.id() == version_id)
            .cloned())
    }

    async fn find_committed_s3_object_version_for_application(
        &self,
        application_id: ApplicationId,
        version_id: ObjectVersionId,
    ) -> Result<Option<S3ObjectVersionRead>, RepositoryError> {
        let state = self.state();
        let Some(version) = state
            .versions
            .values()
            .flatten()
            .find(|version| {
                version.id() == version_id
                    && version.application_id() == application_id
                    && version.state() == ObjectVersionState::Committed
                    && !version.is_delete_marker()
            })
            .cloned()
        else {
            return Ok(None);
        };
        let Some(object) = state.objects.get(&version.object_id()) else {
            return Err(RepositoryError::Invariant(
                "object version has no logical object".into(),
            ));
        };
        Ok(Some(S3ObjectVersionRead {
            object_key: object.key().to_owned(),
            version,
        }))
    }

    async fn find_current_s3_object_version(
        &self,
        object_id: ObjectId,
    ) -> Result<Option<ObjectVersion>, RepositoryError> {
        let state = self.state();
        let current_id = state
            .objects
            .get(&object_id)
            .and_then(S3Object::current_version_id);
        Ok(current_id.and_then(|current_id| {
            if state.superseded_versions.contains(&current_id)
                || state.deleted_versions.contains(&current_id)
            {
                return None;
            }
            state
                .versions
                .get(&object_id)
                .and_then(|versions| versions.iter().find(|version| version.id() == current_id))
                .cloned()
        }))
    }

    async fn list_s3_object_versions(
        &self,
        object_id: ObjectId,
    ) -> Result<Vec<ObjectVersion>, RepositoryError> {
        let state = self.state();
        Ok(state
            .versions
            .get(&object_id)
            .into_iter()
            .flatten()
            .filter(|version| {
                !state.superseded_versions.contains(&version.id())
                    && !state.deleted_versions.contains(&version.id())
                    && version.state() == ObjectVersionState::Committed
            })
            .cloned()
            .collect())
    }

    async fn delete_s3_object(
        &self,
        command: &DeleteS3ObjectCommand,
    ) -> Result<DeleteS3ObjectOutcome, RepositoryError> {
        if command.deleted_by.is_empty() || command.gc_max_attempts == 0 {
            return Err(RepositoryError::Invariant(
                "S3 delete command contains invalid audit or GC facts".into(),
            ));
        }

        let mut state = self.state();
        let versioning_status = state
            .buckets
            .iter()
            .find(|bucket| {
                bucket.application_id() == command.application_id
                    && bucket.id() == command.bucket_id
            })
            .map(|bucket| bucket.configuration().versioning_status())
            .ok_or(RepositoryError::NotFound)?;
        let object = state
            .objects
            .values()
            .find(|object| {
                object.application_id() == command.application_id
                    && object.bucket_id() == command.bucket_id
                    && object.key() == command.object_key
            })
            .cloned();
        let Some(object) = object else {
            return Ok(if command.version_id.is_some() {
                DeleteS3ObjectOutcome::VersionNotFound
            } else {
                DeleteS3ObjectOutcome::NoOp
            });
        };
        let active_versions = state
            .versions
            .get(&object.id())
            .into_iter()
            .flatten()
            .filter(|version| {
                version.state() == ObjectVersionState::Committed
                    && !state.superseded_versions.contains(&version.id())
                    && !state.deleted_versions.contains(&version.id())
            })
            .cloned()
            .collect::<Vec<_>>();

        if let Some(requested_version_id) = &command.version_id {
            let Some(target) = active_versions
                .iter()
                .find(|version| version.external_version_id() == requested_version_id)
                .cloned()
            else {
                return Ok(DeleteS3ObjectOutcome::VersionNotFound);
            };
            if let Some(reason) = delete_lock_reason(&target, command) {
                return Ok(DeleteS3ObjectOutcome::Locked(reason));
            }
            if let Some(task) = explicit_delete_gc_task(&target, command) {
                enqueue_memory_gc_task(&mut state, task)?;
            }

            if target.is_null_version() {
                state.superseded_versions.insert(target.id());
            } else {
                state.deleted_versions.insert(target.id());
            }
            if object.current_version_id() == Some(target.id()) {
                let next_current = active_versions
                    .iter()
                    .filter(|version| version.id() != target.id())
                    .max_by_key(|version| version.generation())
                    .map(ObjectVersion::id);
                state.objects.insert(
                    object.id(),
                    rebuild_object_head(&object, next_current, command.deleted_at)?,
                );
            }
            return Ok(DeleteS3ObjectOutcome::Deleted(DeletedS3ObjectVersion {
                version_id: Some(target.external_version_id().clone()),
                delete_marker: target.is_delete_marker(),
            }));
        }

        match versioning_status {
            VersioningStatus::Enabled => {
                let generation = object.generation().checked_add(1).ok_or_else(|| {
                    RepositoryError::Invariant("object generation overflow".into())
                })?;
                let marker = ObjectVersion::new_delete_marker(
                    command.delete_marker_id,
                    object.id(),
                    command.application_id,
                    command.bucket_id,
                    command.delete_marker_version_id.clone(),
                    generation,
                    false,
                    &command.deleted_by,
                    SourceProtocol::S3,
                    command.deleted_at,
                )
                .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
                let advanced = object
                    .advanced_to(&marker, command.deleted_at)
                    .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
                state.versions.entry(object.id()).or_default().push(marker);
                state.objects.insert(object.id(), advanced);
                Ok(DeleteS3ObjectOutcome::Deleted(DeletedS3ObjectVersion {
                    version_id: Some(command.delete_marker_version_id.clone()),
                    delete_marker: true,
                }))
            }
            VersioningStatus::Suspended => {
                let active_null = active_versions
                    .iter()
                    .find(|version| version.is_null_version())
                    .cloned();
                if let Some(reason) = active_null
                    .as_ref()
                    .and_then(|version| delete_lock_reason(version, command))
                {
                    return Ok(DeleteS3ObjectOutcome::Locked(reason));
                }
                let generation = object.generation().checked_add(1).ok_or_else(|| {
                    RepositoryError::Invariant("object generation overflow".into())
                })?;
                let null_version_id = S3VersionId::new("null")
                    .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
                let marker = ObjectVersion::new_delete_marker(
                    command.delete_marker_id,
                    object.id(),
                    command.application_id,
                    command.bucket_id,
                    null_version_id.clone(),
                    generation,
                    true,
                    &command.deleted_by,
                    SourceProtocol::S3,
                    command.deleted_at,
                )
                .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
                let advanced = object
                    .advanced_to(&marker, command.deleted_at)
                    .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
                if let Some(task) = active_null
                    .as_ref()
                    .and_then(|version| explicit_delete_gc_task(version, command))
                {
                    enqueue_memory_gc_task(&mut state, task)?;
                }
                if let Some(previous) = active_null {
                    state.superseded_versions.insert(previous.id());
                }
                state.versions.entry(object.id()).or_default().push(marker);
                state.objects.insert(object.id(), advanced);
                Ok(DeleteS3ObjectOutcome::Deleted(DeletedS3ObjectVersion {
                    version_id: Some(null_version_id),
                    delete_marker: true,
                }))
            }
            VersioningStatus::Unversioned => {
                let active_null = active_versions
                    .iter()
                    .find(|version| version.is_null_version())
                    .cloned();
                let Some(active_null) = active_null else {
                    return Ok(DeleteS3ObjectOutcome::NoOp);
                };
                if let Some(reason) = delete_lock_reason(&active_null, command) {
                    return Ok(DeleteS3ObjectOutcome::Locked(reason));
                }
                if let Some(task) = explicit_delete_gc_task(&active_null, command) {
                    enqueue_memory_gc_task(&mut state, task)?;
                }
                state.superseded_versions.insert(active_null.id());
                state.objects.insert(
                    object.id(),
                    rebuild_object_head(&object, None, command.deleted_at)?,
                );
                Ok(DeleteS3ObjectOutcome::Deleted(DeletedS3ObjectVersion {
                    version_id: None,
                    delete_marker: false,
                }))
            }
        }
    }

    async fn put_s3_object_lock(
        &self,
        command: &PutS3ObjectLockCommand,
    ) -> Result<PutS3ObjectLockOutcome, RepositoryError> {
        let mut state = self.state();
        let Some(bucket) = state.buckets.iter().find(|bucket| {
            bucket.application_id() == command.application_id && bucket.id() == command.bucket_id
        }) else {
            return Err(RepositoryError::NotFound);
        };
        if !bucket.configuration().object_lock_enabled() {
            return Ok(PutS3ObjectLockOutcome::ObjectLockNotEnabled);
        }
        let Some(object) = state
            .objects
            .values()
            .find(|object| {
                object.application_id() == command.application_id
                    && object.bucket_id() == command.bucket_id
                    && object.key() == command.object_key
            })
            .cloned()
        else {
            return Ok(if command.version_id.is_some() {
                PutS3ObjectLockOutcome::VersionNotFound
            } else {
                PutS3ObjectLockOutcome::ObjectNotFound
            });
        };
        let target_id = match &command.version_id {
            Some(version_id) => state
                .versions
                .get(&object.id())
                .into_iter()
                .flatten()
                .find(|version| {
                    version.external_version_id() == version_id
                        && !state.superseded_versions.contains(&version.id())
                        && !state.deleted_versions.contains(&version.id())
                })
                .map(ObjectVersion::id),
            None => object.current_version_id(),
        };
        let Some(target_id) = target_id else {
            return Ok(if command.version_id.is_some() {
                PutS3ObjectLockOutcome::VersionNotFound
            } else {
                PutS3ObjectLockOutcome::ObjectNotFound
            });
        };
        let versions = state.versions.get_mut(&object.id()).ok_or_else(|| {
            RepositoryError::Invariant("logical object has no version history".into())
        })?;
        let target = versions
            .iter_mut()
            .find(|version| version.id() == target_id)
            .ok_or_else(|| RepositoryError::Invariant("object head version is missing".into()))?;
        if target.is_delete_marker() {
            return Ok(PutS3ObjectLockOutcome::DeleteMarker {
                version_id: target.external_version_id().clone(),
                is_current: object.current_version_id() == Some(target.id()),
            });
        }
        let updated = match command.mutation {
            S3ObjectLockMutation::Retention {
                retention,
                bypass_governance,
            } => {
                match target.with_retention_update(retention, command.updated_at, bypass_governance)
                {
                    Ok(updated) => updated,
                    Err(ObjectRetentionUpdateError::RetainUntilMustBeFuture) => {
                        return Ok(PutS3ObjectLockOutcome::InvalidRetention);
                    }
                    Err(ObjectRetentionUpdateError::ComplianceRetentionLocked) => {
                        return Ok(PutS3ObjectLockOutcome::RetentionLocked(
                            S3DeleteLockReason::ComplianceRetention,
                        ));
                    }
                    Err(ObjectRetentionUpdateError::GovernanceRetentionLocked) => {
                        return Ok(PutS3ObjectLockOutcome::RetentionLocked(
                            S3DeleteLockReason::GovernanceRetention,
                        ));
                    }
                    Err(ObjectRetentionUpdateError::InvalidVersion) => {
                        return Ok(PutS3ObjectLockOutcome::VersionNotFound);
                    }
                }
            }
            S3ObjectLockMutation::LegalHold(legal_hold) => target
                .with_legal_hold_update(legal_hold)
                .map_err(|error| RepositoryError::Invariant(error.to_string()))?,
        };
        *target = updated.clone();
        Ok(PutS3ObjectLockOutcome::Updated(updated))
    }
    async fn create_s3_object_with_version(
        &self,
        _object: S3Object,
        _version: ObjectVersion,
        _updated_at: OffsetDateTime,
    ) -> Result<S3Object, RepositoryError> {
        unreachable!("ObjectService commits through the intent repository")
    }

    async fn append_s3_object_version(
        &self,
        _object_id: ObjectId,
        _expected_generation: u64,
        _version: ObjectVersion,
        _updated_at: OffsetDateTime,
    ) -> Result<S3Object, RepositoryError> {
        unreachable!("ObjectService commits through the intent repository")
    }
}

fn delete_lock_reason(
    version: &ObjectVersion,
    command: &DeleteS3ObjectCommand,
) -> Option<S3DeleteLockReason> {
    if version.legal_hold() {
        return Some(S3DeleteLockReason::LegalHold);
    }
    let retention = version.retention()?;
    if retention.retain_until() <= command.deleted_at {
        return None;
    }
    match retention.mode() {
        RetentionMode::Compliance => Some(S3DeleteLockReason::ComplianceRetention),
        RetentionMode::Governance if !command.bypass_governance => {
            Some(S3DeleteLockReason::GovernanceRetention)
        }
        RetentionMode::Governance => None,
    }
}

fn explicit_delete_gc_task(
    version: &ObjectVersion,
    command: &DeleteS3ObjectCommand,
) -> Option<NewStorageGcTask> {
    let ObjectVersionPayload::Object(payload) = version.payload() else {
        return None;
    };
    Some(NewStorageGcTask {
        id: command.gc_task_id,
        application_id: version.application_id(),
        bucket_id: version.bucket_id(),
        object_version_id: Some(version.id()),
        upload_intent_id: None,
        multipart_upload_id: None,
        storage_backend: payload.storage_backend().to_owned(),
        storage_key: payload.storage_key().to_owned(),
        reason: StorageGcReason::ExplicitDelete,
        not_before: command.gc_not_before,
        max_attempts: command.gc_max_attempts,
        created_at: command.deleted_at,
    })
}

fn enqueue_memory_gc_task(
    state: &mut State,
    task: NewStorageGcTask,
) -> Result<(), RepositoryError> {
    task.validate()
        .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
    if let Some(existing) = state
        .gc_tasks
        .iter()
        .find(|existing| existing.storage_key == task.storage_key)
    {
        let exact_target = existing.application_id == task.application_id
            && existing.bucket_id == task.bucket_id
            && existing.object_version_id == task.object_version_id
            && existing.upload_intent_id.is_none()
            && existing.multipart_upload_id.is_none()
            && existing.storage_backend == task.storage_backend
            && existing.storage_key == task.storage_key
            && existing.reason == StorageGcReason::ExplicitDelete;
        return if exact_target {
            Ok(())
        } else {
            Err(RepositoryError::Conflict)
        };
    }
    state.gc_tasks.push(task);
    Ok(())
}

fn freeze_memory_object_lock(
    buckets: &[S3Bucket],
    commit: &mut S3ObjectVersionCommit,
) -> Result<(), RepositoryError> {
    let bucket = buckets
        .iter()
        .find(|bucket| {
            bucket.application_id() == commit.version.application_id()
                && bucket.id() == commit.version.bucket_id()
        })
        .ok_or(RepositoryError::NotFound)?;
    let requested = commit.requested_retention.is_some() || commit.requested_legal_hold.is_some();
    if requested && !bucket.configuration().object_lock_enabled() {
        return Err(RepositoryError::Conflict);
    }
    let retention = if bucket.configuration().object_lock_enabled() {
        commit.requested_retention.or_else(|| {
            bucket
                .configuration()
                .default_retention()
                .and_then(|value| value.for_object_at(commit.committed_at).ok())
        })
    } else {
        None
    };
    if bucket.configuration().default_retention().is_some()
        && bucket.configuration().object_lock_enabled()
        && retention.is_none()
    {
        return Err(RepositoryError::Invariant(
            "bucket default retention could not be resolved".into(),
        ));
    }
    commit.version = commit
        .version
        .with_initial_object_lock(retention, commit.requested_legal_hold.unwrap_or(false))
        .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
    Ok(())
}

fn rebuild_object_head(
    object: &S3Object,
    current_version_id: Option<ObjectVersionId>,
    updated_at: OffsetDateTime,
) -> Result<S3Object, RepositoryError> {
    S3Object::from_persistence(PersistedS3Object {
        id: object.id(),
        application_id: object.application_id(),
        bucket_id: object.bucket_id(),
        key: object.key().to_owned(),
        current_version_id,
        generation: object.generation(),
        created_at: object.created_at(),
        updated_at,
    })
    .map_err(|error| RepositoryError::Invariant(error.to_string()))
}

#[async_trait]
impl S3UploadIntentRepository for MemoryS3Repository {
    async fn create_upload_intent(&self, intent: &UploadIntent) -> Result<(), RepositoryError> {
        self.record("intent.create");
        if self
            .state()
            .intents
            .insert(intent.id(), intent.clone())
            .is_some()
        {
            return Err(RepositoryError::Conflict);
        }
        Ok(())
    }

    async fn find_upload_intent(
        &self,
        intent_id: UploadIntentId,
    ) -> Result<Option<UploadIntent>, RepositoryError> {
        Ok(self.state().intents.get(&intent_id).cloned())
    }

    async fn complete_upload_intent_staging(
        &self,
        intent_id: UploadIntentId,
        entity_tag: &EntityTag,
        checksum: &Checksum,
        size_bytes: u64,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError> {
        let mut state = self.state();
        let intent = state
            .intents
            .get(&intent_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        if intent.state() != UploadIntentState::Staging
            || intent.expected_size_bytes() != size_bytes
        {
            return Err(RepositoryError::Conflict);
        }
        let ready = rebuild_intent(
            &intent,
            UploadIntentState::Ready,
            Some((entity_tag.clone(), checksum.clone(), size_bytes)),
            None,
            None,
            now,
        );
        state.intents.insert(intent_id, ready.clone());
        Ok(ready)
    }

    async fn claim_upload_intent(
        &self,
        intent_id: UploadIntentId,
        lease_token: &str,
        lease_until: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError> {
        let mut state = self.state();
        let intent = state
            .intents
            .get(&intent_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        let claimable = intent.state() == UploadIntentState::Ready
            || (intent.state() == UploadIntentState::Committing
                && intent.lease_until().is_some_and(|until| until <= now));
        if !claimable || intent.expires_at() <= now || lease_until <= now {
            return Err(RepositoryError::Conflict);
        }
        let committing = rebuild_intent(
            &intent,
            UploadIntentState::Committing,
            facts(&intent),
            Some((lease_token.to_owned(), lease_until)),
            None,
            now,
        );
        state.intents.insert(intent_id, committing.clone());
        Ok(committing)
    }

    async fn release_upload_intent(
        &self,
        intent_id: UploadIntentId,
        lease_token: &str,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError> {
        self.record("intent.release");
        let mut state = self.state();
        let intent = state
            .intents
            .get(&intent_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        if intent.state() != UploadIntentState::Committing
            || intent.lease_token() != Some(lease_token)
        {
            return Err(RepositoryError::Conflict);
        }
        let ready = rebuild_intent(
            &intent,
            UploadIntentState::Ready,
            facts(&intent),
            None,
            None,
            now,
        );
        state.intents.insert(intent_id, ready.clone());
        Ok(ready)
    }

    async fn commit_upload_intent(
        &self,
        intent_id: UploadIntentId,
        lease_token: &str,
        commit: S3ObjectVersionCommit,
    ) -> Result<S3Object, RepositoryError> {
        commit.validate()?;
        self.record("intent.commit");
        let mut state = self.state();
        let mut commit = commit;
        let intent = state
            .intents
            .get(&intent_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        if intent.state() != UploadIntentState::Committing
            || intent.lease_token() != Some(lease_token)
        {
            return Err(RepositoryError::Conflict);
        }
        if state.fail_commit {
            state.fail_commit = false;
            return Err(RepositoryError::Unavailable(
                "injected DB commit failure".into(),
            ));
        }

        freeze_memory_object_lock(&state.buckets, &mut commit)?;

        let active_null = if commit.version.is_null_version() {
            state
                .versions
                .get(&commit.version.object_id())
                .into_iter()
                .flatten()
                .find(|version| {
                    version.is_null_version()
                        && !state.superseded_versions.contains(&version.id())
                        && !state.deleted_versions.contains(&version.id())
                        && version.state() == ObjectVersionState::Committed
                })
                .cloned()
        } else {
            None
        };
        if active_null.as_ref().map(ObjectVersion::id) != commit.replaced_null_version_id {
            return Err(RepositoryError::Conflict);
        }
        let replacement_tasks = commit
            .gc_tasks
            .iter()
            .filter(|task| task.reason == StorageGcReason::ReplacedNullVersion)
            .collect::<Vec<_>>();
        match active_null.as_ref().map(ObjectVersion::payload) {
            None | Some(ObjectVersionPayload::DeleteMarker) if replacement_tasks.is_empty() => {}
            Some(ObjectVersionPayload::Object(payload)) if replacement_tasks.len() == 1 => {
                let task = replacement_tasks[0];
                if task.object_version_id != active_null.as_ref().map(ObjectVersion::id)
                    || task.storage_backend != payload.storage_backend()
                    || task.storage_key != payload.storage_key()
                {
                    return Err(RepositoryError::Conflict);
                }
            }
            _ => return Err(RepositoryError::Conflict),
        }
        if let Some(previous) = &active_null {
            state.superseded_versions.insert(previous.id());
        }

        let version = commit.version.clone();
        let object = match commit.target {
            S3ObjectCommitTarget::Create(object) => object
                .advanced_to(&version, commit.committed_at)
                .map_err(|error| RepositoryError::Invariant(error.to_string()))?,
            S3ObjectCommitTarget::Append {
                object_id,
                expected_generation,
            } => {
                let current = state
                    .objects
                    .get(&object_id)
                    .cloned()
                    .ok_or(RepositoryError::NotFound)?;
                if current.generation() != expected_generation {
                    return Err(RepositoryError::Conflict);
                }
                current
                    .advanced_to(&version, commit.committed_at)
                    .map_err(|error| RepositoryError::Invariant(error.to_string()))?
            }
        };
        state
            .versions
            .entry(object.id())
            .or_default()
            .push(version.clone());
        state.objects.insert(object.id(), object.clone());
        state.gc_tasks.extend(commit.gc_tasks.clone());
        let committed = rebuild_intent(
            &intent,
            UploadIntentState::Committed,
            facts(&intent),
            None,
            Some((object.id(), version.id())),
            commit.committed_at,
        );
        state.intents.insert(intent_id, committed);
        Ok(object)
    }

    async fn abort_upload_intent_and_enqueue_gc(
        &self,
        intent_id: UploadIntentId,
        gc_task: NewStorageGcTask,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError> {
        let mut state = self.state();
        let intent = state
            .intents
            .get(&intent_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        if matches!(
            intent.state(),
            UploadIntentState::Aborted | UploadIntentState::Expired
        ) {
            return Ok(intent);
        }
        if !matches!(
            intent.state(),
            UploadIntentState::Staging | UploadIntentState::Ready
        ) || gc_task.upload_intent_id != Some(intent.id())
            || gc_task.storage_key != intent.temporary_storage_key()
        {
            return Err(RepositoryError::Conflict);
        }
        state.gc_tasks.push(gc_task);
        let aborted = rebuild_intent(
            &intent,
            UploadIntentState::Aborted,
            facts(&intent),
            None,
            None,
            now,
        );
        state.intents.insert(intent_id, aborted.clone());
        Ok(aborted)
    }

    async fn expire_upload_intents(
        &self,
        _now: OffsetDateTime,
        _limit: usize,
        _gc_max_attempts: u32,
    ) -> Result<usize, RepositoryError> {
        unreachable!("not used by ObjectService tests")
    }

    async fn commit_multipart_object_version(
        &self,
        _upload_id: &str,
        _completion_token: &str,
        _commit: S3ObjectVersionCommit,
        _final_entity_tag: &EntityTag,
        _final_checksum: &Checksum,
    ) -> Result<S3Object, RepositoryError> {
        unreachable!("not used by ObjectService tests")
    }
}

#[derive(Clone)]
struct MemoryS3Store {
    inner: InMemoryObjectStore,
    events: Arc<Mutex<Vec<String>>>,
    promotion_error: Arc<Mutex<Option<ObjectStoreError>>>,
}

impl MemoryS3Store {
    fn new(events: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            inner: InMemoryObjectStore::default(),
            events,
            promotion_error: Arc::new(Mutex::new(None)),
        }
    }

    fn fail_next_promotion(&self) {
        *self.promotion_error.lock().expect("promotion error lock") = Some(
            ObjectStoreError::Unavailable("injected promotion failure".into()),
        );
    }

    fn record(&self, event: &str) {
        self.events
            .lock()
            .expect("memory S3 event lock")
            .push(event.to_owned());
    }
}

#[async_trait]
impl ObjectStore for MemoryS3Store {
    fn backend_name(&self) -> &str {
        self.inner.backend_name()
    }

    async fn put_temporary(
        &self,
        temporary_key: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        self.record("storage.put");
        self.inner
            .put_temporary(temporary_key, content, content_type)
            .await
    }

    async fn compose_temporary(
        &self,
        temporary_key: &str,
        source_keys: &[String],
        content_type: &str,
    ) -> Result<ComposedObject, ObjectStoreError> {
        self.inner
            .compose_temporary(temporary_key, source_keys, content_type)
            .await
    }

    async fn commit_temporary(
        &self,
        temporary_key: &str,
        final_key: &str,
    ) -> Result<(), ObjectStoreError> {
        self.record("storage.promote");
        if let Some(error) = self
            .promotion_error
            .lock()
            .expect("promotion error lock")
            .take()
        {
            return Err(error);
        }
        self.inner.commit_temporary(temporary_key, final_key).await
    }

    async fn read(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        self.inner.read(key).await
    }

    async fn read_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ObjectStoreError> {
        self.inner.read_range(key, range).await
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, ObjectStoreError> {
        self.inner.head(key).await
    }

    async fn checksum_sha256(&self, key: &str) -> Result<String, ObjectStoreError> {
        self.inner.checksum_sha256(key).await
    }

    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ObjectPage, ObjectStoreError> {
        self.inner.list(prefix, cursor, limit).await
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        self.inner.delete(key).await
    }

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        self.inner.exists(key).await
    }
}

fn rebuild_intent(
    intent: &UploadIntent,
    state: UploadIntentState,
    facts: Option<(EntityTag, Checksum, u64)>,
    lease: Option<(String, OffsetDateTime)>,
    committed: Option<(ObjectId, ObjectVersionId)>,
    now: OffsetDateTime,
) -> UploadIntent {
    let (entity_tag, checksum, size_bytes) = facts
        .map(|(etag, checksum, size)| (Some(etag), Some(checksum), Some(size)))
        .unwrap_or((None, None, None));
    let (lease_token, lease_until) = lease
        .map(|(token, until)| (Some(token), Some(until)))
        .unwrap_or((None, None));
    let (committed_object_id, committed_version_id) = committed
        .map(|(object, version)| (Some(object), Some(version)))
        .unwrap_or((None, None));
    UploadIntent::from_persistence(PersistedUploadIntent {
        id: intent.id(),
        application_id: intent.application_id(),
        bucket_id: intent.bucket_id(),
        object_key: intent.object_key().to_owned(),
        proposed_version_id: intent.proposed_version_id(),
        state,
        storage_backend: intent.storage_backend().to_owned(),
        temporary_storage_key: intent.temporary_storage_key().to_owned(),
        final_storage_key: intent.final_storage_key().to_owned(),
        entity_tag,
        checksum,
        expected_size_bytes: intent.expected_size_bytes(),
        size_bytes,
        content_type: intent.content_type().map(str::to_owned),
        user_metadata: intent.user_metadata().clone(),
        lease_token,
        lease_until,
        committed_object_id,
        committed_version_id,
        expires_at: intent.expires_at(),
        created_at: intent.created_at(),
        updated_at: now,
    })
    .expect("valid memory upload intent")
}

fn facts(intent: &UploadIntent) -> Option<(EntityTag, Checksum, u64)> {
    Some((
        intent.entity_tag()?.clone(),
        intent.checksum()?.clone(),
        intent.size_bytes()?,
    ))
}

type Service = S3ObjectService<
    MemoryS3Repository,
    MemoryS3Repository,
    MemoryS3Repository,
    MemoryS3Store,
    FixedClock,
>;

fn bucket(
    application_id: ApplicationId,
    bucket_id: BucketId,
    status: VersioningStatus,
) -> S3Bucket {
    S3Bucket::from_persistence(PersistedS3Bucket {
        id: bucket_id,
        application_id,
        name: "assets".into(),
        configuration: PersistedBucketS3Configuration {
            region: "us-east-1".into(),
            versioning_status: status,
            object_lock_enabled: false,
            default_retention: None,
            lifecycle_configuration: None,
            revision: 1,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        },
        created_at: OffsetDateTime::UNIX_EPOCH,
    })
    .expect("valid test bucket")
}

fn setup(
    status: VersioningStatus,
) -> (
    ApplicationId,
    BucketId,
    MemoryS3Repository,
    MemoryS3Store,
    Service,
) {
    let application_id = ApplicationId::new();
    let bucket_id = BucketId::new();
    let events = Arc::new(Mutex::new(Vec::new()));
    let repository =
        MemoryS3Repository::new(bucket(application_id, bucket_id, status), events.clone());
    let store = MemoryS3Store::new(events);
    let service = S3ObjectService::new(
        repository.clone(),
        repository.clone(),
        repository.clone(),
        store.clone(),
        FixedClock::new(OffsetDateTime::UNIX_EPOCH),
    );
    (application_id, bucket_id, repository, store, service)
}

#[test]
fn memory_bucket_repository_persists_object_lock_configuration() {
    block_on(async {
        let (application_id, _, repository, _, _) = setup(VersioningStatus::Unversioned);
        let retention =
            DefaultRetention::new(RetentionMode::Governance, DefaultRetentionPeriod::Days(30))
                .expect("valid default retention");
        let updated_at = OffsetDateTime::UNIX_EPOCH + Duration::seconds(1);
        let configuration = repository
            .replace_s3_bucket_object_lock(application_id, "assets", Some(retention), updated_at)
            .await
            .expect("replace in-memory Object Lock configuration");
        assert!(configuration.object_lock_enabled());
        assert_eq!(configuration.versioning_status(), VersioningStatus::Enabled);
        assert_eq!(configuration.default_retention(), Some(retention));
        assert_eq!(configuration.revision(), 2);
        assert_eq!(configuration.updated_at(), updated_at);

        let persisted = repository
            .get_s3_bucket_configuration(application_id, "assets")
            .await
            .expect("read in-memory Object Lock configuration")
            .expect("bucket exists");
        assert_eq!(persisted, configuration);
    });
}

#[test]
fn object_lock_defaults_and_mutations_are_frozen_on_object_versions() {
    block_on(async {
        let content = b"prismark";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Unversioned);
        let default =
            DefaultRetention::new(RetentionMode::Governance, DefaultRetentionPeriod::Days(30))
                .expect("default retention");
        repository
            .replace_s3_bucket_object_lock(
                application_id,
                "assets",
                Some(default),
                OffsetDateTime::UNIX_EPOCH,
            )
            .await
            .expect("enable Object Lock");

        let begun = begin_and_stage(application_id, &service, &store, content).await;
        let completed = service
            .complete_put_with_object_lock(
                &complete_request(application_id, begun.intent.id(), content),
                NewS3ObjectLock {
                    retention: None,
                    legal_hold: Some(true),
                },
            )
            .await
            .expect("commit locked version");
        assert_eq!(
            completed.version.retention(),
            Some(ObjectRetention::new(
                RetentionMode::Governance,
                OffsetDateTime::UNIX_EPOCH + Duration::days(30),
            ))
        );
        assert!(completed.version.legal_hold());

        let object = S3ObjectRequest {
            application_id,
            bucket_name: "assets".into(),
            object_key: "folder/example.txt".into(),
            version_id: Some(completed.version.external_version_id().clone()),
        };
        assert!(matches!(
            service
                .put_object_retention(&PutObjectRetentionRequest {
                    object: object.clone(),
                    retention: ObjectRetention::new(
                        RetentionMode::Governance,
                        OffsetDateTime::UNIX_EPOCH + Duration::days(20),
                    ),
                    bypass_governance: false,
                })
                .await,
            Err(S3ObjectServiceError::RetentionUpdateLocked(
                S3DeleteLockReason::GovernanceRetention
            ))
        ));
        let shortened = service
            .put_object_retention(&PutObjectRetentionRequest {
                object: object.clone(),
                retention: ObjectRetention::new(
                    RetentionMode::Governance,
                    OffsetDateTime::UNIX_EPOCH + Duration::days(20),
                ),
                bypass_governance: true,
            })
            .await
            .expect("signed governance bypass is represented by the command flag");
        assert_eq!(
            shortened.retention().expect("retention").retain_until(),
            OffsetDateTime::UNIX_EPOCH + Duration::days(20)
        );
        let released = service
            .put_object_legal_hold(&PutObjectLegalHoldRequest {
                object,
                legal_hold: false,
            })
            .await
            .expect("release legal hold");
        assert!(!released.legal_hold());
    });
}

#[test]
fn explicit_put_object_lock_is_rejected_before_promotion_on_an_unlocked_bucket() {
    block_on(async {
        let content = b"prismark";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Enabled);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        let result = service
            .complete_put_with_object_lock(
                &complete_request(application_id, begun.intent.id(), content),
                NewS3ObjectLock {
                    retention: Some(ObjectRetention::new(
                        RetentionMode::Governance,
                        OffsetDateTime::UNIX_EPOCH + Duration::days(30),
                    )),
                    legal_hold: None,
                },
            )
            .await;
        assert!(matches!(
            result,
            Err(S3ObjectServiceError::ObjectLockNotEnabled)
        ));
        assert!(
            !repository
                .events
                .lock()
                .expect("events")
                .iter()
                .any(|event| event == "storage.promote")
        );
    });
}

fn begin_request(application_id: ApplicationId, size: u64) -> BeginPutObjectRequest {
    BeginPutObjectRequest {
        application_id,
        bucket_name: "assets".into(),
        object_key: "folder/example.txt".into(),
        expected_size_bytes: size,
        content_type: Some("text/plain".into()),
        user_metadata: serde_json::json!({ "origin": "test" }),
        expires_at: None,
    }
}

fn streamed(content: &[u8]) -> StreamedObject {
    let md5 = match content {
        b"prismark" => "c89d43adb247379adc03e0f63806210a",
        b"null-version" => "8cc0972b3440a1f75dd1d5c3867e30c8",
        b"first-null" => "4962f55f83e53d6406f8ec94f56e8758",
        b"second-null" => "b7f26341f6367e27a46651b4262dad96",
        b"retry" => "165e6d21e0a2cc9ebb32ca05f90e0fa7",
        b"reconcile" => "378d41584078efc0587e7dfc62b2ae8b",
        b"idempotent" => "579ef30b5aa1632d360fb53065f2ccda",
        b"racing-completion" => "63eb344dcced910062e609417d601297",
        _ => panic!("missing test MD5 vector"),
    };
    StreamedObject {
        size: content.len() as u64,
        sha256: hex::encode(Sha256::digest(content)),
        md5: md5.into(),
    }
}

async fn begin_and_stage(
    application_id: ApplicationId,
    service: &Service,
    store: &MemoryS3Store,
    content: &[u8],
) -> BeginPutObjectReceipt {
    let receipt = service
        .begin_put(&begin_request(application_id, content.len() as u64))
        .await
        .expect("begin put");
    store
        .put_temporary(
            receipt.intent.temporary_storage_key(),
            content,
            "text/plain",
        )
        .await
        .expect("stage object");
    receipt
}

fn complete_request(
    application_id: ApplicationId,
    intent_id: UploadIntentId,
    content: &[u8],
) -> CompletePutObjectRequest {
    CompletePutObjectRequest {
        application_id,
        intent_id,
        streamed: streamed(content),
        created_by: "test-access-key".into(),
        source_protocol: SourceProtocol::S3,
    }
}

fn list_data_head(
    application_id: ApplicationId,
    bucket_id: BucketId,
    key: &str,
) -> (S3Object, ObjectVersion) {
    let object = S3Object::new(
        ObjectId::new(),
        application_id,
        bucket_id,
        key,
        OffsetDateTime::UNIX_EPOCH,
    )
    .expect("list object");
    let version = ObjectVersion::new_object(
        ObjectVersionId::new(),
        object.id(),
        application_id,
        bucket_id,
        S3VersionId::new(format!("version-{key}")).expect("list version id"),
        1,
        false,
        ObjectVersionState::Committed,
        StoredObjectVersion::new(
            "filesystem",
            format!("objects/{key}"),
            None,
            None,
            EntityTag::new("list-etag").expect("etag"),
            4,
            None,
            serde_json::json!({}),
            Some(Checksum::sha256_hex("0".repeat(64)).expect("checksum")),
        )
        .expect("stored list object"),
        None,
        false,
        "test",
        SourceProtocol::S3,
        OffsetDateTime::UNIX_EPOCH,
    )
    .expect("list data version");
    let object = object
        .advanced_to(&version, OffsetDateTime::UNIX_EPOCH)
        .expect("current list head");
    (object, version)
}

fn list_delete_marker_head(
    application_id: ApplicationId,
    bucket_id: BucketId,
    key: &str,
) -> (S3Object, ObjectVersion) {
    let object = S3Object::new(
        ObjectId::new(),
        application_id,
        bucket_id,
        key,
        OffsetDateTime::UNIX_EPOCH,
    )
    .expect("list object");
    let marker = ObjectVersion::new_delete_marker(
        ObjectVersionId::new(),
        object.id(),
        application_id,
        bucket_id,
        S3VersionId::new(format!("marker-{key}")).expect("marker version id"),
        1,
        false,
        "test",
        SourceProtocol::S3,
        OffsetDateTime::UNIX_EPOCH,
    )
    .expect("delete marker");
    let object = object
        .advanced_to(&marker, OffsetDateTime::UNIX_EPOCH)
        .expect("current delete marker");
    (object, marker)
}

#[test]
fn memory_list_current_objects_matches_v2_prefix_delimiter_cursor_and_limit_semantics() {
    block_on(async {
        let (application_id, bucket_id, repository, _, _) = setup(VersioningStatus::Enabled);
        for key in [
            "alpha.txt",
            "docs/readme.md",
            "docs/setup/guide.md",
            "docs/setup/install.md",
            "zeta.txt",
        ] {
            let (object, version) = list_data_head(application_id, bucket_id, key);
            repository.seed_object(object, vec![version]);
        }
        let (hidden, marker) =
            list_delete_marker_head(application_id, bucket_id, "hidden-marker.txt");
        repository.seed_object(hidden, vec![marker]);
        let (superseded, superseded_version) =
            list_data_head(application_id, bucket_id, "superseded.txt");
        let superseded_version_id = superseded_version.id();
        repository.seed_object(superseded, vec![superseded_version]);
        repository
            .state()
            .superseded_versions
            .insert(superseded_version_id);

        let first = repository
            .list_current_s3_objects(
                application_id,
                &S3ObjectListQuery {
                    bucket_id,
                    prefix: String::new(),
                    start_after: None,
                    delimiter: true,
                    limit: 2,
                },
            )
            .await
            .expect("first delimiter page");
        assert_eq!(
            first
                .items
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            ["alpha.txt"]
        );
        assert_eq!(first.common_prefixes, ["docs/"]);
        assert_eq!(first.next_cursor.as_deref(), Some("docs/"));
        assert!(first.items.iter().all(|item| {
            item.version.state() == ObjectVersionState::Committed
                && matches!(item.version.payload(), ObjectVersionPayload::Object(_))
        }));

        let after_prefix = repository
            .list_current_s3_objects(
                application_id,
                &S3ObjectListQuery {
                    bucket_id,
                    prefix: String::new(),
                    start_after: Some("docs/".into()),
                    delimiter: true,
                    limit: 1_000,
                },
            )
            .await
            .expect("cursor after common prefix");
        assert_eq!(
            after_prefix
                .items
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            ["zeta.txt"]
        );
        assert!(after_prefix.common_prefixes.is_empty());
        assert_eq!(after_prefix.next_cursor, None);

        let directory = repository
            .list_current_s3_objects(
                application_id,
                &S3ObjectListQuery {
                    bucket_id,
                    prefix: "docs/".into(),
                    start_after: None,
                    delimiter: true,
                    limit: 1_000,
                },
            )
            .await
            .expect("prefixed directory page");
        assert_eq!(
            directory
                .items
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            ["docs/readme.md"]
        );
        assert_eq!(directory.common_prefixes, ["docs/setup/"]);
        assert_eq!(directory.next_cursor, None);

        let recursive = repository
            .list_current_s3_objects(
                application_id,
                &S3ObjectListQuery {
                    bucket_id,
                    prefix: "docs/".into(),
                    start_after: Some("docs/readme.md".into()),
                    delimiter: false,
                    limit: 1_000,
                },
            )
            .await
            .expect("recursive page");
        assert_eq!(
            recursive
                .items
                .iter()
                .map(|item| item.key.as_str())
                .collect::<Vec<_>>(),
            ["docs/setup/guide.md", "docs/setup/install.md"]
        );
        assert!(recursive.common_prefixes.is_empty());

        let empty = repository
            .list_current_s3_objects(
                application_id,
                &S3ObjectListQuery {
                    bucket_id,
                    prefix: String::new(),
                    start_after: None,
                    delimiter: true,
                    limit: 0,
                },
            )
            .await
            .expect("zero limit page");
        assert_eq!(empty, S3ObjectPage::default());

        assert!(matches!(
            repository
                .list_current_s3_objects(
                    application_id,
                    &S3ObjectListQuery {
                        bucket_id,
                        prefix: String::new(),
                        start_after: None,
                        delimiter: false,
                        limit: 1_001,
                    },
                )
                .await,
            Err(RepositoryError::Invariant(_))
        ));
    });
}

#[test]
fn intent_is_persisted_before_storage_is_addressable() {
    block_on(async {
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Enabled);
        let begun = service
            .begin_put(&begin_request(application_id, 4))
            .await
            .expect("begin put");
        assert_eq!(
            repository.events.lock().expect("events").as_slice(),
            ["intent.create"]
        );
        assert_eq!(repository.intent(begun.intent.id()), begun.intent);
        store
            .put_temporary(begun.intent.temporary_storage_key(), b"data", "text/plain")
            .await
            .expect("stage after begin");
        assert_eq!(
            repository.events.lock().expect("events").as_slice(),
            ["intent.create", "storage.put"]
        );
        assert!(
            begun
                .intent
                .temporary_storage_key()
                .contains(&begun.intent.id().to_string())
        );
        assert!(
            begun
                .intent
                .final_storage_key()
                .contains(&begun.intent.proposed_version_id().to_string())
        );
    });
}

#[test]
fn enabled_put_creates_opaque_version_and_current_or_exact_reads() {
    block_on(async {
        let content = b"prismark";
        let (application_id, _, _, store, service) = setup(VersioningStatus::Enabled);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        let completed = service
            .complete_put(&complete_request(
                application_id,
                begun.intent.id(),
                content,
            ))
            .await
            .expect("complete put");
        assert!(!completed.version.is_null_version());
        assert_ne!(completed.version.external_version_id().as_str(), "null");
        let ObjectVersionPayload::Object(stored) = completed.version.payload() else {
            panic!("PutObject must create an object payload");
        };
        assert_eq!(stored.etag().as_str(), "c89d43adb247379adc03e0f63806210a");
        assert_eq!(
            stored.checksum().expect("SHA-256 checksum").value(),
            hex::encode(Sha256::digest(content))
        );

        let current = service
            .get(&S3ObjectRequest {
                application_id,
                bucket_name: "assets".into(),
                object_key: "folder/example.txt".into(),
                version_id: None,
            })
            .await
            .expect("get current");
        assert_eq!(current.content, content);
        let exact = service
            .head(&S3ObjectRequest {
                application_id,
                bucket_name: "assets".into(),
                object_key: "folder/example.txt".into(),
                version_id: Some(completed.version.external_version_id().clone()),
            })
            .await
            .expect("head exact version");
        assert_eq!(exact.version.id(), completed.version.id());
    });
}

#[test]
fn unversioned_and_suspended_puts_use_null_version() {
    for status in [VersioningStatus::Unversioned, VersioningStatus::Suspended] {
        block_on(async {
            let content = b"null-version";
            let (application_id, _, _, store, service) = setup(status);
            let begun = begin_and_stage(application_id, &service, &store, content).await;
            let completed = service
                .complete_put(&complete_request(
                    application_id,
                    begun.intent.id(),
                    content,
                ))
                .await
                .expect("complete null version");
            assert!(completed.version.is_null_version());
            assert_eq!(completed.version.external_version_id().as_str(), "null");
        });
    }
}

#[test]
fn expired_committing_upload_intent_can_be_taken_over_with_a_new_fence() {
    block_on(async {
        let content = b"retry";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Enabled);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        let streamed = streamed(content);
        let entity_tag = EntityTag::new(&streamed.md5).expect("entity tag");
        let checksum = Checksum::sha256_hex(&streamed.sha256).expect("checksum");
        repository
            .complete_upload_intent_staging(
                begun.intent.id(),
                &entity_tag,
                &checksum,
                streamed.size,
                OffsetDateTime::UNIX_EPOCH,
            )
            .await
            .expect("freeze intent facts");
        let first_lease_until = OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(10);
        repository
            .claim_upload_intent(
                begun.intent.id(),
                "first-fence",
                first_lease_until,
                OffsetDateTime::UNIX_EPOCH,
            )
            .await
            .expect("first claim");
        assert!(matches!(
            repository
                .claim_upload_intent(
                    begun.intent.id(),
                    "early-takeover",
                    first_lease_until + time::Duration::seconds(10),
                    OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(9),
                )
                .await,
            Err(RepositoryError::Conflict)
        ));
        let taken_over = repository
            .claim_upload_intent(
                begun.intent.id(),
                "second-fence",
                first_lease_until + time::Duration::seconds(10),
                first_lease_until,
            )
            .await
            .expect("expired lease takeover");
        assert_eq!(taken_over.state(), UploadIntentState::Committing);
        assert_eq!(taken_over.lease_token(), Some("second-fence"));
        assert_eq!(taken_over.entity_tag(), Some(&entity_tag));
        assert_eq!(taken_over.checksum(), Some(&checksum));
    });
}
#[test]
fn promotion_failure_releases_the_lease() {
    block_on(async {
        let content = b"retry";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Enabled);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        store.fail_next_promotion();
        let error = service
            .complete_put(&complete_request(
                application_id,
                begun.intent.id(),
                content,
            ))
            .await
            .expect_err("promotion fails");
        assert!(matches!(
            error,
            S3ObjectServiceError::Promotion(ObjectStoreError::Unavailable(_))
        ));
        assert_eq!(
            repository.intent(begun.intent.id()).state(),
            UploadIntentState::Ready
        );
        assert!(
            repository
                .events
                .lock()
                .expect("events")
                .iter()
                .any(|event| event == "intent.release")
        );
    });
}

#[test]
fn database_failure_after_promotion_retains_committing_lease() {
    block_on(async {
        let content = b"reconcile";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Enabled);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        repository.fail_next_commit();
        let error = service
            .complete_put(&complete_request(
                application_id,
                begun.intent.id(),
                content,
            ))
            .await
            .expect_err("DB commit fails");
        assert!(matches!(
            error,
            S3ObjectServiceError::Repository(RepositoryError::Unavailable(_))
        ));
        let intent = repository.intent(begun.intent.id());
        assert_eq!(intent.state(), UploadIntentState::Committing);
        assert!(intent.lease_token().is_some());
        assert!(
            store
                .exists(intent.final_storage_key())
                .await
                .expect("final key exists")
        );
    });
}

#[test]
fn exact_version_is_readable_while_delete_marker_hides_current_and_lists_in_history() {
    block_on(async {
        let (application_id, bucket_id, repository, store, service) =
            setup(VersioningStatus::Enabled);
        let now = OffsetDateTime::UNIX_EPOCH;
        let object = S3Object::new(
            ObjectId::new(),
            application_id,
            bucket_id,
            "folder/example.txt",
            now,
        )
        .expect("object");
        let storage_key = "s3/objects/seed-version";
        store
            .put_temporary("s3/staging/seed", b"older", "text/plain")
            .await
            .expect("stage seed");
        store
            .commit_temporary("s3/staging/seed", storage_key)
            .await
            .expect("promote seed");
        let version = ObjectVersion::new_object(
            ObjectVersionId::new(),
            object.id(),
            application_id,
            bucket_id,
            S3VersionId::new("version-one").expect("version ID"),
            1,
            false,
            ObjectVersionState::Committed,
            StoredObjectVersion::new(
                store.backend_name(),
                storage_key,
                None,
                None,
                EntityTag::new("etag-one").expect("etag"),
                5,
                Some("text/plain".into()),
                serde_json::json!({}),
                Some(
                    Checksum::sha256_hex(hex::encode(Sha256::digest(b"older"))).expect("checksum"),
                ),
            )
            .expect("stored object"),
            None,
            false,
            "test",
            SourceProtocol::S3,
            now,
        )
        .expect("object version");
        let object = object.advanced_to(&version, now).expect("advance object");
        let marker = ObjectVersion::new_delete_marker(
            ObjectVersionId::new(),
            object.id(),
            application_id,
            bucket_id,
            S3VersionId::new("delete-marker").expect("marker ID"),
            2,
            false,
            "test",
            SourceProtocol::S3,
            now,
        )
        .expect("delete marker");
        let object = object.advanced_to(&marker, now).expect("advance marker");
        repository.seed_object(object, vec![version.clone(), marker.clone()]);

        let current = service
            .get(&S3ObjectRequest {
                application_id,
                bucket_name: "assets".into(),
                object_key: "folder/example.txt".into(),
                version_id: None,
            })
            .await;
        assert!(matches!(
            current,
            Err(S3ObjectServiceError::DeleteMarker {
                ref version_id,
                is_current: true,
            }) if version_id == marker.external_version_id()
        ));
        let exact = service
            .get(&S3ObjectRequest {
                application_id,
                bucket_name: "assets".into(),
                object_key: "folder/example.txt".into(),
                version_id: Some(version.external_version_id().clone()),
            })
            .await
            .expect("read exact version");
        assert_eq!(exact.content, b"older");
        let marker_result = service
            .head(&S3ObjectRequest {
                application_id,
                bucket_name: "assets".into(),
                object_key: "folder/example.txt".into(),
                version_id: Some(marker.external_version_id().clone()),
            })
            .await;
        assert!(matches!(
            marker_result,
            Err(S3ObjectServiceError::DeleteMarker {
                is_current: true,
                ..
            })
        ));
        let versions = service
            .list_versions(&ListObjectVersionsRequest {
                application_id,
                bucket_name: "assets".into(),
                object_key: "folder/example.txt".into(),
            })
            .await
            .expect("list versions");
        assert_eq!(versions, vec![version, marker]);
    });
}

#[test]
fn second_null_put_atomically_supersedes_the_old_version_and_enqueues_gc() {
    block_on(async {
        let first_content = b"first-null";
        let second_content = b"second-null";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Unversioned);
        let first = begin_and_stage(application_id, &service, &store, first_content).await;
        let first_receipt = service
            .complete_put(&complete_request(
                application_id,
                first.intent.id(),
                first_content,
            ))
            .await
            .expect("first null version");

        let second = begin_and_stage(application_id, &service, &store, second_content).await;
        let second_receipt = service
            .complete_put(&complete_request(
                application_id,
                second.intent.id(),
                second_content,
            ))
            .await
            .expect("replace null version");
        assert_ne!(first_receipt.version.id(), second_receipt.version.id());
        assert_eq!(
            second_receipt.version.external_version_id().as_str(),
            "null"
        );

        let visible = service
            .list_versions(&ListObjectVersionsRequest {
                application_id,
                bucket_name: "assets".into(),
                object_key: "folder/example.txt".into(),
            })
            .await
            .expect("list active history");
        assert_eq!(visible, vec![second_receipt.version.clone()]);

        {
            let state = repository.state();
            assert!(
                state
                    .superseded_versions
                    .contains(&first_receipt.version.id())
            );
            assert_eq!(state.gc_tasks.len(), 1);
            let task = &state.gc_tasks[0];
            assert_eq!(task.reason, StorageGcReason::ReplacedNullVersion);
            assert_eq!(task.object_version_id, Some(first_receipt.version.id()));
        }

        let replay = service
            .complete_put(&complete_request(
                application_id,
                first.intent.id(),
                first_content,
            ))
            .await
            .expect("superseded committed intent still replays");
        assert_eq!(replay.version.id(), first_receipt.version.id());

        let events = repository.events.lock().expect("events");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.as_str() == "storage.promote")
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.as_str() == "intent.commit")
                .count(),
            2
        );
    });
}

#[test]
fn repeated_complete_returns_same_commit_without_second_promotion() {
    block_on(async {
        let content = b"idempotent";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Enabled);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        let request = complete_request(application_id, begun.intent.id(), content);
        let first = service
            .complete_put(&request)
            .await
            .expect("first completion");
        let second = service
            .complete_put(&request)
            .await
            .expect("idempotent completion");
        assert_eq!(first.object.id(), second.object.id());
        assert_eq!(first.version.id(), second.version.id());
        let events = repository.events.lock().expect("events");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.as_str() == "storage.promote")
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.as_str() == "intent.commit")
                .count(),
            1
        );
    });
}

#[test]
fn retry_while_commit_is_uncertain_reports_in_progress_without_repromotion() {
    block_on(async {
        let content = b"racing-completion";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Enabled);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        let request = complete_request(application_id, begun.intent.id(), content);
        repository.fail_next_commit();
        assert!(matches!(
            service.complete_put(&request).await,
            Err(S3ObjectServiceError::Repository(
                RepositoryError::Unavailable(_)
            ))
        ));
        assert!(matches!(
            service.complete_put(&request).await,
            Err(S3ObjectServiceError::CompletionInProgress)
        ));
        let events = repository.events.lock().expect("events");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.as_str() == "storage.promote")
                .count(),
            1
        );
    });
}

fn delete_request(
    application_id: ApplicationId,
    object_key: &str,
    version_id: Option<S3VersionId>,
    bypass_governance: bool,
) -> DeleteObjectRequest {
    DeleteObjectRequest {
        application_id,
        bucket_name: "assets".into(),
        object_key: object_key.into(),
        version_id,
        bypass_governance,
        deleted_by: "delete-test-access-key".into(),
    }
}

fn locked_data_head(
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: &str,
    external_version_id: &str,
    is_null_version: bool,
    retention: Option<ObjectRetention>,
    legal_hold: bool,
) -> (S3Object, ObjectVersion) {
    let object = S3Object::new(
        ObjectId::new(),
        application_id,
        bucket_id,
        object_key,
        OffsetDateTime::UNIX_EPOCH,
    )
    .expect("locked object");
    let version = ObjectVersion::new_object(
        ObjectVersionId::new(),
        object.id(),
        application_id,
        bucket_id,
        S3VersionId::new(external_version_id).expect("locked version id"),
        1,
        is_null_version,
        ObjectVersionState::Committed,
        StoredObjectVersion::new(
            "filesystem",
            format!("objects/locked/{object_key}"),
            None,
            None,
            EntityTag::new("locked-etag").expect("locked etag"),
            4,
            None,
            serde_json::json!({}),
            Some(Checksum::sha256_hex("0".repeat(64)).expect("locked checksum")),
        )
        .expect("locked stored object"),
        retention,
        legal_hold,
        "test",
        SourceProtocol::S3,
        OffsetDateTime::UNIX_EPOCH,
    )
    .expect("locked version");
    let object = object
        .advanced_to(&version, OffsetDateTime::UNIX_EPOCH)
        .expect("locked current head");
    (object, version)
}

#[test]
fn enabled_delete_appends_current_marker_and_exact_marker_delete_restores_data_head() {
    block_on(async {
        let content = b"prismark";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Enabled);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        let data = service
            .complete_put(&complete_request(
                application_id,
                begun.intent.id(),
                content,
            ))
            .await
            .expect("seed enabled object");

        let deleted = service
            .delete(&delete_request(
                application_id,
                "folder/example.txt",
                None,
                false,
            ))
            .await
            .expect("append delete marker");
        let marker_version_id = deleted.version_id.clone().expect("marker version id");
        assert!(deleted.delete_marker);
        assert_ne!(marker_version_id.as_str(), "null");
        assert!(matches!(
            service
                .head(&S3ObjectRequest {
                    application_id,
                    bucket_name: "assets".into(),
                    object_key: "folder/example.txt".into(),
                    version_id: None,
                })
                .await,
            Err(S3ObjectServiceError::DeleteMarker {
                version_id,
                is_current: true,
            }) if version_id == marker_version_id
        ));
        assert_eq!(
            service
                .head(&S3ObjectRequest {
                    application_id,
                    bucket_name: "assets".into(),
                    object_key: "folder/example.txt".into(),
                    version_id: Some(data.version.external_version_id().clone()),
                })
                .await
                .expect("old data remains readable")
                .version
                .id(),
            data.version.id()
        );

        let marker_deleted = service
            .delete(&delete_request(
                application_id,
                "folder/example.txt",
                Some(marker_version_id.clone()),
                false,
            ))
            .await
            .expect("delete exact marker");
        assert_eq!(marker_deleted.version_id, Some(marker_version_id));
        assert!(marker_deleted.delete_marker);
        assert_eq!(
            service
                .head(&S3ObjectRequest {
                    application_id,
                    bucket_name: "assets".into(),
                    object_key: "folder/example.txt".into(),
                    version_id: None,
                })
                .await
                .expect("data becomes current again")
                .version
                .id(),
            data.version.id()
        );
        assert!(repository.state().gc_tasks.is_empty());
    });
}

#[test]
fn suspended_delete_replaces_active_null_with_null_marker_and_enqueues_gc() {
    block_on(async {
        let content = b"null-version";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Suspended);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        let data = service
            .complete_put(&complete_request(
                application_id,
                begun.intent.id(),
                content,
            ))
            .await
            .expect("seed suspended null");

        let deleted = service
            .delete(&delete_request(
                application_id,
                "folder/example.txt",
                None,
                false,
            ))
            .await
            .expect("replace null with marker");
        assert_eq!(
            deleted.version_id.as_ref().map(S3VersionId::as_str),
            Some("null")
        );
        assert!(deleted.delete_marker);
        assert!(matches!(
            service
                .head(&S3ObjectRequest {
                    application_id,
                    bucket_name: "assets".into(),
                    object_key: "folder/example.txt".into(),
                    version_id: None,
                })
                .await,
            Err(S3ObjectServiceError::DeleteMarker {
                ref version_id,
                is_current: true,
            }) if version_id.as_str() == "null"
        ));

        let state = repository.state();
        assert!(state.superseded_versions.contains(&data.version.id()));
        assert_eq!(state.gc_tasks.len(), 1);
        let task = &state.gc_tasks[0];
        let ObjectVersionPayload::Object(payload) = data.version.payload() else {
            panic!("seed must be data");
        };
        assert_eq!(task.reason, StorageGcReason::ExplicitDelete);
        assert_eq!(task.object_version_id, Some(data.version.id()));
        assert_eq!(task.storage_backend, payload.storage_backend());
        assert_eq!(task.storage_key, payload.storage_key());
        assert_eq!(
            task.not_before,
            OffsetDateTime::UNIX_EPOCH + Duration::hours(24)
        );
    });
}

#[test]
fn unversioned_delete_clears_head_preserves_audit_and_is_idempotent() {
    block_on(async {
        let content = b"null-version";
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Unversioned);
        let begun = begin_and_stage(application_id, &service, &store, content).await;
        let data = service
            .complete_put(&complete_request(
                application_id,
                begun.intent.id(),
                content,
            ))
            .await
            .expect("seed unversioned null");

        let first = service
            .delete(&delete_request(
                application_id,
                "folder/example.txt",
                None,
                false,
            ))
            .await
            .expect("delete unversioned object");
        assert_eq!(
            first,
            crate::DeleteObjectReceipt {
                version_id: None,
                delete_marker: false,
            }
        );
        assert!(matches!(
            service
                .head(&S3ObjectRequest {
                    application_id,
                    bucket_name: "assets".into(),
                    object_key: "folder/example.txt".into(),
                    version_id: None,
                })
                .await,
            Err(S3ObjectServiceError::ObjectNotFound)
        ));
        assert_eq!(
            repository
                .find_s3_object_version_by_id(data.version.id())
                .await
                .expect("audit lookup"),
            Some(data.version.clone())
        );
        assert!(
            service
                .list_versions(&ListObjectVersionsRequest {
                    application_id,
                    bucket_name: "assets".into(),
                    object_key: "folder/example.txt".into(),
                })
                .await
                .expect("visible history")
                .is_empty()
        );

        let second = service
            .delete(&delete_request(
                application_id,
                "folder/example.txt",
                None,
                false,
            ))
            .await
            .expect("idempotent repeated delete");
        assert_eq!(second, first);
        assert_eq!(repository.state().gc_tasks.len(), 1);
    });
}

#[test]
fn exact_delete_hides_noncurrent_and_recomputes_current_by_generation() {
    block_on(async {
        let (application_id, _, repository, store, service) = setup(VersioningStatus::Enabled);
        let mut versions = Vec::new();
        for content in [
            b"prismark".as_slice(),
            b"retry".as_slice(),
            b"idempotent".as_slice(),
        ] {
            let begun = begin_and_stage(application_id, &service, &store, content).await;
            versions.push(
                service
                    .complete_put(&complete_request(
                        application_id,
                        begun.intent.id(),
                        content,
                    ))
                    .await
                    .expect("append numbered version")
                    .version,
            );
        }

        let noncurrent = service
            .delete(&delete_request(
                application_id,
                "folder/example.txt",
                Some(versions[1].external_version_id().clone()),
                false,
            ))
            .await
            .expect("delete noncurrent version");
        assert_eq!(
            noncurrent.version_id.as_ref(),
            Some(versions[1].external_version_id())
        );
        assert!(!noncurrent.delete_marker);
        assert_eq!(
            service
                .head(&S3ObjectRequest {
                    application_id,
                    bucket_name: "assets".into(),
                    object_key: "folder/example.txt".into(),
                    version_id: None,
                })
                .await
                .expect("third remains current")
                .version
                .id(),
            versions[2].id()
        );

        service
            .delete(&delete_request(
                application_id,
                "folder/example.txt",
                Some(versions[2].external_version_id().clone()),
                false,
            ))
            .await
            .expect("delete current version");
        assert_eq!(
            service
                .head(&S3ObjectRequest {
                    application_id,
                    bucket_name: "assets".into(),
                    object_key: "folder/example.txt".into(),
                    version_id: None,
                })
                .await
                .expect("highest surviving generation becomes current")
                .version
                .id(),
            versions[0].id()
        );
        assert!(matches!(
            service
                .head(&S3ObjectRequest {
                    application_id,
                    bucket_name: "assets".into(),
                    object_key: "folder/example.txt".into(),
                    version_id: Some(versions[1].external_version_id().clone()),
                })
                .await,
            Err(S3ObjectServiceError::VersionNotFound)
        ));
        let state = repository.state();
        assert!(state.deleted_versions.contains(&versions[1].id()));
        assert!(state.deleted_versions.contains(&versions[2].id()));
        assert_eq!(state.gc_tasks.len(), 2);
        assert!(state.gc_tasks.iter().all(|task| {
            task.reason == StorageGcReason::ExplicitDelete && task.object_version_id.is_some()
        }));
    });
}

#[test]
fn delete_enforces_legal_hold_and_retention_before_gc_enqueue() {
    block_on(async {
        let (application_id, bucket_id, repository, _, service) = setup(VersioningStatus::Enabled);
        let future = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
        let cases = [
            (
                "legal.txt",
                "legal-version",
                None,
                true,
                S3DeleteLockReason::LegalHold,
            ),
            (
                "compliance.txt",
                "compliance-version",
                Some(ObjectRetention::new(RetentionMode::Compliance, future)),
                false,
                S3DeleteLockReason::ComplianceRetention,
            ),
            (
                "governance.txt",
                "governance-version",
                Some(ObjectRetention::new(RetentionMode::Governance, future)),
                false,
                S3DeleteLockReason::GovernanceRetention,
            ),
        ];
        let mut seeded = Vec::new();
        for (key, version_id, retention, legal_hold, _) in &cases {
            let (object, version) = locked_data_head(
                application_id,
                bucket_id,
                key,
                version_id,
                false,
                *retention,
                *legal_hold,
            );
            seeded.push(version.clone());
            repository.seed_object(object, vec![version]);
        }

        for ((key, _, _, _, expected), version) in cases.iter().zip(&seeded) {
            assert!(matches!(
                service
                    .delete(&delete_request(
                        application_id,
                        key,
                        Some(version.external_version_id().clone()),
                        false,
                    ))
                    .await,
                Err(S3ObjectServiceError::DeleteLocked(reason)) if reason == *expected
            ));
        }
        assert!(matches!(
            service
                .delete(&delete_request(
                    application_id,
                    "compliance.txt",
                    Some(seeded[1].external_version_id().clone()),
                    true,
                ))
                .await,
            Err(S3ObjectServiceError::DeleteLocked(
                S3DeleteLockReason::ComplianceRetention
            ))
        ));
        assert!(repository.state().gc_tasks.is_empty());

        let bypassed = service
            .delete(&delete_request(
                application_id,
                "governance.txt",
                Some(seeded[2].external_version_id().clone()),
                true,
            ))
            .await
            .expect("governance bypass");
        assert_eq!(
            bypassed.version_id.as_ref(),
            Some(seeded[2].external_version_id())
        );
        assert_eq!(repository.state().gc_tasks.len(), 1);
    });
}

#[test]
fn delete_missing_key_is_idempotent_but_missing_explicit_version_is_not_found() {
    block_on(async {
        let (application_id, _, _, _, service) = setup(VersioningStatus::Enabled);
        assert_eq!(
            service
                .delete(&delete_request(application_id, "missing.txt", None, false,))
                .await
                .expect("missing key no-op"),
            crate::DeleteObjectReceipt {
                version_id: None,
                delete_marker: false,
            }
        );
        assert!(matches!(
            service
                .delete(&delete_request(
                    application_id,
                    "missing.txt",
                    Some(S3VersionId::new("missing-version").expect("version id")),
                    false,
                ))
                .await,
            Err(S3ObjectServiceError::VersionNotFound)
        ));
    });
}
