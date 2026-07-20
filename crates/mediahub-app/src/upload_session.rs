use std::{collections::BTreeMap, fmt};

use mediahub_core::{
    ApplicationId, Bucket, BucketId, ClientMetadata, Media, MediaId, NewMedia, OffsetDateTime,
    SystemMetadata, UploadSession, UploadSessionId, UploadSessionState, Visibility,
};
use time::Duration;

use crate::{
    ApplicationError, BucketRepository, Clock, OutboxEvent, Redacted, RepositoryError,
    UploadSessionCancellation, UploadSessionCompletion, UploadSessionExpiration,
    UploadSessionRepository, UploadSessionStorage,
};

pub const DEFAULT_UPLOAD_SESSION_TTL: Duration = Duration::minutes(15);

/// Transport-neutral input for creating a direct upload intent.
#[derive(Clone)]
pub struct CreateUploadSessionRequest {
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub object_key: String,
    pub original_name: Option<String>,
    pub display_name: String,
    pub extension: Option<String>,
    pub expected_size: u64,
    pub expected_mime: String,
    pub visibility_override: Option<Visibility>,
    pub media_expires_at: Option<OffsetDateTime>,
    pub metadata: ClientMetadata,
}

impl fmt::Debug for CreateUploadSessionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateUploadSessionRequest")
            .field("application_id", &self.application_id)
            .field("bucket_id", &self.bucket_id)
            .field("object_key", &self.object_key)
            .field("original_name", &self.original_name)
            .field("display_name", &self.display_name)
            .field("extension", &self.extension)
            .field("expected_size", &self.expected_size)
            .field("expected_mime", &self.expected_mime)
            .field("visibility_override", &self.visibility_override)
            .field("media_expires_at", &self.media_expires_at)
            .field("metadata", &Redacted(&self.metadata))
            .finish()
    }
}

/// Opaque upload target returned to a transport layer. `url` can represent a
/// MediaHub endpoint, S3-compatible presigned URL, or R2 upload target.
#[derive(Clone, PartialEq, Eq)]
pub struct UploadTarget {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub expires_at: OffsetDateTime,
}

impl fmt::Debug for UploadTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UploadTarget")
            .field("method", &self.method)
            .field("url", &Redacted(&self.url))
            .field("headers", &Redacted(&self.headers))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Adapter-created storage location and client upload target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedUpload {
    pub target: UploadTarget,
    pub storage_backend: String,
    pub storage_key: String,
}

/// Independently verified object facts returned by `UploadSessionStorage`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredUpload {
    pub size: u64,
    pub mime: String,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct UploadSessionReceipt {
    pub session: UploadSession,
    pub target: UploadTarget,
}

/// Transport-neutral input for finalizing a client upload. The submitted
/// checksum is compared to independently inspected object facts.
#[derive(Clone, Debug)]
pub struct CompleteUploadSessionRequest {
    pub application_id: ApplicationId,
    pub upload_session_id: UploadSessionId,
    pub sha256: String,
}

#[derive(Clone, Debug)]
pub struct CompletedUploadSessionReceipt {
    pub session: UploadSession,
    pub media: Media,
    pub event_id: String,
    pub already_completed: bool,
}

#[derive(Clone, Debug)]
pub struct CancelUploadSessionRequest {
    pub application_id: ApplicationId,
    pub upload_session_id: UploadSessionId,
}

