use std::{collections::HashSet, fmt};

use async_trait::async_trait;
use mediahub_core::{
    ApplicationId, AsyncJob, AsyncJobAction, AsyncJobError, AsyncJobId, AsyncJobItemResult,
    MAX_ASYNC_JOB_ITEMS, MediaId, NewAsyncJob, OffsetDateTime,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::Duration;

use crate::Redacted;
use crate::{Clock, RepositoryError};

pub const DEFAULT_ASYNC_JOB_MAX_ATTEMPTS: u32 = 8;
pub const DEFAULT_ASYNC_JOB_LEASE_SECONDS: i64 = 30;
pub const MAX_ASYNC_JOB_CLAIM_LIMIT: usize = 100;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateAsyncJobRequest {
    pub application_id: ApplicationId,
    pub operation_scope: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub request_id: Option<String>,
    pub action: AsyncJobAction,
    pub media_ids: Vec<MediaId>,
    #[serde(default = "default_max_attempts")]
    pub max_attempts: u32,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteAsyncJobRequest {
    pub job_id: AsyncJobId,
    pub lease_token: String,
    pub item_results: Vec<AsyncJobItemResult>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailAsyncJobRequest {
    pub job_id: AsyncJobId,
    pub lease_token: String,
    pub error_summary: String,
    pub retry_at: Option<OffsetDateTime>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelAsyncJobRequest {
    pub application_id: ApplicationId,
    pub job_id: AsyncJobId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AsyncJobReceipt {
    pub job: AsyncJob,
    pub already_existed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct AsyncJobDetails {
    pub job: AsyncJob,
    pub item_results: Vec<AsyncJobItemResult>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct LeasedAsyncJob {
    pub job: AsyncJob,
    pub lease_token: String,
    pub pending_media_ids: Vec<MediaId>,
}

impl fmt::Debug for CreateAsyncJobRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CreateAsyncJobRequest")
            .field("application_id", &self.application_id)
            .field("operation_scope", &self.operation_scope)
            .field("idempotency_key", &Redacted(&self.idempotency_key))
            .field("request_hash", &Redacted(&self.request_hash))
            .field("request_id", &self.request_id)
            .field("action", &self.action)
            .field("media_ids", &self.media_ids)
            .field("max_attempts", &self.max_attempts)
            .finish()
    }
}

impl fmt::Debug for CompleteAsyncJobRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CompleteAsyncJobRequest")
            .field("job_id", &self.job_id)
            .field("lease_token", &Redacted(&self.lease_token))
            .field("item_results", &self.item_results)
            .finish()
    }
}

impl fmt::Debug for FailAsyncJobRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FailAsyncJobRequest")
            .field("job_id", &self.job_id)
            .field("lease_token", &Redacted(&self.lease_token))
            .field("error_summary", &self.error_summary)
            .field("retry_at", &self.retry_at)
            .finish()
    }
}

impl fmt::Debug for LeasedAsyncJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LeasedAsyncJob")
            .field("job", &self.job)
            .field("lease_token", &Redacted(&self.lease_token))
            .field("pending_media_ids", &self.pending_media_ids)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncJobCreation {
    Created(AsyncJob),
    Existing(AsyncJob),
    IdempotencyConflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncJobCompletion {
    Completed(AsyncJob),
    AlreadyCompleted(AsyncJob),
    LeaseLost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncJobFailure {
    RetryScheduled(AsyncJob),
    Terminal(AsyncJob),
    LeaseLost,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AsyncJobCancellation {
    Cancelled(AsyncJob),
    AlreadyCancelled(AsyncJob),
    Completed,
    Failed,
}

/// Persistence port for durable batch jobs. Every mutation that accepts a
/// lease token must fence its write against both the token and lease expiry.
#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait AsyncJobRepository: Send + Sync {
    /// Creates the job and all pending item rows atomically. A key replay with
    /// the same request hash returns `Existing`; a changed hash returns
    /// `IdempotencyConflict` without changing the original job.
    async fn create_async_job(
        &self,
        job: AsyncJob,
        media_ids: &[MediaId],
    ) -> Result<AsyncJobCreation, RepositoryError>;

    async fn find_async_job(
        &self,
        application_id: ApplicationId,
        job_id: AsyncJobId,
    ) -> Result<Option<AsyncJob>, RepositoryError>;

    async fn list_async_job_items(
        &self,
        application_id: ApplicationId,
        job_id: AsyncJobId,
    ) -> Result<Vec<AsyncJobItemResult>, RepositoryError>;

    /// Atomically leases ready pending jobs and expired running jobs. Claiming
    /// increments the attempt count and returns only unfinished media IDs.
    async fn claim_async_jobs(
        &self,
        now: OffsetDateTime,
        leased_until: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<LeasedAsyncJob>, RepositoryError>;

    /// Extends a still-valid lease while fencing by the current token.
    async fn renew_async_job_lease(
        &self,
        job_id: AsyncJobId,
        lease_token: &str,
        now: OffsetDateTime,
        leased_until: OffsetDateTime,
    ) -> Result<bool, RepositoryError>;

    /// Stores every item result and completes the job in one transaction. The
    /// repository must verify that results cover all unfinished targets and
    /// reject results from another job or application.
    async fn complete_async_job(
        &self,
        job_id: AsyncJobId,
        lease_token: &str,
        item_results: &[AsyncJobItemResult],
        completed_at: OffsetDateTime,
    ) -> Result<AsyncJobCompletion, RepositoryError>;

    /// Releases the current lease and either schedules the next attempt or
    /// enters terminal failure when max attempts have been exhausted.
    async fn fail_async_job(
        &self,
        job_id: AsyncJobId,
        lease_token: &str,
        error_summary: &str,
        retry_at: Option<OffsetDateTime>,
        failed_at: OffsetDateTime,
    ) -> Result<AsyncJobFailure, RepositoryError>;

    /// Cancels only a job owned by `application_id`; a repeated cancellation
    /// returns the existing cancelled job without rewriting item results.
    async fn cancel_async_job(
        &self,
        application_id: ApplicationId,
        job_id: AsyncJobId,
        cancelled_at: OffsetDateTime,
    ) -> Result<AsyncJobCancellation, RepositoryError>;
}

#[derive(Clone)]
pub struct AsyncJobService<R, C> {
    repository: R,
    clock: C,
}

impl<R, C> AsyncJobService<R, C>
where
    R: AsyncJobRepository,
    C: Clock,
{
    #[must_use]
    pub const fn new(repository: R, clock: C) -> Self {
        Self { repository, clock }
    }

    pub async fn create(
        &self,
        request: &CreateAsyncJobRequest,
    ) -> Result<AsyncJobReceipt, AsyncJobApplicationError> {
        validate_unique_media_ids(&request.media_ids)?;
        let total_items = u32::try_from(request.media_ids.len())
            .map_err(|_| AsyncJobApplicationError::TooManyItems)?;
        let job = AsyncJob::new(
            NewAsyncJob {
                id: AsyncJobId::new(),
                application_id: request.application_id,
                operation_scope: request.operation_scope.clone(),
                idempotency_key: request.idempotency_key.clone(),
                request_hash: request.request_hash.clone(),
                request_id: request.request_id.clone(),
                action: request.action.clone(),
                total_items,
                max_attempts: request.max_attempts,
            },
            self.clock.now(),
        )?;

        match self
            .repository
            .create_async_job(job, &request.media_ids)
            .await?
        {
            AsyncJobCreation::Created(job) => Ok(AsyncJobReceipt {
                job,
                already_existed: false,
            }),
            AsyncJobCreation::Existing(job) => Ok(AsyncJobReceipt {
                job,
                already_existed: true,
            }),
            AsyncJobCreation::IdempotencyConflict => {
                Err(AsyncJobApplicationError::IdempotencyConflict)
            }
        }
    }

    pub async fn get(
        &self,
        application_id: ApplicationId,
        job_id: AsyncJobId,
    ) -> Result<AsyncJobDetails, AsyncJobApplicationError> {
        let job = self
            .repository
            .find_async_job(application_id, job_id)
            .await?
            .ok_or(AsyncJobApplicationError::NotFound)?;
        let item_results = self
            .repository
            .list_async_job_items(application_id, job_id)
            .await?;
        Ok(AsyncJobDetails { job, item_results })
    }

    pub async fn claim(
        &self,
        limit: usize,
    ) -> Result<Vec<LeasedAsyncJob>, AsyncJobApplicationError> {
        if limit == 0 || limit > MAX_ASYNC_JOB_CLAIM_LIMIT {
            return Err(AsyncJobApplicationError::InvalidClaimLimit);
        }
        let now = self.clock.now();
        self.repository
            .claim_async_jobs(
                now,
                now + Duration::seconds(DEFAULT_ASYNC_JOB_LEASE_SECONDS),
                limit,
            )
            .await
            .map_err(Into::into)
    }

    pub async fn complete(
        &self,
        request: &CompleteAsyncJobRequest,
    ) -> Result<AsyncJob, AsyncJobApplicationError> {
        match self
            .repository
            .complete_async_job(
                request.job_id,
                &request.lease_token,
                &request.item_results,
                self.clock.now(),
            )
            .await?
        {
            AsyncJobCompletion::Completed(job) | AsyncJobCompletion::AlreadyCompleted(job) => {
                Ok(job)
            }
            AsyncJobCompletion::LeaseLost => Err(AsyncJobApplicationError::LeaseLost),
        }
    }

    pub async fn renew(
        &self,
        job_id: AsyncJobId,
        lease_token: &str,
    ) -> Result<bool, AsyncJobApplicationError> {
        let now = self.clock.now();
        let leased_until = now + Duration::seconds(DEFAULT_ASYNC_JOB_LEASE_SECONDS);
        Ok(self
            .repository
            .renew_async_job_lease(job_id, lease_token, now, leased_until)
            .await?)
    }

    pub async fn fail(
        &self,
        request: &FailAsyncJobRequest,
    ) -> Result<AsyncJobFailure, AsyncJobApplicationError> {
        let result = self
            .repository
            .fail_async_job(
                request.job_id,
                &request.lease_token,
                &request.error_summary,
                request.retry_at,
                self.clock.now(),
            )
            .await?;
        if result == AsyncJobFailure::LeaseLost {
            return Err(AsyncJobApplicationError::LeaseLost);
        }
        Ok(result)
    }

    pub async fn cancel(
        &self,
        request: &CancelAsyncJobRequest,
    ) -> Result<AsyncJob, AsyncJobApplicationError> {
        match self
            .repository
            .cancel_async_job(request.application_id, request.job_id, self.clock.now())
            .await?
        {
            AsyncJobCancellation::Cancelled(job) | AsyncJobCancellation::AlreadyCancelled(job) => {
                Ok(job)
            }
            AsyncJobCancellation::Completed => Err(AsyncJobApplicationError::AlreadyCompleted),
            AsyncJobCancellation::Failed => Err(AsyncJobApplicationError::AlreadyFailed),
        }
    }
}

#[derive(Debug, Error)]
pub enum AsyncJobApplicationError {
    #[error("async job was not found")]
    NotFound,
    #[error("batch contains duplicate media IDs")]
    DuplicateMediaIds,
    #[error("batch contains too many media IDs")]
    TooManyItems,
    #[error("idempotency key was already used with a different request")]
    IdempotencyConflict,
    #[error("claim limit must be between 1 and {MAX_ASYNC_JOB_CLAIM_LIMIT}")]
    InvalidClaimLimit,
    #[error("async job lease was lost")]
    LeaseLost,
    #[error("completed async job cannot be cancelled")]
    AlreadyCompleted,
    #[error("failed async job cannot be cancelled")]
    AlreadyFailed,
    #[error(transparent)]
    Domain(#[from] AsyncJobError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
}

const fn default_max_attempts() -> u32 {
    DEFAULT_ASYNC_JOB_MAX_ATTEMPTS
}

fn validate_unique_media_ids(media_ids: &[MediaId]) -> Result<(), AsyncJobApplicationError> {
    if media_ids.is_empty() {
        return Err(AsyncJobError::EmptyBatch.into());
    }
    if media_ids.len() > MAX_ASYNC_JOB_ITEMS as usize {
        return Err(AsyncJobApplicationError::TooManyItems);
    }
    let unique = media_ids.iter().copied().collect::<HashSet<_>>();
    if unique.len() != media_ids.len() {
        return Err(AsyncJobApplicationError::DuplicateMediaIds);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures::executor::block_on;
    use mediahub_core::{AsyncJobState, Visibility};

    use crate::FixedClock;

    use super::*;

    #[derive(Clone, Default)]
    struct StubRepository {
        created: Arc<Mutex<Option<AsyncJob>>>,
    }

    #[async_trait]
    impl AsyncJobRepository for StubRepository {
        async fn create_async_job(
            &self,
            job: AsyncJob,
            _media_ids: &[MediaId],
        ) -> Result<AsyncJobCreation, RepositoryError> {
            *self.created.lock().expect("lock") = Some(job.clone());
            Ok(AsyncJobCreation::Created(job))
        }

        async fn find_async_job(
            &self,
            _application_id: ApplicationId,
            _job_id: AsyncJobId,
        ) -> Result<Option<AsyncJob>, RepositoryError> {
            Ok(self.created.lock().expect("lock").clone())
        }

        async fn list_async_job_items(
            &self,
            _application_id: ApplicationId,
            _job_id: AsyncJobId,
        ) -> Result<Vec<AsyncJobItemResult>, RepositoryError> {
            Ok(Vec::new())
        }

        async fn claim_async_jobs(
            &self,
            _now: OffsetDateTime,
            _leased_until: OffsetDateTime,
            _limit: usize,
        ) -> Result<Vec<LeasedAsyncJob>, RepositoryError> {
            Ok(Vec::new())
        }

        async fn renew_async_job_lease(
            &self,
            _job_id: AsyncJobId,
            _lease_token: &str,
            _now: OffsetDateTime,
            _leased_until: OffsetDateTime,
        ) -> Result<bool, RepositoryError> {
            Ok(false)
        }

        async fn complete_async_job(
            &self,
            _job_id: AsyncJobId,
            _lease_token: &str,
            _item_results: &[AsyncJobItemResult],
            _completed_at: OffsetDateTime,
        ) -> Result<AsyncJobCompletion, RepositoryError> {
            Err(RepositoryError::Unavailable("not used".to_owned()))
        }

        async fn fail_async_job(
            &self,
            _job_id: AsyncJobId,
            _lease_token: &str,
            _error_summary: &str,
            _retry_at: Option<OffsetDateTime>,
            _failed_at: OffsetDateTime,
        ) -> Result<AsyncJobFailure, RepositoryError> {
            Err(RepositoryError::Unavailable("not used".to_owned()))
        }

        async fn cancel_async_job(
            &self,
            _application_id: ApplicationId,
            _job_id: AsyncJobId,
            _cancelled_at: OffsetDateTime,
        ) -> Result<AsyncJobCancellation, RepositoryError> {
            Err(RepositoryError::Unavailable("not used".to_owned()))
        }
    }

    fn request(media_ids: Vec<MediaId>) -> CreateAsyncJobRequest {
        CreateAsyncJobRequest {
            application_id: ApplicationId::new(),
            operation_scope: "media.batch".to_owned(),
            idempotency_key: "test-key".to_owned(),
            request_hash: "b".repeat(64),
            request_id: None,
            action: AsyncJobAction::UpdateVisibility {
                visibility: Visibility::Private,
            },
            media_ids,
            max_attempts: DEFAULT_ASYNC_JOB_MAX_ATTEMPTS,
        }
    }

    #[test]
    fn create_persists_a_pending_job() {
        let repository = StubRepository::default();
        let service = AsyncJobService::new(repository, FixedClock::new(OffsetDateTime::UNIX_EPOCH));
        let receipt = block_on(service.create(&request(vec![MediaId::new(), MediaId::new()])))
            .expect("create job");
        assert_eq!(receipt.job.state(), AsyncJobState::Pending);
        assert_eq!(receipt.job.total_items(), 2);
        assert!(!receipt.already_existed);
    }

    #[test]
    fn duplicate_targets_are_rejected_before_persistence() {
        let media_id = MediaId::new();
        let service = AsyncJobService::new(
            StubRepository::default(),
            FixedClock::new(OffsetDateTime::UNIX_EPOCH),
        );
        let result = block_on(service.create(&request(vec![media_id, media_id])));
        assert!(matches!(
            result,
            Err(AsyncJobApplicationError::DuplicateMediaIds)
        ));
    }

    #[test]
    fn invalid_claim_limits_are_rejected() {
        let service = AsyncJobService::new(
            StubRepository::default(),
            FixedClock::new(OffsetDateTime::UNIX_EPOCH),
        );
        assert!(matches!(
            block_on(service.claim(0)),
            Err(AsyncJobApplicationError::InvalidClaimLimit)
        ));
    }
}
