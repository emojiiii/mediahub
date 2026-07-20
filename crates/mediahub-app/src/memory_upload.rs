// In-memory upload-session repository.

#[async_trait]
impl UploadSessionRepository for InMemoryMediaRepository {
    async fn create_upload_session(&self, session: UploadSession) -> Result<(), RepositoryError> {
        if session.state() != UploadSessionState::Pending {
            return Err(RepositoryError::Invariant(
                "only pending upload sessions can be created".into(),
            ));
        }
        let location = Self::session_location(&session);
        let mut state = self.lock()?;
        if state.upload_sessions.contains_key(&session.id())
            || state.media.contains_key(&session.media_id())
            || state.locations.contains_key(&location)
            || state.upload_session_locations.contains_key(&location)
        {
            return Err(RepositoryError::Conflict);
        }
        let quota = state
            .quotas
            .get_mut(&session.application_id())
            .ok_or(RepositoryError::NotFound)?;
        if quota.available_bytes() < session.reserved_bytes() {
            return Err(RepositoryError::QuotaExceeded);
        }
        quota.reserved_bytes = quota
            .reserved_bytes
            .checked_add(session.reserved_bytes())
            .ok_or_else(|| RepositoryError::Invariant("reserved quota overflow".into()))?;
        state
            .upload_session_locations
            .insert(location, session.id());
        state.upload_sessions.insert(session.id(), session);
        Ok(())
    }

    async fn find_upload_session(
        &self,
        upload_session_id: UploadSessionId,
    ) -> Result<Option<UploadSession>, RepositoryError> {
        Ok(self
            .lock()?
            .upload_sessions
            .get(&upload_session_id)
            .cloned())
    }