#[derive(Clone, Debug)]
pub struct CancelUploadSessionReceipt {
    pub session: UploadSession,
    pub already_cancelled: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ExpireUploadSessionsReceipt {
    pub expired_sessions: Vec<UploadSession>,
}

/// Coordinates backend-neutral direct uploads. The storage adapter owns target
/// generation and inspection; the repository owns durable state transitions
/// and quota accounting. No HTTP or async runtime type leaks into this API.
pub struct UploadSessionService<S, R, B, C> {
    storage: S,
    repository: R,
    bucket_repository: B,
    clock: C,
    session_ttl: Duration,
}

impl<S, R, B, C> UploadSessionService<S, R, B, C>
where
    S: UploadSessionStorage,
    R: UploadSessionRepository,
    B: BucketRepository,
    C: Clock,
{
    #[must_use]
    pub fn new(storage: S, repository: R, bucket_repository: B, clock: C) -> Self {
        Self::with_session_ttl(
            storage,
            repository,
            bucket_repository,
            clock,
            DEFAULT_UPLOAD_SESSION_TTL,
        )
    }

    #[must_use]
    pub fn with_session_ttl(
        storage: S,
        repository: R,
        bucket_repository: B,
        clock: C,
        session_ttl: Duration,
    ) -> Self {
        Self {
            storage,
            repository,
            bucket_repository,
            clock,
            session_ttl,
        }
    }

    pub async fn create(
        &self,
        request: &CreateUploadSessionRequest,
    ) -> Result<UploadSessionReceipt, ApplicationError> {
        let receipt = self.prepare(request).await?;
        if let Err(error) = self
            .repository
            .create_upload_session(receipt.session.clone())
            .await
        {
            let _ = self.storage.abort_upload(&receipt.session).await;
            return Err(map_create_session_error(error));
        }
        Ok(receipt)
    }

    /// Builds a backend-owned target and validated Session without persisting
    /// quota or state. Runtime adapters use this only when they must combine
    /// Session creation with another durable record in one transaction.
    pub async fn prepare(
        &self,
        request: &CreateUploadSessionRequest,
    ) -> Result<UploadSessionReceipt, ApplicationError> {
        let bucket = self
            .owned_bucket(request.application_id, request.bucket_id)
            .await?;
        bucket.validate_upload(&request.expected_mime, request.expected_size)?;

        let now = self.clock.now();
        let upload_session_id = UploadSessionId::new();
        let media_id = MediaId::new();
        let session_expires_at = now + self.session_ttl;
        let prepared = self
            .storage
            .prepare_upload(
                upload_session_id,
                media_id,
                request.expected_size,
                &request.expected_mime,
                session_expires_at,
            )
            .await?;
        let media_expires_at = request.media_expires_at.or_else(|| {
            bucket
                .policy()
                .default_ttl_seconds()
                .and_then(|seconds| i64::try_from(seconds).ok())
                .map(|seconds| now + Duration::seconds(seconds))
        });
        let session = UploadSession::new(
            mediahub_core::NewUploadSession {
                id: upload_session_id,
                media_id,
                application_id: request.application_id,
                bucket_id: request.bucket_id,
                object_key: request.object_key.clone(),
                original_name: request.original_name.clone(),
                display_name: request.display_name.clone(),
                extension: request.extension.clone(),
                expected_size: request.expected_size,
                expected_mime: request.expected_mime.clone(),
                storage_backend: prepared.storage_backend,
                storage_key: prepared.storage_key,
                visibility_override: request.visibility_override,
                media_expires_at,
                client_metadata: request.metadata.clone(),
                session_expires_at,
            },
            now,
        )?;
        Ok(UploadSessionReceipt {
            session,
            target: prepared.target,
        })
    }

    pub async fn complete(
        &self,
        request: &CompleteUploadSessionRequest,
    ) -> Result<CompletedUploadSessionReceipt, ApplicationError> {
        let session = self
            .owned_session(request.application_id, request.upload_session_id)
            .await?;
        match session.state() {
            UploadSessionState::Completed => return self.completed_receipt(session, true).await,
            UploadSessionState::Cancelled => return Err(ApplicationError::UploadSessionCancelled),
            UploadSessionState::Expired => return Err(ApplicationError::UploadSessionExpired),
            UploadSessionState::Pending => {}
        }

        let initial_now = self.clock.now();
        if session.is_expired_at(initial_now) {
            self.expire_one(&session, initial_now).await?;
            return Err(ApplicationError::UploadSessionExpired);
        }

        let bucket = self
            .owned_bucket(session.application_id(), session.bucket_id())
            .await?;
        let stored = self
            .storage
            .inspect_upload(&session)
            .await
            .map_err(|error| match error {
                crate::ObjectStoreError::NotFound => {
                    ApplicationError::UploadSessionVerificationFailed
                }
                error => ApplicationError::ObjectStore(error),
            })?;
        // Inspection can be slow enough to cross the session TTL. Re-read the
        // clock after the remote operation and fence completion before any
        // durable media/session transition.
        let verification_now = self.clock.now();
        if session.is_expired_at(verification_now) {
            self.expire_one(&session, verification_now).await?;
            return Err(ApplicationError::UploadSessionExpired);
        }
        let final_storage_key = format!("objects/{}", session.media_id());
        let media = match self.build_media(
            &session,
            &bucket,
            &stored,
            &request.sha256,
            &final_storage_key,
            verification_now,
        ) {
            Ok(media) => media,
            Err(error) => {
                self.cancel_uncommitted(&session, verification_now).await?;
                return Err(error);
            }
        };
        // Storage promotion is deliberately separate from the DB transaction.
        // A failed/uncertain promotion leaves the session pending so a later
        // completion attempt can recover it without deleting a possible final
        // object.
        self.storage
            .finalize_upload(&session, &final_storage_key)
            .await?;
        let commit_now = self.clock.now();
        if session.is_expired_at(commit_now) {
            self.expire_one(&session, commit_now).await?;
            return Err(ApplicationError::UploadSessionExpired);
        }
        let event = OutboxEvent::media_uploaded(&media, commit_now);
        match self
            .repository
            .complete_upload_session(session.id(), media, commit_now, event)
            .await?
        {
            UploadSessionCompletion::Completed(media) => {
                let session = self
                    .repository
                    .find_upload_session(request.upload_session_id)
                    .await?
                    .ok_or(ApplicationError::UploadSessionNotFound)?;
                Ok(CompletedUploadSessionReceipt {
                    event_id: format!("media.uploaded:{}", media.id()),
                    session,
                    media,
                    already_completed: false,
                })
            }
            UploadSessionCompletion::AlreadyCompleted(media) => {
                let session = self
                    .repository
                    .find_upload_session(request.upload_session_id)
                    .await?
                    .ok_or(ApplicationError::UploadSessionNotFound)?;
                Ok(CompletedUploadSessionReceipt {
                    event_id: format!("media.uploaded:{}", media.id()),
                    session,
                    media,
                    already_completed: true,
                })
            }
            UploadSessionCompletion::Cancelled => Err(ApplicationError::UploadSessionCancelled),
            UploadSessionCompletion::Expired => Err(ApplicationError::UploadSessionExpired),
        }
    }

    pub async fn cancel(
        &self,
        request: &CancelUploadSessionRequest,
    ) -> Result<CancelUploadSessionReceipt, ApplicationError> {
        let session = self
            .owned_session(request.application_id, request.upload_session_id)
            .await?;
        match self
            .repository
            .cancel_upload_session(session.id(), self.clock.now())
            .await?
        {
            UploadSessionCancellation::Cancelled(session) => {
                self.cleanup_if_target_expired(&session).await?;
                Ok(CancelUploadSessionReceipt {
                    session,
                    already_cancelled: false,
                })
            }
            UploadSessionCancellation::AlreadyCancelled(session) => {
                self.cleanup_if_target_expired(&session).await?;
                Ok(CancelUploadSessionReceipt {
                    session,
                    already_cancelled: true,
                })
            }
            UploadSessionCancellation::Completed => {
                Err(ApplicationError::UploadSessionAlreadyCompleted)
            }
            UploadSessionCancellation::Expired => Err(ApplicationError::UploadSessionExpired),
        }
    }

    pub async fn expire_due(
        &self,
        limit: usize,
    ) -> Result<ExpireUploadSessionsReceipt, ApplicationError> {
        let expired_sessions = self
            .repository
            .expire_upload_sessions(self.clock.now(), limit)
            .await?;
        let mut first_error = None;
        for session in &expired_sessions {
            // Durable expiry and quota release have already succeeded. Object
            // cleanup is idempotent and is acknowledged only after success;
            // failed cleanup leaves this terminal row eligible for retry.
            if let Err(error) = self.cleanup_storage(session).await {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(ExpireUploadSessionsReceipt { expired_sessions })
    }

    async fn owned_bucket(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
    ) -> Result<Bucket, ApplicationError> {
        let bucket = self
            .bucket_repository
            .find_by_id(bucket_id)
            .await?
            .ok_or(ApplicationError::BucketNotFound)?;
        if bucket.application_id() != application_id {
            return Err(ApplicationError::BucketDoesNotBelongToApplication);
        }
        Ok(bucket)
    }

    async fn owned_session(
        &self,
        application_id: ApplicationId,
        upload_session_id: UploadSessionId,
    ) -> Result<UploadSession, ApplicationError> {
        let session = self
            .repository
            .find_upload_session(upload_session_id)
            .await?
            .ok_or(ApplicationError::UploadSessionNotFound)?;
        if session.application_id() != application_id {
            return Err(ApplicationError::UploadSessionDoesNotBelongToApplication);
        }
        Ok(session)
    }

    async fn completed_receipt(
        &self,
        session: UploadSession,
        already_completed: bool,
    ) -> Result<CompletedUploadSessionReceipt, ApplicationError> {
        let media = self
            .repository
            .completed_upload_media(session.id())
            .await?
            .ok_or_else(|| {
                ApplicationError::Repository(RepositoryError::Invariant(
                    "completed upload session has no media".to_owned(),
                ))
            })?;
        Ok(CompletedUploadSessionReceipt {
            event_id: format!("media.uploaded:{}", media.id()),
            session,
            media,
            already_completed,
        })
    }

    async fn expire_one(
        &self,
        session: &UploadSession,
        now: OffsetDateTime,
    ) -> Result<(), ApplicationError> {
        match self
            .repository
            .expire_upload_session(session.id(), now)
            .await?
        {
            UploadSessionExpiration::Expired(session)
            | UploadSessionExpiration::AlreadyExpired(session) => {
                self.cleanup_storage(&session).await?;
                Ok(())
            }
            UploadSessionExpiration::Completed => {
                Err(ApplicationError::UploadSessionAlreadyCompleted)
            }
            UploadSessionExpiration::Cancelled => Err(ApplicationError::UploadSessionCancelled),
            UploadSessionExpiration::NotDue => Ok(()),
        }
    }

    async fn cancel_uncommitted(
        &self,
        session: &UploadSession,
        now: OffsetDateTime,
    ) -> Result<(), ApplicationError> {
        match self
            .repository
            .cancel_upload_session(session.id(), now)
            .await?
        {
            UploadSessionCancellation::Cancelled(session)
            | UploadSessionCancellation::AlreadyCancelled(session) => {
                self.cleanup_if_target_expired(&session).await?;
            }
            UploadSessionCancellation::Completed | UploadSessionCancellation::Expired => {}
        }
        Ok(())
    }

    async fn cleanup_storage(&self, session: &UploadSession) -> Result<(), ApplicationError> {
        self.storage.abort_upload(session).await?;
        self.repository
            .complete_upload_session_cleanup(session.id())
            .await?;
        Ok(())
    }

    async fn cleanup_if_target_expired(
        &self,
        session: &UploadSession,
    ) -> Result<(), ApplicationError> {
        if session.is_expired_at(self.clock.now()) {
            self.cleanup_storage(session).await?;
        }
        Ok(())
    }

    fn build_media(
        &self,
        session: &UploadSession,
        bucket: &Bucket,
        stored: &StoredUpload,
        submitted_sha256: &str,
        final_storage_key: &str,
        now: OffsetDateTime,
    ) -> Result<Media, ApplicationError> {
        let system_metadata =
            SystemMetadata::new(&stored.mime, stored.size, None, None, None, &stored.sha256)?;
        if stored.size != session.expected_size()
            || system_metadata.mime() != session.expected_mime()
            || !system_metadata
                .sha256()
                .eq_ignore_ascii_case(submitted_sha256)
        {
            return Err(ApplicationError::UploadSessionVerificationFailed);
        }
        bucket.validate_upload(system_metadata.mime(), system_metadata.size())?;

        Ok(Media::new(
            NewMedia {
                id: session.media_id(),
                application_id: session.application_id(),
                bucket_id: session.bucket_id(),
                object_key: session.object_key().to_owned(),
                original_name: session.original_name().map(str::to_owned),
                display_name: session.display_name().to_owned(),
                extension: session.extension().map(str::to_owned),
                storage_backend: session.storage_backend().to_owned(),
                storage_key: final_storage_key.to_owned(),
                visibility_override: session.visibility_override(),
                expire_at: session.media_expires_at(),
                system_metadata,
                client_metadata: session.client_metadata().clone(),
            },
            now,
        )?)
    }
}

fn map_create_session_error(error: RepositoryError) -> ApplicationError {
    match error {
        RepositoryError::QuotaExceeded => ApplicationError::QuotaExceeded,
        RepositoryError::Conflict => ApplicationError::ObjectAlreadyExists,
        other => ApplicationError::Repository(other),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use futures::executor::block_on;
    use mediahub_core::{BucketPolicy, UploadSessionState};
    use sha2::{Digest, Sha256};

    use crate::{
        Clock, FixedClock, InMemoryBucketRepository, InMemoryMediaRepository, InMemoryObjectStore,
        ObjectStoreError, PreparedUpload, QuotaSnapshot, StoredUpload, UploadSessionExpiration,
        UploadSessionRepository, UploadSessionStorage,
    };

    use super::*;

    #[derive(Clone)]
    struct AdvancingClock {
        now: Arc<Mutex<OffsetDateTime>>,
    }

    impl Clock for AdvancingClock {
        fn now(&self) -> OffsetDateTime {
            *self.now.lock().expect("clock lock")
        }
    }

    #[derive(Clone)]
    struct AdvancingInspectStorage {
        inner: InMemoryObjectStore,
        clock: Arc<Mutex<OffsetDateTime>>,
    }

    #[async_trait]
    impl UploadSessionStorage for AdvancingInspectStorage {
        async fn prepare_upload(
            &self,
            upload_session_id: UploadSessionId,
            media_id: MediaId,
            expected_size: u64,
            expected_mime: &str,
            expires_at: OffsetDateTime,
        ) -> Result<PreparedUpload, ObjectStoreError> {
            self.inner
                .prepare_upload(
                    upload_session_id,
                    media_id,
                    expected_size,
                    expected_mime,
                    expires_at,
                )
                .await
        }

        async fn inspect_upload(
            &self,
            session: &UploadSession,
        ) -> Result<StoredUpload, ObjectStoreError> {
            let result = self.inner.inspect_upload(session).await;
            *self.clock.lock().expect("clock lock") += Duration::seconds(2);
            result
        }

        async fn abort_upload(&self, session: &UploadSession) -> Result<(), ObjectStoreError> {
            self.inner.abort_upload(session).await
        }
    }

    fn setup() -> (
        ApplicationId,
        InMemoryObjectStore,
        InMemoryMediaRepository,
        UploadSessionService<
            InMemoryObjectStore,
            InMemoryMediaRepository,
            InMemoryBucketRepository,
            FixedClock,
        >,
    ) {
        let now = OffsetDateTime::UNIX_EPOCH;
        let application_id = ApplicationId::new();
        let bucket = Bucket::new(
            BucketId::new(),
            application_id,
            "uploads",
            BucketPolicy::unrestricted(Visibility::Private),
            now,
        )
        .expect("fixture bucket");
        let storage = InMemoryObjectStore::default();
        let repository = InMemoryMediaRepository::with_quota(application_id, 32);
        let service = UploadSessionService::new(
            storage.clone(),
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(now),
        );
        (application_id, storage, repository, service)
    }

    fn request(application_id: ApplicationId, bucket_id: BucketId) -> CreateUploadSessionRequest {
        CreateUploadSessionRequest {
            application_id,
            bucket_id,
            object_key: "avatars/example.png".to_owned(),
            original_name: Some("example.png".to_owned()),
            display_name: "Example".to_owned(),
            extension: Some("png".to_owned()),
            expected_size: 4,
            expected_mime: "image/png".to_owned(),
            visibility_override: None,
            media_expires_at: None,
            metadata: ClientMetadata::default(),
        }
    }

    #[test]
    fn completion_is_idempotent_and_transfers_reserved_quota_once() {
        let (application_id, storage, repository, _service) = setup();
        let bucket_id = BucketId::new();
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "completion",
            BucketPolicy::unrestricted(Visibility::Private),
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("fixture bucket");
        let service = UploadSessionService::new(
            storage.clone(),
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(OffsetDateTime::UNIX_EPOCH),
        );
        let receipt = block_on(service.create(&request(application_id, bucket_id)))
            .expect("upload session created");
        assert_eq!(
            repository.quota(application_id),
            Some(QuotaSnapshot {
                quota_bytes: 32,
                used_bytes: 0,
                reserved_bytes: 4,
            })
        );

        let content = [137, 80, 78, 71];
        storage
            .put_upload(&receipt.session, &content, "image/png")
            .expect("client upload");
        let completion = CompleteUploadSessionRequest {
            application_id,
            upload_session_id: receipt.session.id(),
            sha256: hex::encode(Sha256::digest(content)),
        };
        let first = block_on(service.complete(&completion)).expect("first completion");
        let second = block_on(service.complete(&completion)).expect("replayed completion");

        assert!(!first.already_completed);
        assert!(second.already_completed);
        assert_eq!(first.media.id(), second.media.id());
        assert_eq!(first.session.state(), UploadSessionState::Completed);
        assert_eq!(
            repository.quota(application_id),
            Some(QuotaSnapshot {
                quota_bytes: 32,
                used_bytes: 4,
                reserved_bytes: 0,
            })
        );
    }

    #[test]
    fn cancellation_is_idempotent_and_releases_reserved_quota_once() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "cancel",
            BucketPolicy::unrestricted(Visibility::Private),
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("fixture bucket");
        let storage = InMemoryObjectStore::default();
        let repository = InMemoryMediaRepository::with_quota(application_id, 32);
        let service = UploadSessionService::new(
            storage,
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(OffsetDateTime::UNIX_EPOCH),
        );
        let receipt = block_on(service.create(&request(application_id, bucket_id)))
            .expect("upload session created");
        let cancellation = CancelUploadSessionRequest {
            application_id,
            upload_session_id: receipt.session.id(),
        };

        let first = block_on(service.cancel(&cancellation)).expect("first cancellation");
        let second = block_on(service.cancel(&cancellation)).expect("replayed cancellation");

        assert!(!first.already_cancelled);
        assert!(second.already_cancelled);
        assert_eq!(second.session.state(), UploadSessionState::Cancelled);
        assert_eq!(
            repository.quota(application_id),
            Some(QuotaSnapshot {
                quota_bytes: 32,
                used_bytes: 0,
                reserved_bytes: 0,
            })
        );
    }

    #[test]
    fn cancelled_session_is_not_cleaned_or_acknowledged_before_target_expiry() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let now = OffsetDateTime::UNIX_EPOCH;
        let session_ttl = Duration::seconds(60);
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "cancel-cleanup-delay",
            BucketPolicy::unrestricted(Visibility::Private),
            now,
        )
        .expect("fixture bucket");
        let storage = InMemoryObjectStore::default();
        let repository = InMemoryMediaRepository::with_quota(application_id, 32);
        let service = UploadSessionService::with_session_ttl(
            storage.clone(),
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket.clone()),
            FixedClock::new(now),
            session_ttl,
        );
        let receipt = block_on(service.create(&request(application_id, bucket_id)))
            .expect("upload session created");
        storage
            .put_upload(&receipt.session, b"data", "image/png")
            .expect("client upload");

        block_on(service.cancel(&CancelUploadSessionRequest {
            application_id,
            upload_session_id: receipt.session.id(),
        }))
        .expect("cancel upload session");
        let before_expiry = UploadSessionService::with_session_ttl(
            storage.clone(),
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(receipt.session.session_expires_at() - Duration::seconds(1)),
            session_ttl,
        );
        let scan = block_on(before_expiry.expire_due(10)).expect("pre-expiry cleanup scan");

        assert!(scan.expired_sessions.is_empty());
        assert!(
            storage
                .object_content(receipt.session.storage_key())
                .is_some()
        );
        assert!(
            block_on(repository.complete_upload_session_cleanup(receipt.session.id()))
                .expect("cleanup was not acknowledged early")
        );
    }

