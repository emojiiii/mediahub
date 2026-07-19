// In-memory media repository and quota state.

#[derive(Clone, Default)]
pub struct InMemoryMediaRepository {
    state: Arc<Mutex<MediaRepositoryState>>,
}

#[derive(Default)]
struct MediaRepositoryState {
    quotas: HashMap<ApplicationId, QuotaSnapshot>,
    media: HashMap<MediaId, Media>,
    locations: HashMap<(ApplicationId, BucketId, String), MediaId>,
    upload_sessions: HashMap<UploadSessionId, UploadSession>,
    upload_session_locations: HashMap<(ApplicationId, BucketId, String), UploadSessionId>,
    completed_upload_media: HashMap<UploadSessionId, MediaId>,
    outbox: HashMap<String, OutboxEvent>,
    fail_next_commit: Option<RepositoryError>,
}

impl InMemoryMediaRepository {
    #[must_use]
    pub fn with_quota(application_id: ApplicationId, quota_bytes: u64) -> Self {
        let repository = Self::default();
        repository.set_quota(application_id, quota_bytes);
        repository
    }

    pub fn set_quota(&self, application_id: ApplicationId, quota_bytes: u64) {
        self.lock()
            .expect("in-memory media repository lock")
            .quotas
            .insert(
                application_id,
                QuotaSnapshot {
                    quota_bytes,
                    used_bytes: 0,
                    reserved_bytes: 0,
                },
            );
    }

    #[must_use]
    pub fn quota(&self, application_id: ApplicationId) -> Option<QuotaSnapshot> {
        self.lock()
            .expect("in-memory media repository lock")
            .quotas
            .get(&application_id)
            .copied()
    }

    #[must_use]
    pub fn media(&self, media_id: MediaId) -> Option<Media> {
        self.lock()
            .expect("in-memory media repository lock")
            .media
            .get(&media_id)
            .cloned()
    }

    #[must_use]
    pub fn media_count(&self) -> usize {
        self.lock()
            .expect("in-memory media repository lock")
            .media
            .len()
    }

    #[must_use]
    pub fn upload_session(&self, upload_session_id: UploadSessionId) -> Option<UploadSession> {
        self.lock()
            .expect("in-memory media repository lock")
            .upload_sessions
            .get(&upload_session_id)
            .cloned()
    }

    #[must_use]
    pub fn outbox_event(&self, event_id: &str) -> Option<OutboxEvent> {
        self.lock()
            .expect("in-memory media repository lock")
            .outbox
            .get(event_id)
            .cloned()
    }

    pub fn fail_next_commit(&self, error: RepositoryError) {
        self.lock()
            .expect("in-memory media repository lock")
            .fail_next_commit = Some(error);
    }

    fn lock(&self) -> Result<MutexGuard<'_, MediaRepositoryState>, RepositoryError> {
        self.state.lock().map_err(|_| {
            RepositoryError::Unavailable("in-memory media repository lock poisoned".into())
        })
    }

    fn release_session_reservation(
        state: &mut MediaRepositoryState,
        session: &UploadSession,
    ) -> Result<(), RepositoryError> {
        let quota = state
            .quotas
            .get_mut(&session.application_id())
            .ok_or(RepositoryError::NotFound)?;
        if quota.reserved_bytes < session.reserved_bytes() {
            return Err(RepositoryError::Invariant(
                "upload session reservation is absent or smaller than expected size".into(),
            ));
        }
        quota.reserved_bytes -= session.reserved_bytes();
        Ok(())
    }

    fn session_location(session: &UploadSession) -> (ApplicationId, BucketId, String) {
        (
            session.application_id(),
            session.bucket_id(),
            session.object_key().to_owned(),
        )
    }

    fn expire_upload_session_locked(
        state: &mut MediaRepositoryState,
        upload_session_id: UploadSessionId,
        expired_at: OffsetDateTime,
    ) -> Result<UploadSessionExpiration, RepositoryError> {
        let current = state
            .upload_sessions
            .get(&upload_session_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        match current.state() {
            UploadSessionState::Completed => return Ok(UploadSessionExpiration::Completed),
            UploadSessionState::Cancelled => return Ok(UploadSessionExpiration::Cancelled),
            UploadSessionState::Expired => {
                return Ok(UploadSessionExpiration::AlreadyExpired(current));
            }
            UploadSessionState::Pending => {}
        }
        if !current.is_expired_at(expired_at) {
            return Ok(UploadSessionExpiration::NotDue);
        }

        let mut expired = current;
        expired
            .expire(expired_at)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        Self::release_session_reservation(state, &expired)?;
        state
            .upload_session_locations
            .remove(&Self::session_location(&expired));
        state
            .upload_sessions
            .insert(upload_session_id, expired.clone());
        Ok(UploadSessionExpiration::Expired(expired))
    }
}

#[async_trait]
impl MediaRepository for InMemoryMediaRepository {
    async fn find_by_object_key(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        object_key: &str,
    ) -> Result<Option<Media>, RepositoryError> {
        let state = self.lock()?;
        Ok(state
            .locations
            .get(&(application_id, bucket_id, object_key.to_owned()))
            .and_then(|media_id| state.media.get(media_id))
            .cloned())
    }

