use std::{fmt, future::Future, time::Duration as StdDuration};

use futures_timer::Delay;
use futures_util::FutureExt;
use mediahub_core::{
    ApplicationId, BucketId, ClientMetadata, DomainError, Media, MediaId, NewMedia, OffsetDateTime,
    SystemMetadata, Visibility,
};
use sha2::{Digest, Sha256};

use crate::{
    ApplicationError, BucketRepository, Clock, MediaRepository, ObjectStore, OutboxEvent, Redacted,
    RepositoryError, S3MultipartRepository,
};

pub const MEDIA_UPLOAD_LEASE_SECONDS: i64 = 120;
pub const MEDIA_UPLOAD_HEARTBEAT_SECONDS: u64 = 30;

/// Input accepted after authentication and transport-layer parsing. Content
/// facts such as its SHA-256 and size are derived by the application service.
#[derive(Clone)]
pub struct UploadMediaRequest {
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub original_name: Option<String>,
    pub display_name: String,
    pub extension: Option<String>,
    pub mime: String,
    pub content: Vec<u8>,
    pub visibility_override: Option<Visibility>,
    pub expire_at: Option<OffsetDateTime>,
    pub metadata: ClientMetadata,
}

/// A complete object already staged by a transport-specific flow such as S3
/// Multipart. Size and SHA-256 must be derived while composing the staged
/// bytes, never inferred from multipart ETags.
#[derive(Clone)]
pub struct StagedUploadMediaRequest {
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub original_name: Option<String>,
    pub display_name: String,
    pub extension: Option<String>,
    pub mime: String,
    pub temporary_key: String,
    pub size: u64,
    pub sha256: String,
    pub visibility_override: Option<Visibility>,
    pub expire_at: Option<OffsetDateTime>,
    pub metadata: ClientMetadata,
}

impl fmt::Debug for UploadMediaRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadMediaRequest")
            .field("application_id", &self.application_id)
            .field("bucket_id", &self.bucket_id)
            .field("object_key", &self.object_key)
            .field("original_name", &self.original_name)
            .field("display_name", &self.display_name)
            .field("extension", &self.extension)
            .field("mime", &self.mime)
            .field("content", &Redacted(&self.content))
            .field("visibility_override", &self.visibility_override)
            .field("expire_at", &self.expire_at)
            .field("metadata", &Redacted(&self.metadata))
            .finish()
    }
}

impl fmt::Debug for StagedUploadMediaRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StagedUploadMediaRequest")
            .field("application_id", &self.application_id)
            .field("bucket_id", &self.bucket_id)
            .field("object_key", &self.object_key)
            .field("original_name", &self.original_name)
            .field("display_name", &self.display_name)
            .field("extension", &self.extension)
            .field("mime", &self.mime)
            .field("temporary_key", &Redacted(&self.temporary_key))
            .field("size", &self.size)
            .field("sha256", &Redacted(&self.sha256))
            .field("visibility_override", &self.visibility_override)
            .field("expire_at", &self.expire_at)
            .field("metadata", &Redacted(&self.metadata))
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct UploadReceipt {
    pub media: Media,
    pub event_id: String,
}

/// Coordinates the durable state machine around a single-part media upload.
/// Streaming and multipart transport parsing stay in the runtime layer; this
/// service receives a fully validated byte sequence and applies business rules.
pub struct UploadMediaService<O, M, B, C> {
    object_store: O,
    media_repository: M,
    bucket_repository: B,
    clock: C,
}

