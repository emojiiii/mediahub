// Async-job transitions, errors, and validation.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AsyncJobTransition {
    Applied,
    AlreadyApplied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncJobFailureDisposition {
    Terminal,
    RetryScheduled { retry_at: OffsetDateTime },
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum AsyncJobError {
    #[error("async job batch must contain at least one item")]
    EmptyBatch,
    #[error("async job max_attempts is invalid")]
    InvalidMaxAttempts,
    #[error("async job batch exceeds the maximum item count")]
    TooManyItems,
    #[error("async job max_attempts exceeds the maximum")]
    TooManyAttempts,
    #[error("async job ttl_seconds is invalid")]
    InvalidTtlSeconds,
    #[error("async job lease token is invalid")]
    InvalidLeaseToken,
    #[error("async job lease expiry is invalid")]
    InvalidLeaseExpiry,
    #[error("async job lease exceeds the maximum duration")]
    LeaseTooLong,
    #[error("async job attempts are exhausted")]
    AttemptsExhausted,
    #[error("async job is not claimable")]
    NotClaimable,
    #[error("async job item results are incomplete: expected {expected}, actual {actual}")]
    IncompleteItemResults { expected: u32, actual: u32 },
    #[error("async job retry time is invalid")]
    InvalidRetryTime,
    #[error("async job lease is stale")]
    StaleLease,
    #[error("async job lease has expired")]
    ExpiredLease,
    #[error("async job state transition from {from:?} to {to:?} is invalid")]
    InvalidStateTransition {
        from: AsyncJobState,
        to: AsyncJobState,
    },
    #[error("async job operation scope is invalid")]
    InvalidOperationScope,
    #[error("async job idempotency key is invalid")]
    InvalidIdempotencyKey,
    #[error("async job request hash is invalid")]
    InvalidRequestHash,
    #[error("async job request id is invalid")]
    InvalidRequestId,
    #[error("async job error summary is invalid")]
    InvalidErrorSummary,
    #[error("async job error code is invalid")]
    InvalidErrorCode,
    #[error("async job item attempt is invalid")]
    InvalidItemAttempt,
    #[error("async job item timestamps are invalid")]
    InvalidItemTimes,
    #[error("persisted async job is invalid")]
    InvalidPersistedJob,
}

pub type AsyncJobResult<T> = Result<T, AsyncJobError>;

fn validate_identity(scope: &str, key: &str, request_hash: &str) -> AsyncJobResult<()> {
    if scope.is_empty() || scope.len() > MAX_ASYNC_JOB_OPERATION_SCOPE_BYTES {
        return Err(AsyncJobError::InvalidOperationScope);
    }
    if key.is_empty() || key.len() > MAX_ASYNC_JOB_IDEMPOTENCY_KEY_BYTES {
        return Err(AsyncJobError::InvalidIdempotencyKey);
    }
    if request_hash.len() != 64 || !request_hash.bytes().all(|value| value.is_ascii_hexdigit()) {
        return Err(AsyncJobError::InvalidRequestHash);
    }
    Ok(())
}

fn validate_request_id(value: Option<&str>) -> AsyncJobResult<()> {
    if value.is_some_and(|value| {
        value.is_empty() || value.len() > MAX_ASYNC_JOB_REQUEST_ID_BYTES
    }) {
        Err(AsyncJobError::InvalidRequestId)
    } else {
        Ok(())
    }
}

fn validate_error_summary(value: &str) -> AsyncJobResult<()> {
    if value.is_empty() || value.len() > MAX_ASYNC_JOB_ERROR_BYTES {
        Err(AsyncJobError::InvalidErrorSummary)
    } else {
        Ok(())
    }
}

fn validate_error_code(value: &str) -> AsyncJobResult<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        Err(AsyncJobError::InvalidErrorCode)
    } else {
        Ok(())
    }
}

fn validate_item_attempt(value: u32) -> AsyncJobResult<()> {
    if value == 0 {
        Err(AsyncJobError::InvalidItemAttempt)
    } else {
        Ok(())
    }
}