    async fn reserve_quota(
        &self,
        application_id: ApplicationId,
        bytes: u64,
    ) -> Result<(), RepositoryError> {
        let mut state = self.lock()?;
        let quota = state
            .quotas
            .get_mut(&application_id)
            .ok_or(RepositoryError::NotFound)?;
        if quota.available_bytes() < bytes {
            return Err(RepositoryError::QuotaExceeded);
        }
        quota.reserved_bytes = quota
            .reserved_bytes
            .checked_add(bytes)
            .ok_or_else(|| RepositoryError::Invariant("reserved quota overflow".into()))?;
        Ok(())
    }

    async fn create_uploading(&self, media: Media) -> Result<(), RepositoryError> {
        if media.state() != MediaState::Uploading {
            return Err(RepositoryError::Invariant(
                "only uploading media can be created by this operation".into(),
            ));
        }
        let location = (
            media.application_id(),
            media.bucket_id(),
            media.object_key().to_owned(),
        );
        let mut state = self.lock()?;
        if state.locations.contains_key(&location) || state.media.contains_key(&media.id()) {
            return Err(RepositoryError::Conflict);
        }
        state.locations.insert(location, media.id());
        state.media.insert(media.id(), media);
        Ok(())
    }

    async fn commit_upload(
        &self,
        media_id: MediaId,
        committed_at: OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<Media, RepositoryError> {
        let mut state = self.lock()?;
        if let Some(error) = state.fail_next_commit.take() {
            return Err(error);
        }
        let current = state
            .media
            .get(&media_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        if current.state() == MediaState::Active && state.outbox.contains_key(&event.id) {
            return Ok(current);
        }
        if current.state() != MediaState::Uploading {
            return Err(RepositoryError::Conflict);
        }
        if current.application_id() != event.application_id {
            return Err(RepositoryError::Invariant(
                "outbox event application differs from media application".into(),
            ));
        }

        let quota = state
            .quotas
            .get_mut(&current.application_id())
            .ok_or(RepositoryError::NotFound)?;
        if quota.reserved_bytes < current.size() {
            return Err(RepositoryError::Invariant(
                "media reservation is absent or smaller than media size".into(),
            ));
        }

        let mut committed = current;
        committed
            .transition_to(MediaState::Active, committed_at)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        quota.reserved_bytes -= committed.size();
        quota.used_bytes = quota
            .used_bytes
            .checked_add(committed.size())
            .ok_or_else(|| RepositoryError::Invariant("used quota overflow".into()))?;
        state.outbox.entry(event.id.clone()).or_insert(event);
        state.media.insert(media_id, committed.clone());
        Ok(committed)
    }

    async fn abort_upload(&self, media_id: MediaId) -> Result<(), RepositoryError> {
        let mut state = self.lock()?;
        let Some(media) = state.media.get(&media_id).cloned() else {
            return Ok(());
        };
        if media.state() != MediaState::Uploading {
            return Ok(());
        }
        state.media.remove(&media_id);
        state.locations.remove(&(
            media.application_id(),
            media.bucket_id(),
            media.object_key().to_owned(),
        ));
        let quota = state
            .quotas
            .get_mut(&media.application_id())
            .ok_or(RepositoryError::NotFound)?;
        quota.reserved_bytes = quota.reserved_bytes.saturating_sub(media.size());
        Ok(())
    }

    async fn release_quota(
        &self,
        application_id: ApplicationId,
        bytes: u64,
    ) -> Result<(), RepositoryError> {
        let mut state = self.lock()?;
        let quota = state
            .quotas
            .get_mut(&application_id)
            .ok_or(RepositoryError::NotFound)?;
        quota.reserved_bytes = quota.reserved_bytes.saturating_sub(bytes);
        Ok(())
    }

    async fn update_media(
        &self,
        media: Media,
        expected_revision: u64,
        event: OutboxEvent,
    ) -> Result<(), RepositoryError> {
        let mut state = self.lock()?;
        let stored = state
            .media
            .get(&media.id())
            .ok_or(RepositoryError::NotFound)?;
        if stored.application_id() != media.application_id()
            || stored.bucket_id() != media.bucket_id()
            || stored.object_key() != media.object_key()
        {
            return Err(RepositoryError::Invariant(
                "immutable media identity fields changed".into(),
            ));
        }
        if stored.revision() != expected_revision {
            return Err(RepositoryError::Conflict);
        }
        if event.application_id != media.application_id()
            || event.aggregate_id != media.id().to_string()
        {
            return Err(RepositoryError::Invariant(
                "media update event does not match media identity".into(),
            ));
        }
        state.outbox.entry(event.id.clone()).or_insert(event);
        state.media.insert(media.id(), media);
        Ok(())
    }

    async fn schedule_delete(
        &self,
        media_id: MediaId,
        deleted_at: OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<Media, RepositoryError> {
        let mut state = self.lock()?;
        let current = state
            .media
            .get(&media_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        if current.state() == MediaState::DeletePending && state.outbox.contains_key(&event.id) {
            return Ok(current);
        }
        if current.state() != MediaState::Active || event.application_id != current.application_id()
        {
            return Err(RepositoryError::Conflict);
        }
        let mut scheduled = current;
        scheduled
            .transition_to(MediaState::DeletePending, deleted_at)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        state.outbox.entry(event.id.clone()).or_insert(event);
        state.media.insert(media_id, scheduled.clone());
        Ok(scheduled)
    }
}