    async fn complete_upload_session(
        &self,
        upload_session_id: UploadSessionId,
        media: Media,
        completed_at: OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<UploadSessionCompletion, RepositoryError> {
        let mut state = self.lock()?;
        let current = state
            .upload_sessions
            .get(&upload_session_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        match current.state() {
            UploadSessionState::Completed => {
                let media_id = state
                    .completed_upload_media
                    .get(&upload_session_id)
                    .ok_or_else(|| {
                        RepositoryError::Invariant("completed upload has no media mapping".into())
                    })?;
                let media = state.media.get(media_id).cloned().ok_or_else(|| {
                    RepositoryError::Invariant("completed upload media is unavailable".into())
                })?;
                return Ok(UploadSessionCompletion::AlreadyCompleted(media));
            }
            UploadSessionState::Cancelled => return Ok(UploadSessionCompletion::Cancelled),
            UploadSessionState::Expired => return Ok(UploadSessionCompletion::Expired),
            UploadSessionState::Pending => {}
        }
        if current.is_expired_at(completed_at) {
            let _ =
                Self::expire_upload_session_locked(&mut state, upload_session_id, completed_at)?;
            return Ok(UploadSessionCompletion::Expired);
        }
        if media.id() != current.media_id()
            || media.application_id() != current.application_id()
            || media.bucket_id() != current.bucket_id()
            || media.object_key() != current.object_key()
            || media.size() != current.expected_size()
            || media.state() != MediaState::Uploading
            || event.application_id != current.application_id()
        {
            return Err(RepositoryError::Invariant(
                "completed media does not match upload session contract".into(),
            ));
        }
        if state.media.contains_key(&media.id())
            || state
                .locations
                .contains_key(&Self::session_location(&current))
        {
            return Err(RepositoryError::Conflict);
        }
        let quota = state
            .quotas
            .get(&current.application_id())
            .copied()
            .ok_or(RepositoryError::NotFound)?;
        if quota.reserved_bytes < current.reserved_bytes() {
            return Err(RepositoryError::Invariant(
                "upload session reservation is absent or smaller than expected size".into(),
            ));
        }
        let used_bytes = quota
            .used_bytes
            .checked_add(media.size())
            .ok_or_else(|| RepositoryError::Invariant("used quota overflow".into()))?;

        let mut completed_session = current;
        completed_session
            .complete(completed_at)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        let mut committed_media = media;
        committed_media
            .transition_to(MediaState::Active, completed_at)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;

        let quota = state
            .quotas
            .get_mut(&completed_session.application_id())
            .ok_or(RepositoryError::NotFound)?;
        quota.reserved_bytes -= completed_session.reserved_bytes();
        quota.used_bytes = used_bytes;
        state
            .upload_session_locations
            .remove(&Self::session_location(&completed_session));
        state.locations.insert(
            Self::session_location(&completed_session),
            committed_media.id(),
        );
        state.outbox.entry(event.id.clone()).or_insert(event);
        state
            .completed_upload_media
            .insert(upload_session_id, committed_media.id());
        state
            .upload_sessions
            .insert(upload_session_id, completed_session);
        state
            .media
            .insert(committed_media.id(), committed_media.clone());
        Ok(UploadSessionCompletion::Completed(committed_media))
    }

    async fn completed_upload_media(
        &self,
        upload_session_id: UploadSessionId,
    ) -> Result<Option<Media>, RepositoryError> {
        let state = self.lock()?;
        Ok(state
            .completed_upload_media
            .get(&upload_session_id)
            .and_then(|media_id| state.media.get(media_id))
            .cloned())
    }

    async fn cancel_upload_session(
        &self,
        upload_session_id: UploadSessionId,
        cancelled_at: OffsetDateTime,
    ) -> Result<UploadSessionCancellation, RepositoryError> {
        let mut state = self.lock()?;
        let current = state
            .upload_sessions
            .get(&upload_session_id)
            .cloned()
            .ok_or(RepositoryError::NotFound)?;
        match current.state() {
            UploadSessionState::Completed => return Ok(UploadSessionCancellation::Completed),
            UploadSessionState::Expired => return Ok(UploadSessionCancellation::Expired),
            UploadSessionState::Cancelled => {
                return Ok(UploadSessionCancellation::AlreadyCancelled(current));
            }
            UploadSessionState::Pending => {}
        }
        let mut cancelled = current;
        cancelled
            .cancel(cancelled_at)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        Self::release_session_reservation(&mut state, &cancelled)?;
        state
            .upload_session_locations
            .remove(&Self::session_location(&cancelled));
        state
            .upload_sessions
            .insert(upload_session_id, cancelled.clone());
        Ok(UploadSessionCancellation::Cancelled(cancelled))
    }

    async fn expire_upload_session(
        &self,
        upload_session_id: UploadSessionId,
        expired_at: OffsetDateTime,
    ) -> Result<UploadSessionExpiration, RepositoryError> {
        let mut state = self.lock()?;
        Self::expire_upload_session_locked(&mut state, upload_session_id, expired_at)
    }

    async fn expire_upload_sessions(
        &self,
        expired_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<UploadSession>, RepositoryError> {
        let mut state = self.lock()?;
        let due_ids = state
            .upload_sessions
            .iter()
            .filter_map(|(id, session)| {
                let due = session.is_expired_at(expired_at)
                    || (expired_at >= session.session_expires_at()
                        && matches!(
                            session.state(),
                            UploadSessionState::Completed
                                | UploadSessionState::Expired
                                | UploadSessionState::Cancelled
                        )
                        && !state.upload_cleanup_completed.contains(id));
                due.then_some(*id)
            })
            .take(limit)
            .collect::<Vec<_>>();
        let mut expired_sessions = Vec::with_capacity(due_ids.len());
        for upload_session_id in due_ids {
            match Self::expire_upload_session_locked(&mut state, upload_session_id, expired_at)? {
                UploadSessionExpiration::Expired(session)
                | UploadSessionExpiration::AlreadyExpired(session) => {
                    expired_sessions.push(session)
                }
                UploadSessionExpiration::Cancelled => {
                    if let Some(session) = state.upload_sessions.get(&upload_session_id).cloned() {
                        expired_sessions.push(session);
                    }
                }
                UploadSessionExpiration::Completed => {
                    if let Some(session) = state.upload_sessions.get(&upload_session_id).cloned() {
                        expired_sessions.push(session);
                    }
                }
                UploadSessionExpiration::NotDue => {}
            }
        }
        Ok(expired_sessions)
    }

    async fn complete_upload_session_cleanup(
        &self,
        upload_session_id: UploadSessionId,
    ) -> Result<bool, RepositoryError> {
        let mut state = self.lock()?;
        let session = state
            .upload_sessions
            .get(&upload_session_id)
            .ok_or(RepositoryError::NotFound)?;
        if !matches!(
            session.state(),
            UploadSessionState::Completed
                | UploadSessionState::Expired
                | UploadSessionState::Cancelled
        ) {
            return Ok(false);
        }
        Ok(state.upload_cleanup_completed.insert(upload_session_id))
    }
}

