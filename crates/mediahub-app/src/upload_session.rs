use std::collections::BTreeMap;

use mediahub_core::{
    ApplicationId, Bucket, BucketId, ClientMetadata, Media, MediaId, NewMedia, OffsetDateTime,
    SystemMetadata, UploadSession, UploadSessionId, UploadSessionState, Visibility,
};
use time::Duration;

use crate::{
    ApplicationError, BucketRepository, Clock, OutboxEvent, RepositoryError,
    UploadSessionCancellation, UploadSessionCompletion, UploadSessionExpiration,
    UploadSessionRepository, UploadSessionStorage,
};

pub const DEFAULT_UPLOAD_SESSION_TTL: Duration = Duration::minutes(15);

/// Transport-neutral input for creating a direct upload intent.
#[derive(Clone, Debug)]
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

/// Opaque upload target returned to a transport layer. `url` can represent a
/// MediaHub endpoint, S3-compatible presigned URL, or R2 upload target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UploadTarget {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub expires_at: OffsetDateTime,
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

        let now = self.clock.now();
        if session.is_expired_at(now) {
            self.expire_one(&session, now).await?;
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
        let media = match self.build_media(&session, &bucket, &stored, &request.sha256, now) {
            Ok(media) => media,
            Err(error) => {
                self.cancel_uncommitted(&session, now).await?;
                return Err(error);
            }
        };
        let event = OutboxEvent::media_uploaded(&media, now);
        match self
            .repository
            .complete_upload_session(session.id(), media, now, event)
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
                let _ = self.storage.abort_upload(&session).await;
                Ok(CancelUploadSessionReceipt {
                    session,
                    already_cancelled: false,
                })
            }
            UploadSessionCancellation::AlreadyCancelled(session) => {
                let _ = self.storage.abort_upload(&session).await;
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
        for session in &expired_sessions {
            // Durable expiry and quota release have already succeeded. Object
            // cleanup is idempotent and can be retried by a later worker pass.
            let _ = self.storage.abort_upload(session).await;
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
                let _ = self.storage.abort_upload(&session).await;
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
                let _ = self.storage.abort_upload(&session).await;
            }
            UploadSessionCancellation::Completed | UploadSessionCancellation::Expired => {}
        }
        Ok(())
    }

    fn build_media(
        &self,
        session: &UploadSession,
        bucket: &Bucket,
        stored: &StoredUpload,
        submitted_sha256: &str,
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
                storage_key: session.storage_key().to_owned(),
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
    use futures::executor::block_on;
    use mediahub_core::{BucketPolicy, UploadSessionState};
    use sha2::{Digest, Sha256};

    use crate::{
        FixedClock, InMemoryBucketRepository, InMemoryMediaRepository, InMemoryObjectStore,
        QuotaSnapshot, UploadSessionExpiration, UploadSessionRepository,
    };

    use super::*;

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
}