impl<O, M, B, C> UploadMediaService<O, M, B, C>
where
    O: ObjectStore,
    M: MediaRepository,
    B: BucketRepository,
    C: Clock,
{
    #[must_use]
    pub fn new(object_store: O, media_repository: M, bucket_repository: B, clock: C) -> Self {
        Self {
            object_store,
            media_repository,
            bucket_repository,
            clock,
        }
    }

    /// # Errors
    ///
    /// Returns a stable application error for authorization scope, duplicate
    /// keys, policy and quota violations, or an adapter failure. Failures
    /// before promotion clean up the staged row; promotion and DB commit
    /// failures are treated as ambiguous and retain the final object/row for
    /// reconciliation rather than risking data loss.
    pub async fn upload(
        &self,
        request: &UploadMediaRequest,
    ) -> Result<UploadReceipt, ApplicationError> {
        let bucket = self
            .bucket_repository
            .find_by_id(request.bucket_id)
            .await?
            .ok_or(ApplicationError::BucketNotFound)?;
        if bucket.application_id() != request.application_id {
            return Err(ApplicationError::BucketDoesNotBelongToApplication);
        }

        let size =
            u64::try_from(request.content.len()).map_err(|_| DomainError::ObjectTooLarge {
                actual: u64::MAX,
                maximum: u64::MAX,
            })?;
        bucket.validate_upload(&request.mime, size)?;

        if self
            .media_repository
            .find_by_object_key(
                request.application_id,
                request.bucket_id,
                &request.object_key,
            )
            .await?
            .is_some()
        {
            return Err(ApplicationError::ObjectAlreadyExists);
        }

        self.media_repository
            .reserve_quota(request.application_id, size)
            .await
            .map_err(map_quota_error)?;

        let now = self.clock.now();
        let media_id = MediaId::new();
        let temporary_key = format!("temporary/{media_id}");
        let final_key = format!("objects/{media_id}");
        let expire_at = request.expire_at.or_else(|| {
            bucket
                .policy()
                .default_ttl_seconds()
                .and_then(|seconds| i64::try_from(seconds).ok())
                .map(|seconds| now + time::Duration::seconds(seconds))
        });
        let media =
            match self.build_uploading_media(request, media_id, final_key, size, expire_at, now) {
                Ok(media) => media,
                Err(error) => {
                    let _ = self
                        .media_repository
                        .release_quota(request.application_id, size)
                        .await;
                    return Err(error);
                }
            };

        let lease_token = MediaId::new().to_string();
        let leased_until = now + time::Duration::seconds(MEDIA_UPLOAD_LEASE_SECONDS);
        if let Err(error) = self
            .media_repository
            .create_uploading(media.clone(), &temporary_key, &lease_token, leased_until)
            .await
        {
            let _ = self
                .media_repository
                .release_quota(media.application_id(), media.size())
                .await;
            return Err(map_create_error(error));
        }

        if let Err(error) = run_storage_with_upload_lease(
            &self.media_repository,
            &self.clock,
            media.id(),
            &lease_token,
            self.object_store
                .put_temporary(&temporary_key, &request.content, media.mime()),
        )
        .await
        {
            match error {
                LeasedStorageError::Storage(storage_error) => {
                    self.rollback_before_promotion(media.id(), &temporary_key, &lease_token)
                        .await;
                    return Err(storage_error.into());
                }
                LeasedStorageError::Ownership(repository_error) => {
                    return Err(repository_error.into());
                }
            }
        }

        if let Err(error) = run_storage_with_upload_lease(
            &self.media_repository,
            &self.clock,
            media.id(),
            &lease_token,
            self.object_store
                .commit_temporary(&temporary_key, media.storage_key()),
        )
        .await
        {
            // Any response from promotion is ambiguous: the provider may
            // have created the final object before the error reached us.
            // Never abort the durable row or delete the final key here.
            return Err(error.into_application_error());
        }

        let event = OutboxEvent::media_uploaded(&media, now);
        match self
            .media_repository
            .commit_upload(media.id(), &lease_token, self.clock.now(), event.clone())
            .await
        {
            Ok(media) => Ok(UploadReceipt {
                media,
                event_id: event.id,
            }),
            Err(error) => {
                // The repository error may be post-commit (for example a
                // response lost after PostgreSQL committed). Keep both the
                // final object and uploading row for reconciliation instead
                // of deleting potentially active data.
                Err(error.into())
            }
        }
    }

    /// Commits a transport-composed temporary object through the same Media,
    /// quota, policy, outbox, and rollback invariants as a single PUT.
    pub async fn upload_staged(
        &self,
        request: &StagedUploadMediaRequest,
    ) -> Result<UploadReceipt, ApplicationError> {
        let bucket = self
            .bucket_repository
            .find_by_id(request.bucket_id)
            .await?
            .ok_or(ApplicationError::BucketNotFound)?;
        if bucket.application_id() != request.application_id {
            return Err(ApplicationError::BucketDoesNotBelongToApplication);
        }
        bucket.validate_upload(&request.mime, request.size)?;
        if self
            .media_repository
            .find_by_object_key(
                request.application_id,
                request.bucket_id,
                &request.object_key,
            )
            .await?
            .is_some()
        {
            return Err(ApplicationError::ObjectAlreadyExists);
        }
        self.media_repository
            .reserve_quota(request.application_id, request.size)
            .await
            .map_err(map_quota_error)?;

        let now = self.clock.now();
        let media_id = MediaId::new();
        let final_key = format!("objects/{media_id}");
        let expire_at = request.expire_at.or_else(|| {
            bucket
                .policy()
                .default_ttl_seconds()
                .and_then(|seconds| i64::try_from(seconds).ok())
                .map(|seconds| now + time::Duration::seconds(seconds))
        });
        let system_metadata = match SystemMetadata::new(
            &request.mime,
            request.size,
            None,
            None,
            None,
            request.sha256.clone(),
        ) {
            Ok(metadata) => metadata,
            Err(error) => {
                let _ = self
                    .media_repository
                    .release_quota(request.application_id, request.size)
                    .await;
                return Err(error.into());
            }
        };
        let media = match Media::new(
            NewMedia {
                id: media_id,
                application_id: request.application_id,
                bucket_id: request.bucket_id,
                object_key: request.object_key.clone(),
                original_name: request.original_name.clone(),
                display_name: request.display_name.clone(),
                extension: request.extension.clone(),
                storage_backend: self.object_store.backend_name().to_owned(),
                storage_key: final_key,
                visibility_override: request.visibility_override,
                expire_at,
                system_metadata,
                client_metadata: request.metadata.clone(),
            },
            now,
        ) {
            Ok(media) => media,
            Err(error) => {
                let _ = self
                    .media_repository
                    .release_quota(request.application_id, request.size)
                    .await;
                return Err(error.into());
            }
        };
        let lease_token = MediaId::new().to_string();
        let leased_until = now + time::Duration::seconds(MEDIA_UPLOAD_LEASE_SECONDS);
        if let Err(error) = self
            .media_repository
            .create_uploading(
                media.clone(),
                &request.temporary_key,
                &lease_token,
                leased_until,
            )
            .await
        {
            let _ = self
                .media_repository
                .release_quota(media.application_id(), media.size())
                .await;
            return Err(map_create_error(error));
        }
        if let Err(error) = run_storage_with_upload_lease(
            &self.media_repository,
            &self.clock,
            media.id(),
            &lease_token,
            self.object_store
                .commit_temporary(&request.temporary_key, media.storage_key()),
        )
        .await
        {
            return Err(error.into_application_error());
        }
        let event = OutboxEvent::media_uploaded(&media, now);
        match self
            .media_repository
            .commit_upload(media.id(), &lease_token, self.clock.now(), event.clone())
            .await
        {
            Ok(media) => Ok(UploadReceipt {
                media,
                event_id: event.id,
            }),
            Err(error) => Err(error.into()),
        }
    }

    /// Commits a Multipart-composed object whose bytes already own a durable
    /// quota reservation. The completion token binds Media creation and
    /// rollback to the active Multipart claim.
    pub async fn upload_multipart_staged(
        &self,
        upload_id: &str,
        completion_token: &str,
        request: &StagedUploadMediaRequest,
    ) -> Result<UploadReceipt, ApplicationError>
    where
        M: S3MultipartRepository,
    {
        let bucket = self
            .bucket_repository
            .find_by_id(request.bucket_id)
            .await?
            .ok_or(ApplicationError::BucketNotFound)?;
        if bucket.application_id() != request.application_id {
            return Err(ApplicationError::BucketDoesNotBelongToApplication);
        }
        bucket.validate_upload(&request.mime, request.size)?;
        if self
            .media_repository
            .find_by_object_key(
                request.application_id,
                request.bucket_id,
                &request.object_key,
            )
            .await?
            .is_some()
        {
            return Err(ApplicationError::ObjectAlreadyExists);
        }

        let now = self.clock.now();
        let media_id = MediaId::new();
        let final_key = format!("objects/{media_id}");
        let expire_at = request.expire_at.or_else(|| {
            bucket
                .policy()
                .default_ttl_seconds()
                .and_then(|seconds| i64::try_from(seconds).ok())
                .map(|seconds| now + time::Duration::seconds(seconds))
        });
        let media = self.build_staged_media(request, media_id, final_key, expire_at, now)?;
        self.media_repository
            .create_uploading_for_multipart(upload_id, completion_token, media.clone())
            .await
            .map_err(map_create_error)?;
        if let Err(error) = self
            .object_store
            .commit_temporary(&request.temporary_key, media.storage_key())
            .await
        {
            return Err(error.into());
        }
        let event = OutboxEvent::media_uploaded(&media, now);
        match self
            .media_repository
            .commit_upload_for_multipart(
                upload_id,
                completion_token,
                media.id(),
                self.clock.now(),
                event.clone(),
            )
            .await
        {
            Ok(media) => Ok(UploadReceipt {
                media,
                event_id: event.id,
            }),
            Err(error) => Err(error.into()),
        }
    }

    fn build_staged_media(
        &self,
        request: &StagedUploadMediaRequest,
        media_id: MediaId,
        storage_key: String,
        expire_at: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> Result<Media, ApplicationError> {
        let system_metadata = SystemMetadata::new(
            &request.mime,
            request.size,
            None,
            None,
            None,
            request.sha256.clone(),
        )?;
        Ok(Media::new(
            NewMedia {
                id: media_id,
                application_id: request.application_id,
                bucket_id: request.bucket_id,
                object_key: request.object_key.clone(),
                original_name: request.original_name.clone(),
                display_name: request.display_name.clone(),
                extension: request.extension.clone(),
                storage_backend: self.object_store.backend_name().to_owned(),
                storage_key,
                visibility_override: request.visibility_override,
                expire_at,
                system_metadata,
                client_metadata: request.metadata.clone(),
            },
            now,
        )?)
    }

    fn build_uploading_media(
        &self,
        request: &UploadMediaRequest,
        media_id: MediaId,
        storage_key: String,
        size: u64,
        expire_at: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> Result<Media, ApplicationError> {
        let digest = Sha256::digest(&request.content);
        let sha256 = hex::encode(digest);
        let system_metadata = SystemMetadata::new(&request.mime, size, None, None, None, sha256)?;
        Ok(Media::new(
            NewMedia {
                id: media_id,
                application_id: request.application_id,
                bucket_id: request.bucket_id,
                object_key: request.object_key.clone(),
                original_name: request.original_name.clone(),
                display_name: request.display_name.clone(),
                extension: request.extension.clone(),
                storage_backend: self.object_store.backend_name().to_owned(),
                storage_key,
                visibility_override: request.visibility_override,
                expire_at,
                system_metadata,
                client_metadata: request.metadata.clone(),
            },
            now,
        )?)
    }

    async fn rollback_before_promotion(
        &self,
        media_id: MediaId,
        temporary_key: &str,
        lease_token: &str,
    ) {
        if run_storage_with_upload_lease(
            &self.media_repository,
            &self.clock,
            media_id,
            lease_token,
            self.object_store.delete(temporary_key),
        )
        .await
        .is_ok()
        {
            let _ = self
                .media_repository
                .abort_upload(media_id, lease_token, self.clock.now())
                .await;
        }
    }
}

enum LeasedStorageError {
    Storage(crate::ObjectStoreError),
    Ownership(RepositoryError),
}

impl LeasedStorageError {
    fn into_application_error(self) -> ApplicationError {
        match self {
            Self::Storage(error) => error.into(),
            Self::Ownership(error) => error.into(),
        }
    }
}

async fn run_storage_with_upload_lease<M, C, F, T>(
    repository: &M,
    clock: &C,
    media_id: MediaId,
    lease_token: &str,
    operation: F,
) -> Result<T, LeasedStorageError>
where
    M: MediaRepository,
    C: Clock,
    F: Future<Output = Result<T, crate::ObjectStoreError>>,
{
    renew_upload_ownership(repository, clock, media_id, lease_token).await?;
    let operation = operation.fuse();
    futures_util::pin_mut!(operation);
    loop {
        let heartbeat = Delay::new(StdDuration::from_secs(MEDIA_UPLOAD_HEARTBEAT_SECONDS)).fuse();
        futures_util::pin_mut!(heartbeat);
        futures_util::select! {
            result = &mut operation => {
                renew_upload_ownership(repository, clock, media_id, lease_token).await?;
                return result.map_err(LeasedStorageError::Storage);
            }
            () = heartbeat => {
                renew_upload_ownership(repository, clock, media_id, lease_token).await?;
            }
        }
    }
}

async fn renew_upload_ownership<M, C>(
    repository: &M,
    clock: &C,
    media_id: MediaId,
    lease_token: &str,
) -> Result<(), LeasedStorageError>
where
    M: MediaRepository,
    C: Clock,
{
    let now = clock.now();
    let leased_until = now + time::Duration::seconds(MEDIA_UPLOAD_LEASE_SECONDS);
    match repository
        .renew_upload_lease(media_id, lease_token, now, leased_until)
        .await
        .map_err(LeasedStorageError::Ownership)?
    {
        true => Ok(()),
        false => Err(LeasedStorageError::Ownership(RepositoryError::Conflict)),
    }
}

fn map_quota_error(error: RepositoryError) -> ApplicationError {
    match error {
        RepositoryError::QuotaExceeded => ApplicationError::QuotaExceeded,
        other => ApplicationError::Repository(other),
    }
}

fn map_create_error(error: RepositoryError) -> ApplicationError {
    match error {
        RepositoryError::Conflict => ApplicationError::ObjectAlreadyExists,
        other => ApplicationError::Repository(other),
    }
}

#[cfg(test)]
mod tests {
    use futures::executor::block_on;
    use mediahub_core::{
        ApplicationId, Bucket, BucketId, BucketPolicy, ClientMetadata, MediaState, OffsetDateTime,
        Visibility,
    };
    use sha2::Digest as _;

    use crate::{
        ApplicationError, FixedClock, InMemoryBucketRepository, InMemoryMediaRepository,
        InMemoryObjectStore, ObjectStore, ObjectStoreError, OutboxRepository, RepositoryError,
    };

    use super::{StagedUploadMediaRequest, UploadMediaRequest, UploadMediaService};

    fn setup() -> (
        ApplicationId,
        BucketId,
        InMemoryObjectStore,
        InMemoryMediaRepository,
        UploadMediaService<
            InMemoryObjectStore,
            InMemoryMediaRepository,
            InMemoryBucketRepository,
            FixedClock,
        >,
    ) {
        let now = OffsetDateTime::UNIX_EPOCH;
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "assets",
            BucketPolicy::unrestricted(Visibility::Private),
            now,
        )
        .expect("fixture bucket");
        let object_store = InMemoryObjectStore::default();
        let media_repository = InMemoryMediaRepository::with_quota(application_id, 1024);
        let bucket_repository = InMemoryBucketRepository::with_bucket(bucket);
        let service = UploadMediaService::new(
            object_store.clone(),
            media_repository.clone(),
            bucket_repository,
            FixedClock::new(now),
        );
        (
            application_id,
            bucket_id,
            object_store,
            media_repository,
            service,
        )
    }

    fn request(
        application_id: ApplicationId,
        bucket_id: BucketId,
        object_key: &str,
    ) -> UploadMediaRequest {
        UploadMediaRequest {
            application_id,
            bucket_id,
            object_key: object_key.to_owned(),
            original_name: Some("avatar.png".to_owned()),
            display_name: "Avatar".to_owned(),
            extension: Some("png".to_owned()),
            mime: "image/png".to_owned(),
            content: vec![137, 80, 78, 71],
            visibility_override: None,
            expire_at: None,
            metadata: ClientMetadata::default(),
        }
    }

    #[test]
    fn upload_activates_media_commits_quota_and_writes_outbox_event() {
        let (application_id, bucket_id, object_store, repository, service) = setup();

        let receipt =
            block_on(service.upload(&request(application_id, bucket_id, "avatars/one.png")))
                .expect("upload succeeds");

        assert_eq!(receipt.media.state(), MediaState::Active);
        assert_eq!(object_store.temporary_count(), 0);
        assert_eq!(object_store.object_count(), 1);
        assert_eq!(
            object_store.object_content(receipt.media.storage_key()),
            Some(vec![137, 80, 78, 71])
        );
        assert_eq!(
            repository.quota(application_id),
            Some(crate::QuotaSnapshot {
                quota_bytes: 1024,
                used_bytes: 4,
                reserved_bytes: 0,
            })
        );
        assert!(repository.outbox_event(&receipt.event_id).is_some());
        assert_eq!(
            block_on(repository.list_pending(OffsetDateTime::UNIX_EPOCH, 10))
                .expect("pending events")
                .len(),
            1
        );
    }

    #[test]
    fn staged_upload_commits_composed_bytes_and_independent_sha256() {
        let (application_id, bucket_id, object_store, repository, service) = setup();
        let content = b"multipart-content";
        block_on(object_store.put_temporary(
            "temporary/multipart-complete",
            content,
            "application/octet-stream",
        ))
        .expect("stage composed content");
        let sha256 = hex::encode(sha2::Sha256::digest(content));
        let receipt = block_on(service.upload_staged(&StagedUploadMediaRequest {
            application_id,
            bucket_id,
            object_key: "multipart/result.bin".to_owned(),
            original_name: Some("result.bin".to_owned()),
            display_name: "result.bin".to_owned(),
            extension: Some("bin".to_owned()),
            mime: "application/octet-stream".to_owned(),
            temporary_key: "temporary/multipart-complete".to_owned(),
            size: content.len() as u64,
            sha256: sha256.clone(),
            visibility_override: Some(Visibility::Public),
            expire_at: None,
            metadata: ClientMetadata::default(),
        }))
        .expect("commit staged upload");

        assert_eq!(receipt.media.sha256(), sha256);
        assert_eq!(
            receipt.media.visibility_override(),
            Some(Visibility::Public)
        );
        assert_eq!(
            object_store.object_content(receipt.media.storage_key()),
            Some(content.to_vec())
        );
        assert_eq!(object_store.temporary_count(), 0);
        assert_eq!(repository.media_count(), 1);
    }

    #[test]
    fn duplicate_object_key_is_rejected_without_overwriting_content() {
        let (application_id, bucket_id, object_store, repository, service) = setup();
        let first =
            block_on(service.upload(&request(application_id, bucket_id, "avatars/one.png")))
                .expect("first upload succeeds");

        let error =
            block_on(service.upload(&request(application_id, bucket_id, "avatars/one.png")))
                .expect_err("duplicate object key is rejected");

        assert!(matches!(error, ApplicationError::ObjectAlreadyExists));
        assert_eq!(object_store.object_count(), 1);
        assert_eq!(repository.media_count(), 1);
        assert_eq!(
            object_store.object_content(first.media.storage_key()),
            Some(vec![137, 80, 78, 71])
        );
    }

    #[test]
    fn temporary_write_failure_aborts_media_and_releases_reservation() {
        let (application_id, bucket_id, object_store, repository, service) = setup();
        object_store.fail_next_put(ObjectStoreError::Unavailable("disk full".to_owned()));

        let error =
            block_on(service.upload(&request(application_id, bucket_id, "avatars/one.png")))
                .expect_err("storage error is returned");

        assert!(matches!(error, ApplicationError::ObjectStore(_)));
        assert_eq!(object_store.temporary_count(), 0);
        assert_eq!(object_store.object_count(), 0);
        assert_eq!(repository.media_count(), 0);
        assert_eq!(
            repository
                .quota(application_id)
                .expect("quota exists")
                .reserved_bytes,
            0
        );
    }

    #[test]
    fn uncertain_commit_failure_keeps_promoted_object_for_reconciliation() {
        let (application_id, bucket_id, object_store, repository, service) = setup();
        repository.fail_next_commit(RepositoryError::Unavailable(
            "database unavailable".to_owned(),
        ));

        let error =
            block_on(service.upload(&request(application_id, bucket_id, "avatars/one.png")))
                .expect_err("database commit error is returned");

        assert!(matches!(error, ApplicationError::Repository(_)));
        assert_eq!(object_store.temporary_count(), 0);
        assert_eq!(object_store.object_count(), 1);
        assert_eq!(repository.media_count(), 1);
        assert_eq!(
            repository.quota(application_id).expect("quota exists"),
            crate::QuotaSnapshot {
                quota_bytes: 1024,
                used_bytes: 0,
                reserved_bytes: 4,
            }
        );
    }

    #[test]
    fn bucket_default_ttl_applies_when_upload_has_no_explicit_expiry() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "ttl-assets",
            BucketPolicy::new(Visibility::Private, Some(60), None, []).expect("policy"),
            now,
        )
        .expect("bucket");
        let service = UploadMediaService::new(
            InMemoryObjectStore::default(),
            InMemoryMediaRepository::with_quota(application_id, 1024),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(now),
        );
        let media = block_on(service.upload(&request(application_id, bucket_id, "ttl.png")))
            .expect("upload")
            .media;
        assert_eq!(media.expire_at(), Some(now + time::Duration::seconds(60)));
    }

    #[test]
    fn explicit_upload_expiry_overrides_bucket_default_ttl() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "ttl-assets",
            BucketPolicy::new(Visibility::Private, Some(60), None, []).expect("policy"),
            now,
        )
        .expect("bucket");
        let service = UploadMediaService::new(
            InMemoryObjectStore::default(),
            InMemoryMediaRepository::with_quota(application_id, 1024),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(now),
        );
        let mut upload = request(application_id, bucket_id, "explicit.png");
        upload.expire_at = Some(now + time::Duration::seconds(30));
        let media = block_on(service.upload(&upload)).expect("upload").media;
        assert_eq!(media.expire_at(), Some(now + time::Duration::seconds(30)));
    }
}
