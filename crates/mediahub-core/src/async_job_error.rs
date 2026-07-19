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
    #[error("async job ttl_seconds is invalid")]
    InvalidTtlSeconds,
    #[error("async job lease token is invalid")]
    InvalidLeaseToken,
    #[error("async job lease expiry is invalid")]
    InvalidLeaseExpiry,
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
    if value.total_items == 0
        || value.max_attempts == 0
        || value.succeeded_items.saturating_add(value.failed_items) > value.total_items
        || value.attempt_count > value.max_attempts
    {
        return Err(AsyncJobError::InvalidPersistedJob);
    }
    if let Some(summary) = &value.error_summary {
        validate_error_summary(summary)?;
    }
    if let Some(lease_token) = &value.lease_token {
        if lease_token.is_empty() || lease_token.len() > MAX_ASYNC_JOB_IDEMPOTENCY_KEY_BYTES {
            return Err(AsyncJobError::InvalidLeaseToken);
        }
    }
    Ok(())
}