fn validate_item_times(started_at: OffsetDateTime, completed_at: OffsetDateTime) -> AsyncJobResult<()> {
    if completed_at < started_at {
        Err(AsyncJobError::InvalidItemTimes)
    } else {
        Ok(())
    }
}

fn validate_persisted(value: &PersistedAsyncJob) -> AsyncJobResult<()> {
    if value.total_items == 0 || value.total_items > MAX_ASYNC_JOB_ITEMS {
        return Err(if value.total_items == 0 {
            AsyncJobError::EmptyBatch
        } else {
            AsyncJobError::TooManyItems
        });
    }
    if value.max_attempts == 0 {
        return Err(AsyncJobError::InvalidMaxAttempts);
    }
    if value.max_attempts > MAX_ASYNC_JOB_ATTEMPTS {
        return Err(AsyncJobError::TooManyAttempts);
    }
    if value.succeeded_items.saturating_add(value.failed_items) > value.total_items
        || value.attempt_count > value.max_attempts
        || value.updated_at < value.created_at
        || value.started_at.is_some_and(|at| at < value.created_at || at > value.updated_at)
        || value.completed_at.is_some_and(|at| at < value.created_at || at > value.updated_at)
        || value.failed_at.is_some_and(|at| at < value.created_at || at > value.updated_at)
        || value.cancelled_at.is_some_and(|at| at < value.created_at || at > value.updated_at)
    {
        return Err(AsyncJobError::InvalidPersistedJob);
    }
    validate_request_id(value.request_id.as_deref())?;
    if let Some(summary) = &value.error_summary {
        validate_error_summary(summary)?;
    }
    if let Some(lease_token) = &value.lease_token
        && (lease_token.is_empty() || lease_token.len() > MAX_ASYNC_JOB_IDEMPOTENCY_KEY_BYTES)
    {
        return Err(AsyncJobError::InvalidLeaseToken);
    }
    match value.state {
        AsyncJobState::Pending => {
            if value.lease_token.is_some() || value.leased_until.is_some()
                || value.next_attempt_at.is_none()
                || value.completed_at.is_some()
                || value.failed_at.is_some()
                || value.cancelled_at.is_some()
            {
                return Err(AsyncJobError::InvalidPersistedJob);
            }
        }
        AsyncJobState::Running => {
            let Some(leased_until) = value.leased_until else {
                return Err(AsyncJobError::InvalidPersistedJob);
            };
            if value.lease_token.is_none()
                || value.next_attempt_at.is_some()
                || value.started_at.is_none()
                || value.completed_at.is_some()
                || value.failed_at.is_some()
                || value.cancelled_at.is_some()
                || leased_until <= value.updated_at
                || leased_until - value.updated_at > time::Duration::seconds(MAX_ASYNC_JOB_LEASE_SECONDS)
            {
                return Err(AsyncJobError::InvalidPersistedJob);
            }
        }
        AsyncJobState::Completed => {
            if value.completed_at.is_none()
                || value.started_at.is_none()
                || value.failed_at.is_some()
                || value.cancelled_at.is_some()
                || value.lease_token.is_some()
                || value.leased_until.is_some()
                || value.next_attempt_at.is_some()
                || value.succeeded_items.saturating_add(value.failed_items) != value.total_items
                || value.error_summary.is_some()
            {
                return Err(AsyncJobError::InvalidPersistedJob);
            }
        }
        AsyncJobState::Failed => {
            if value.failed_at.is_none()
                || value.started_at.is_none()
                || value.completed_at.is_some()
                || value.cancelled_at.is_some()
                || value.lease_token.is_some()
                || value.leased_until.is_some()
                || value.next_attempt_at.is_some()
                || value.attempt_count != value.max_attempts
                || value.error_summary.is_none()
            {
                return Err(AsyncJobError::InvalidPersistedJob);
            }
        }
        AsyncJobState::Cancelled => {
            if value.cancelled_at.is_none()
                || value.completed_at.is_some()
                || value.failed_at.is_some()
                || value.lease_token.is_some()
                || value.leased_until.is_some()
                || value.next_attempt_at.is_some()
            {
                return Err(AsyncJobError::InvalidPersistedJob);
            }
        }
    }
    Ok(())
}