    #[test]
    fn completed_session_is_cleaned_after_target_expiry_without_deleting_final_object() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let now = OffsetDateTime::UNIX_EPOCH;
        let session_ttl = Duration::seconds(60);
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "completed-cleanup-delay",
            BucketPolicy::unrestricted(Visibility::Private),
            now,
        )
        .expect("fixture bucket");
        let storage = InMemoryObjectStore::default();
        let repository = InMemoryMediaRepository::with_quota(application_id, 32);
        let service = UploadSessionService::with_session_ttl(
            storage.clone(),
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket.clone()),
            FixedClock::new(now),
            session_ttl,
        );
        let receipt = block_on(service.create(&request(application_id, bucket_id)))
            .expect("upload session created");
        storage
            .put_upload(&receipt.session, b"data", "image/png")
            .expect("client upload");
        block_on(service.complete(&CompleteUploadSessionRequest {
            application_id,
            upload_session_id: receipt.session.id(),
            sha256: hex::encode(Sha256::digest(b"data")),
        }))
        .expect("complete upload session");

        let before_expiry = UploadSessionService::with_session_ttl(
            storage.clone(),
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket.clone()),
            FixedClock::new(receipt.session.session_expires_at() - Duration::seconds(1)),
            session_ttl,
        );
        assert!(
            block_on(before_expiry.expire_due(10))
                .expect("pre-expiry cleanup scan")
                .expired_sessions
                .is_empty()
        );

        let after_expiry = UploadSessionService::with_session_ttl(
            storage.clone(),
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(receipt.session.session_expires_at()),
            session_ttl,
        );
        let scan = block_on(after_expiry.expire_due(10)).expect("post-expiry cleanup scan");

        assert_eq!(scan.expired_sessions.len(), 1);
        assert_eq!(
            scan.expired_sessions[0].state(),
            UploadSessionState::Completed
        );
        assert_eq!(
            storage.object_content(receipt.session.storage_key()),
            Some(b"data".to_vec())
        );
        assert!(
            !block_on(repository.complete_upload_session_cleanup(receipt.session.id()))
                .expect("cleanup was already acknowledged")
        );
    }

    #[test]
    fn expiry_is_idempotent_and_releases_reserved_quota_once() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "expiry",
            BucketPolicy::unrestricted(Visibility::Private),
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("fixture bucket");
        let storage = InMemoryObjectStore::default();
        let repository = InMemoryMediaRepository::with_quota(application_id, 32);
        let service = UploadSessionService::new(
            storage,
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(OffsetDateTime::UNIX_EPOCH),
        );
        let receipt = block_on(service.create(&request(application_id, bucket_id)))
            .expect("upload session created");
        let expires_at = receipt.session.session_expires_at();

        let first = block_on(repository.expire_upload_session(receipt.session.id(), expires_at))
            .expect("first expiry");
        let second = block_on(
            repository
                .expire_upload_session(receipt.session.id(), expires_at + Duration::seconds(1)),
        )
        .expect("replayed expiry");

        assert!(matches!(first, UploadSessionExpiration::Expired(_)));
        assert!(matches!(second, UploadSessionExpiration::AlreadyExpired(_)));
        assert_eq!(
            repository.quota(application_id),
            Some(QuotaSnapshot {
                quota_bytes: 32,
                used_bytes: 0,
                reserved_bytes: 0,
            })
        );
    }

    #[test]
    fn bucket_default_media_ttl_is_persisted_and_explicit_expiry_wins() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "ttl",
            BucketPolicy::new(Visibility::Private, Some(60), None, [] as [String; 0])
                .expect("valid policy"),
            now,
        )
        .expect("fixture bucket");
        let service = UploadSessionService::new(
            InMemoryObjectStore::default(),
            InMemoryMediaRepository::with_quota(application_id, 32),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(now),
        );

        let defaulted = block_on(service.create(&request(application_id, bucket_id)))
            .expect("defaulted session");
        assert_eq!(
            defaulted.session.media_expires_at(),
            Some(now + Duration::seconds(60))
        );

        let mut explicit_request = request(application_id, bucket_id);
        explicit_request.object_key = "avatars/explicit.png".to_owned();
        explicit_request.media_expires_at = Some(now + Duration::seconds(120));
        let explicit = block_on(service.create(&explicit_request)).expect("explicit session");
        assert_eq!(
            explicit.session.media_expires_at(),
            explicit_request.media_expires_at
        );
    }

    #[test]
    fn completion_without_uploaded_content_is_a_verification_failure() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "missing-content",
            BucketPolicy::unrestricted(Visibility::Private),
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("fixture bucket");
        let service = UploadSessionService::new(
            InMemoryObjectStore::default(),
            InMemoryMediaRepository::with_quota(application_id, 32),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(OffsetDateTime::UNIX_EPOCH),
        );
        let receipt = block_on(service.create(&request(application_id, bucket_id)))
            .expect("upload session created");
        let result = block_on(service.complete(&CompleteUploadSessionRequest {
            application_id,
            upload_session_id: receipt.session.id(),
            sha256: "0".repeat(64),
        }));

        assert!(matches!(
            result,
            Err(ApplicationError::UploadSessionVerificationFailed)
        ));
    }

    #[test]
    fn completion_rechecks_expiry_after_slow_object_inspection() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let now = OffsetDateTime::UNIX_EPOCH;
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "slow-inspection",
            BucketPolicy::unrestricted(Visibility::Private),
            now,
        )
        .expect("fixture bucket");
        let clock_now = Arc::new(Mutex::new(now));
        let inner = InMemoryObjectStore::default();
        let storage = AdvancingInspectStorage {
            inner: inner.clone(),
            clock: clock_now.clone(),
        };
        let repository = InMemoryMediaRepository::with_quota(application_id, 32);
        let service = UploadSessionService::with_session_ttl(
            storage,
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket),
            AdvancingClock { now: clock_now },
            Duration::seconds(1),
        );
        let receipt = block_on(service.create(&request(application_id, bucket_id)))
            .expect("upload session created");
        inner
            .put_upload(&receipt.session, b"data", "image/png")
            .expect("client upload");

        let result = block_on(service.complete(&CompleteUploadSessionRequest {
            application_id,
            upload_session_id: receipt.session.id(),
            sha256: hex::encode(Sha256::digest(b"data")),
        }));

        assert!(matches!(
            result,
            Err(ApplicationError::UploadSessionExpired)
        ));
        assert_eq!(
            repository
                .upload_session(receipt.session.id())
                .expect("session")
                .state(),
            UploadSessionState::Expired
        );
    }

    #[test]
    fn failed_expired_object_cleanup_is_retried_on_the_next_scan() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let now = OffsetDateTime::UNIX_EPOCH;
        let bucket = Bucket::new(
            bucket_id,
            application_id,
            "cleanup-retry",
            BucketPolicy::unrestricted(Visibility::Private),
            now,
        )
        .expect("fixture bucket");
        let storage = InMemoryObjectStore::default();
        let repository = InMemoryMediaRepository::with_quota(application_id, 32);
        let service = UploadSessionService::with_session_ttl(
            storage.clone(),
            repository.clone(),
            InMemoryBucketRepository::with_bucket(bucket),
            FixedClock::new(now),
            Duration::seconds(1),
        );
        let receipt = block_on(service.create(&request(application_id, bucket_id)))
            .expect("upload session created");
        let expires_at = receipt.session.session_expires_at();

        storage.fail_next_abort(ObjectStoreError::Unavailable("temporary outage".to_owned()));
        assert!(matches!(
            block_on(repository.expire_upload_sessions(expires_at, 10)),
            Ok(sessions) if sessions.len() == 1
        ));
        let cleanup_service = UploadSessionService::with_session_ttl(
            storage,
            repository.clone(),
            service.bucket_repository.clone(),
            FixedClock::new(expires_at),
            Duration::seconds(1),
        );
        let first = block_on(cleanup_service.expire_due(10));
        assert!(
            matches!(first, Err(ApplicationError::ObjectStore(_))),
            "unexpected cleanup result: {first:?}"
        );

        let second = block_on(cleanup_service.expire_due(10)).expect("retry cleanup");
        assert_eq!(second.expired_sessions.len(), 1);
        assert!(
            !block_on(repository.complete_upload_session_cleanup(receipt.session.id()))
                .expect("cleanup ack")
        );
    }
}
