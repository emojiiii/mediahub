// Async-job aggregate and persistence model.

#[derive(Clone, Debug)]
pub struct NewAsyncJob {
    pub id: AsyncJobId,
    pub application_id: ApplicationId,
    pub operation_scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub request_id: Option<String>,
    pub action: AsyncJobAction,
    pub total_items: u32,
    pub max_attempts: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedAsyncJob {
    pub id: AsyncJobId,
    pub application_id: ApplicationId,
    pub operation_scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub request_id: Option<String>,
    pub action: AsyncJobAction,
    pub state: AsyncJobState,
    pub total_items: u32,
    pub succeeded_items: u32,
    pub failed_items: u32,
    pub attempt_count: u32,
    pub max_attempts: u32,
    pub next_attempt_at: Option<OffsetDateTime>,
    pub lease_token: Option<String>,
    pub leased_until: Option<OffsetDateTime>,
    pub error_summary: Option<String>,
    pub started_at: Option<OffsetDateTime>,
    pub completed_at: Option<OffsetDateTime>,
    pub failed_at: Option<OffsetDateTime>,
    pub cancelled_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AsyncJob {
    id: AsyncJobId,
    application_id: ApplicationId,
    operation_scope: String,
    idempotency_key: String,
    request_hash: String,
    request_id: Option<String>,
    action: AsyncJobAction,
    state: AsyncJobState,
    total_items: u32,
    succeeded_items: u32,
    failed_items: u32,
    attempt_count: u32,
    max_attempts: u32,
    next_attempt_at: Option<OffsetDateTime>,
    lease_token: Option<String>,
    leased_until: Option<OffsetDateTime>,
    error_summary: Option<String>,
    started_at: Option<OffsetDateTime>,
    completed_at: Option<OffsetDateTime>,
    failed_at: Option<OffsetDateTime>,
    cancelled_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl AsyncJob {
    pub fn new(input: NewAsyncJob, now: OffsetDateTime) -> AsyncJobResult<Self> {
        validate_identity(
            &input.operation_scope,
            &input.idempotency_key,
            &input.request_hash,
        )?;
        input.action.validate()?;
        if input.total_items == 0 {
            return Err(AsyncJobError::EmptyBatch);
        }
        if input.max_attempts == 0 {
            return Err(AsyncJobError::InvalidMaxAttempts);
        }

        Ok(Self {
            id: input.id,
            application_id: input.application_id,
            operation_scope: input.operation_scope,
            idempotency_key: input.idempotency_key,
            request_hash: input.request_hash,
            request_id: input.request_id,
            action: input.action,
            state: AsyncJobState::Pending,
            total_items: input.total_items,
            succeeded_items: 0,
            failed_items: 0,
            attempt_count: 0,
            max_attempts: input.max_attempts,
            next_attempt_at: Some(now),
            lease_token: None,
            leased_until: None,
            error_summary: None,
            started_at: None,
            completed_at: None,
            failed_at: None,
            cancelled_at: None,
            created_at: now,
            updated_at: now,
        })
    }

    pub fn from_persistence(value: PersistedAsyncJob) -> AsyncJobResult<Self> {
        validate_identity(
            &value.operation_scope,
            &value.idempotency_key,
            &value.request_hash,
        )?;
        value.action.validate()?;
        validate_persisted(&value)?;
        Ok(Self {
            id: value.id,
            application_id: value.application_id,
            operation_scope: value.operation_scope,
            idempotency_key: value.idempotency_key,
            request_hash: value.request_hash,
            request_id: value.request_id,
            action: value.action,
            state: value.state,
            total_items: value.total_items,
            succeeded_items: value.succeeded_items,
            failed_items: value.failed_items,
            attempt_count: value.attempt_count,
            max_attempts: value.max_attempts,
            next_attempt_at: value.next_attempt_at,
            lease_token: value.lease_token,
            leased_until: value.leased_until,
            error_summary: value.error_summary,
            started_at: value.started_at,
            completed_at: value.completed_at,
            failed_at: value.failed_at,
            cancelled_at: value.cancelled_at,
            created_at: value.created_at,
            updated_at: value.updated_at,
        })
    }

    #[must_use]
    pub fn to_persisted(&self) -> PersistedAsyncJob {
        PersistedAsyncJob {
            id: self.id,
            application_id: self.application_id,
            operation_scope: self.operation_scope.clone(),
            idempotency_key: self.idempotency_key.clone(),
            request_hash: self.request_hash.clone(),
            request_id: self.request_id.clone(),
            action: self.action.clone(),
            state: self.state,
            total_items: self.total_items,
            succeeded_items: self.succeeded_items,
            failed_items: self.failed_items,
            attempt_count: self.attempt_count,
            max_attempts: self.max_attempts,
            next_attempt_at: self.next_attempt_at,
            lease_token: self.lease_token.clone(),
            leased_until: self.leased_until,
            error_summary: self.error_summary.clone(),
            started_at: self.started_at,
            completed_at: self.completed_at,
            failed_at: self.failed_at,
            cancelled_at: self.cancelled_at,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }

    #[must_use]
    pub const fn id(&self) -> AsyncJobId {
        self.id
    }

    #[must_use]
    pub const fn application_id(&self) -> ApplicationId {
        self.application_id
    }

    #[must_use]
    pub fn operation_scope(&self) -> &str {
        &self.operation_scope
    }

    #[must_use]
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    #[must_use]
    pub fn request_hash(&self) -> &str {
        &self.request_hash
    }

    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
    }

    #[must_use]
    pub const fn action(&self) -> &AsyncJobAction {
        &self.action
    }

    #[must_use]
    pub const fn state(&self) -> AsyncJobState {
        self.state
    }

    #[must_use]
    pub const fn total_items(&self) -> u32 {
        self.total_items
    }

    #[must_use]
    pub const fn succeeded_items(&self) -> u32 {
        self.succeeded_items
    }

    #[must_use]
    pub const fn failed_items(&self) -> u32 {
        self.failed_items
    }

    #[must_use]
    pub const fn attempt_count(&self) -> u32 {
        self.attempt_count
    }

    #[must_use]
    pub const fn max_attempts(&self) -> u32 {
        self.max_attempts
    }

    #[must_use]
    pub const fn next_attempt_at(&self) -> Option<OffsetDateTime> {
        self.next_attempt_at
    }

    #[must_use]
    pub fn lease_token(&self) -> Option<&str> {
        self.lease_token.as_deref()
    }

    #[must_use]
    pub const fn leased_until(&self) -> Option<OffsetDateTime> {
        self.leased_until
    }

    #[must_use]
    pub fn error_summary(&self) -> Option<&str> {
        self.error_summary.as_deref()
    }

    #[must_use]
    pub const fn created_at(&self) -> OffsetDateTime {
        self.created_at
    }

    #[must_use]
    pub const fn updated_at(&self) -> OffsetDateTime {
        self.updated_at
    }

    #[must_use]
    pub fn is_claimable_at(&self, now: OffsetDateTime) -> bool {
        if self.attempt_count >= self.max_attempts {
            return false;
        }
        match self.state {
            AsyncJobState::Pending => self.next_attempt_at.is_some_and(|due| due <= now),
            AsyncJobState::Running => self.leased_until.is_some_and(|until| until <= now),
            AsyncJobState::Completed | AsyncJobState::Failed | AsyncJobState::Cancelled => false,
        }
    }

    pub fn claim(
        &mut self,
        lease_token: impl Into<String>,
        leased_until: OffsetDateTime,
        now: OffsetDateTime,
    ) -> AsyncJobResult<()> {
        let lease_token = lease_token.into();
        if lease_token.is_empty() || lease_token.len() > 255 {
            return Err(AsyncJobError::InvalidLeaseToken);
        }
        if leased_until <= now {
            return Err(AsyncJobError::InvalidLeaseExpiry);
        }
        if !self.is_claimable_at(now) {
            return Err(if self.attempt_count >= self.max_attempts {
                AsyncJobError::AttemptsExhausted
            } else {
                AsyncJobError::NotClaimable
            });
        }

        self.state = AsyncJobState::Running;
        self.attempt_count += 1;
        self.next_attempt_at = None;
        self.lease_token = Some(lease_token);
        self.leased_until = Some(leased_until);
        self.started_at.get_or_insert(now);
        self.updated_at = now;
        Ok(())
    }

    pub fn complete(
        &mut self,
        lease_token: &str,
        succeeded_items: u32,
        failed_items: u32,
        now: OffsetDateTime,
    ) -> AsyncJobResult<AsyncJobTransition> {
        if self.state == AsyncJobState::Completed {
            return Ok(AsyncJobTransition::AlreadyApplied);
        }
        self.verify_lease(lease_token, now)?;
        if succeeded_items.saturating_add(failed_items) != self.total_items {
            return Err(AsyncJobError::IncompleteItemResults {
                expected: self.total_items,
                actual: succeeded_items.saturating_add(failed_items),
            });
        }

        self.state = AsyncJobState::Completed;
        self.succeeded_items = succeeded_items;
        self.failed_items = failed_items;
        self.completed_at = Some(now);
        self.next_attempt_at = None;
        self.lease_token = None;
        self.leased_until = None;
        self.error_summary = None;
        self.updated_at = now;
        Ok(AsyncJobTransition::Applied)
    }

    pub fn fail(
        &mut self,
        lease_token: &str,
        error_summary: impl Into<String>,
        retry_at: Option<OffsetDateTime>,
        now: OffsetDateTime,
    ) -> AsyncJobResult<AsyncJobFailureDisposition> {
        self.verify_lease(lease_token, now)?;
        let error_summary = error_summary.into();
        validate_error_summary(&error_summary)?;
        let exhausted = self.attempt_count >= self.max_attempts;
        if !exhausted && retry_at.is_none_or(|retry_at| retry_at <= now) {
            return Err(AsyncJobError::InvalidRetryTime);
        }

        self.lease_token = None;
        self.leased_until = None;
        self.error_summary = Some(error_summary);
        self.updated_at = now;
        if exhausted {
            self.state = AsyncJobState::Failed;
            self.next_attempt_at = None;
            self.failed_at = Some(now);
            Ok(AsyncJobFailureDisposition::Terminal)
        } else {
            self.state = AsyncJobState::Pending;
            self.next_attempt_at = retry_at;
            Ok(AsyncJobFailureDisposition::RetryScheduled {
                retry_at: retry_at.expect("validated retry timestamp"),
            })
        }
    }

    pub fn cancel(&mut self, now: OffsetDateTime) -> AsyncJobResult<AsyncJobTransition> {
        if self.state == AsyncJobState::Cancelled {
            return Ok(AsyncJobTransition::AlreadyApplied);
        }
        if self.state.is_terminal() {
            return Err(AsyncJobError::InvalidStateTransition {
                from: self.state,
                to: AsyncJobState::Cancelled,
            });
        }

        self.state = AsyncJobState::Cancelled;
        self.next_attempt_at = None;
        self.lease_token = None;
        self.leased_until = None;
        self.cancelled_at = Some(now);
        self.updated_at = now;
        Ok(AsyncJobTransition::Applied)
    }

    fn verify_lease(&self, lease_token: &str, now: OffsetDateTime) -> AsyncJobResult<()> {
        if self.state != AsyncJobState::Running {
            return Err(AsyncJobError::InvalidStateTransition {
                from: self.state,
                to: AsyncJobState::Completed,
            });
        }
        if self.lease_token.as_deref() != Some(lease_token) {
            return Err(AsyncJobError::StaleLease);
        }
        if self.leased_until.is_none_or(|until| until <= now) {
            return Err(AsyncJobError::ExpiredLease);
        }
        Ok(())
    }
}

