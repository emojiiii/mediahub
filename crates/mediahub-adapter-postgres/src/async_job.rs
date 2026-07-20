use std::collections::HashSet;

use async_trait::async_trait;
use mediahub_app::{
    AsyncJobCancellation, AsyncJobCompletion, AsyncJobCreation, AsyncJobFailure,
    AsyncJobRepository, LeasedAsyncJob, RepositoryError,
};
use mediahub_core::{
    ApplicationId, AsyncJob, AsyncJobAction, AsyncJobFailureDisposition, AsyncJobId,
    AsyncJobItemResult, AsyncJobItemState, AsyncJobState, MediaId, OffsetDateTime,
    PersistedAsyncJob,
};
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction, postgres::PgRow, types::Json};
use uuid::Uuid;

use crate::{
    PostgresRepository,
    codec::{as_i64, as_u32, database_error, postgres_time},
};

#[async_trait]
impl AsyncJobRepository for PostgresRepository {
    async fn create_async_job(
        &self,
        job: AsyncJob,
        media_ids: &[MediaId],
    ) -> Result<AsyncJobCreation, RepositoryError> {
        if media_ids.is_empty() || usize::try_from(job.total_items()).ok() != Some(media_ids.len())
        {
            return Err(RepositoryError::Invariant(
                "async job item count does not match media IDs".into(),
            ));
        }
        let unique = media_ids.iter().copied().collect::<HashSet<_>>();
        if unique.len() != media_ids.len() {
            return Err(RepositoryError::Invariant(
                "async job media IDs contain duplicates".into(),
            ));
        }
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let media_uuids = media_ids.iter().map(|id| id.as_uuid()).collect::<Vec<_>>();
        let owned_count = sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM media WHERE application_id = $1 AND id = ANY($2)",
        )
        .bind(job.application_id().as_uuid())
        .bind(&media_uuids)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if usize::try_from(owned_count).ok() != Some(media_ids.len()) {
            return Err(RepositoryError::NotFound);
        }

        let inserted = insert_job(&mut transaction, &job).await?;
        if !inserted {
            let existing = find_by_idempotency_key(
                &mut transaction,
                job.application_id(),
                job.operation_scope(),
                job.idempotency_key(),
            )
            .await?
            .ok_or_else(|| {
                RepositoryError::Invariant("idempotency conflict has no existing job".into())
            })?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(if existing.request_hash() == job.request_hash() {
                AsyncJobCreation::Existing(existing)
            } else {
                AsyncJobCreation::IdempotencyConflict
            });
        }

        for (ordinal, media_id) in media_ids.iter().enumerate() {
            sqlx::query(
                "INSERT INTO async_job_item_results (job_id, application_id, media_id, ordinal, \
                 state, attempt_count, created_at, updated_at) \
                 VALUES ($1, $2, $3, $4, 'pending', 0, $5, $5)",
            )
            .bind(job.id().as_uuid())
            .bind(job.application_id().as_uuid())
            .bind(media_id.as_uuid())
            .bind(i32::try_from(ordinal).map_err(|_| {
                RepositoryError::Invariant("async job ordinal exceeds PostgreSQL INTEGER".into())
            })?)
            .bind(job.created_at())
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        }
        let stored = locked_job(&mut transaction, job.id()).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(AsyncJobCreation::Created(stored))
    }

    async fn find_async_job(
        &self,
        application_id: ApplicationId,
        job_id: AsyncJobId,
    ) -> Result<Option<AsyncJob>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM async_jobs WHERE application_id = $1 AND id = $2")
            .bind(application_id.as_uuid())
            .bind(job_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_job).transpose()
    }

    async fn list_async_job_items(
        &self,
        application_id: ApplicationId,
        job_id: AsyncJobId,
    ) -> Result<Vec<AsyncJobItemResult>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT * FROM async_job_item_results \
             WHERE application_id = $1 AND job_id = $2 ORDER BY ordinal",
        )
        .bind(application_id.as_uuid())
        .bind(job_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_item).collect()
    }

    async fn claim_async_jobs(
        &self,
        now: OffsetDateTime,
        leased_until: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<LeasedAsyncJob>, RepositoryError> {
        let now = postgres_time(now);
        let leased_until = postgres_time(leased_until);
        if leased_until <= now {
            return Err(RepositoryError::Invariant(
                "async job lease must end in the future".into(),
            ));
        }
        if limit == 0 {
            return Ok(Vec::new());
        }
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT * FROM async_jobs WHERE attempt_count < max_attempts AND ( \
                 (state = 'pending' AND next_attempt_at <= $1) OR \
                 (state = 'running' AND leased_until <= $1) \
             ) ORDER BY COALESCE(next_attempt_at, leased_until), created_at, id \
             FOR UPDATE SKIP LOCKED LIMIT $2",
        )
        .bind(now)
        .bind(as_i64(limit as u64)?)
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;
        let mut claimed = Vec::with_capacity(rows.len());
        for row in rows {
            let mut job = row_to_job(row)?;
            let lease_token = Uuid::new_v4().to_string();
            job.claim(&lease_token, leased_until, now)
                .map_err(invariant)?;
            update_job(&mut transaction, &job).await?;
            let pending_rows = sqlx::query(
                "SELECT media_id FROM async_job_item_results \
                 WHERE job_id = $1 AND state = 'pending' ORDER BY ordinal",
            )
            .bind(job.id().as_uuid())
            .fetch_all(&mut *transaction)
            .await
            .map_err(database_error)?;
            let pending_media_ids = pending_rows
                .into_iter()
                .map(|row| {
                    Ok(MediaId::from_uuid(
                        row.try_get("media_id").map_err(database_error)?,
                    ))
                })
                .collect::<Result<Vec<_>, RepositoryError>>()?;
            claimed.push(LeasedAsyncJob {
                job,
                lease_token,
                pending_media_ids,
            });
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(claimed)
    }

    async fn renew_async_job_lease(
        &self,
        job_id: AsyncJobId,
        lease_token: &str,
        now: OffsetDateTime,
        leased_until: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        let token = Uuid::parse_str(lease_token)
            .map_err(|_| RepositoryError::Invariant("async job lease token is invalid".into()))?;
        let now = postgres_time(now);
        let leased_until = postgres_time(leased_until);
        if leased_until <= now {
            return Err(RepositoryError::Invariant(
                "async job lease must end in the future".into(),
            ));
        }
        let result = sqlx::query(
            "UPDATE async_jobs SET leased_until = $1, updated_at = $2 \
             WHERE id = $3 AND state = 'running' AND lease_token = $4 \
               AND leased_until > $2",
        )
        .bind(leased_until)
        .bind(now)
        .bind(job_id.as_uuid())
        .bind(token)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn complete_async_job(
        &self,
        job_id: AsyncJobId,
        lease_token: &str,
        item_results: &[AsyncJobItemResult],
        completed_at: OffsetDateTime,
    ) -> Result<AsyncJobCompletion, RepositoryError> {
        let completed_at = postgres_time(completed_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let mut job = locked_job(&mut transaction, job_id).await?;
        if job.state() == AsyncJobState::Completed {
            transaction.commit().await.map_err(database_error)?;
            return Ok(AsyncJobCompletion::AlreadyCompleted(job));
        }
        if !owns_lease(&job, lease_token, completed_at) {
            return Ok(AsyncJobCompletion::LeaseLost);
        }
        let pending_rows = sqlx::query(
            "SELECT media_id FROM async_job_item_results \
             WHERE job_id = $1 AND state = 'pending' FOR UPDATE",
        )
        .bind(job_id.as_uuid())
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;
        let pending = pending_rows
            .into_iter()
            .map(|row| row.try_get::<Uuid, _>("media_id").map_err(database_error))
            .collect::<Result<HashSet<_>, _>>()?;
        let supplied = item_results
            .iter()
            .map(|item| item.media_id.as_uuid())
            .collect::<HashSet<_>>();
        if pending != supplied || supplied.len() != item_results.len() {
            return Err(RepositoryError::Invariant(
                "async job completion does not cover pending items exactly once".into(),
            ));
        }
        for item in item_results {
            validate_item(&job, item, completed_at)?;
            update_item_result(&mut transaction, item).await?;
        }
        let counts = sqlx::query(
            "SELECT count(*) FILTER (WHERE state = 'succeeded') AS succeeded, \
             count(*) FILTER (WHERE state IN ('failed', 'cancelled')) AS failed, \
             count(*) FILTER (WHERE state = 'pending') AS pending \
             FROM async_job_item_results WHERE job_id = $1",
        )
        .bind(job_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let pending_count: i64 = counts.try_get("pending").map_err(database_error)?;
        if pending_count != 0 {
            return Err(RepositoryError::Invariant(
                "async job still has pending items after completion".into(),
            ));
        }
        let succeeded = count_as_u32(counts.try_get("succeeded").map_err(database_error)?)?;
        let failed = count_as_u32(counts.try_get("failed").map_err(database_error)?)?;
        job.complete(lease_token, succeeded, failed, completed_at)
            .map_err(invariant)?;
        update_job(&mut transaction, &job).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(AsyncJobCompletion::Completed(job))
    }

    async fn fail_async_job(
        &self,
        job_id: AsyncJobId,
        lease_token: &str,
        error_summary: &str,
        retry_at: Option<OffsetDateTime>,
        failed_at: OffsetDateTime,
    ) -> Result<AsyncJobFailure, RepositoryError> {
        let failed_at = postgres_time(failed_at);
        let retry_at = retry_at.map(postgres_time);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let mut job = locked_job(&mut transaction, job_id).await?;
        if !owns_lease(&job, lease_token, failed_at) {
            return Ok(AsyncJobFailure::LeaseLost);
        }
        let disposition = job
            .fail(lease_token, error_summary, retry_at, failed_at)
            .map_err(invariant)?;
        update_job(&mut transaction, &job).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(match disposition {
            AsyncJobFailureDisposition::RetryScheduled { .. } => {
                AsyncJobFailure::RetryScheduled(job)
            }
            AsyncJobFailureDisposition::Terminal => AsyncJobFailure::Terminal(job),
        })
    }

    async fn cancel_async_job(
        &self,
        application_id: ApplicationId,
        job_id: AsyncJobId,
        cancelled_at: OffsetDateTime,
    ) -> Result<AsyncJobCancellation, RepositoryError> {
        let cancelled_at = postgres_time(cancelled_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query(
            "SELECT * FROM async_jobs WHERE application_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(application_id.as_uuid())
        .bind(job_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
        let mut job = row_to_job(row)?;
        let outcome = match job.state() {
            AsyncJobState::Completed => AsyncJobCancellation::Completed,
            AsyncJobState::Failed => AsyncJobCancellation::Failed,
            AsyncJobState::Cancelled => AsyncJobCancellation::AlreadyCancelled(job),
            AsyncJobState::Pending | AsyncJobState::Running => {
                job.cancel(cancelled_at).map_err(invariant)?;
                update_job(&mut transaction, &job).await?;
                sqlx::query(
                    "UPDATE async_job_item_results SET state = 'cancelled', completed_at = $1, \
                     updated_at = $1 WHERE job_id = $2 AND state = 'pending'",
                )
                .bind(cancelled_at)
                .bind(job_id.as_uuid())
                .execute(&mut *transaction)
                .await
                .map_err(database_error)?;
                AsyncJobCancellation::Cancelled(job)
            }
        };
        transaction.commit().await.map_err(database_error)?;
        Ok(outcome)
    }
}

async fn insert_job(
    transaction: &mut Transaction<'_, Postgres>,
    job: &AsyncJob,
) -> Result<bool, RepositoryError> {
    let value = job.to_persisted();
    let result = sqlx::query(
        "INSERT INTO async_jobs (id, application_id, operation_scope, idempotency_key, \
         request_hash, request_id, action_type, action_payload, state, total_items, \
         succeeded_items, failed_items, attempt_count, max_attempts, next_attempt_at, \
         lease_token, leased_until, error_summary, started_at, completed_at, failed_at, \
         cancelled_at, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, \
                 $15, $16, $17, $18, $19, $20, $21, $22, $23, $24) \
         ON CONFLICT (application_id, operation_scope, idempotency_key) DO NOTHING",
    )
    .bind(value.id.as_uuid())
    .bind(value.application_id.as_uuid())
    .bind(value.operation_scope)
    .bind(value.idempotency_key)
    .bind(value.request_hash)
    .bind(value.request_id)
    .bind(action_name(&value.action))
    .bind(Json(value.action))
    .bind(state_name(value.state))
    .bind(as_i32(value.total_items, "total items")?)
    .bind(as_i32(value.succeeded_items, "succeeded items")?)
    .bind(as_i32(value.failed_items, "failed items")?)
    .bind(as_i32(value.attempt_count, "attempt count")?)
    .bind(as_i32(value.max_attempts, "max attempts")?)
    .bind(value.next_attempt_at)
    .bind(optional_lease_uuid(value.lease_token.as_deref())?)
    .bind(value.leased_until)
    .bind(value.error_summary)
    .bind(value.started_at)
    .bind(value.completed_at)
    .bind(value.failed_at)
    .bind(value.cancelled_at)
    .bind(value.created_at)
    .bind(value.updated_at)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(result.rows_affected() == 1)
}

async fn update_job(
    transaction: &mut Transaction<'_, Postgres>,
    job: &AsyncJob,
) -> Result<(), RepositoryError> {
    let value = job.to_persisted();
    let result = sqlx::query(
        "UPDATE async_jobs SET state = $1, succeeded_items = $2, failed_items = $3, \
         attempt_count = $4, next_attempt_at = $5, lease_token = $6, leased_until = $7, \
         error_summary = $8, started_at = $9, completed_at = $10, failed_at = $11, \
         cancelled_at = $12, updated_at = $13 WHERE id = $14",
    )
    .bind(state_name(value.state))
    .bind(as_i32(value.succeeded_items, "succeeded items")?)
    .bind(as_i32(value.failed_items, "failed items")?)
    .bind(as_i32(value.attempt_count, "attempt count")?)
    .bind(value.next_attempt_at)
    .bind(optional_lease_uuid(value.lease_token.as_deref())?)
    .bind(value.leased_until)
    .bind(value.error_summary)
    .bind(value.started_at)
    .bind(value.completed_at)
    .bind(value.failed_at)
    .bind(value.cancelled_at)
    .bind(value.updated_at)
    .bind(value.id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn update_item_result(
    transaction: &mut Transaction<'_, Postgres>,
    item: &AsyncJobItemResult,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE async_job_item_results SET state = $1, attempt_count = $2, result = $3, \
         error_code = $4, error_summary = $5, started_at = $6, completed_at = $7, \
         updated_at = $7 WHERE job_id = $8 AND media_id = $9 AND state = 'pending'",
    )
    .bind(item_state_name(item.state))
    .bind(as_i32(item.attempt_count, "item attempt count")?)
    .bind(item.result.clone().map(Json))
    .bind(&item.error_code)
    .bind(&item.error_summary)
    .bind(item.started_at)
    .bind(item.completed_at)
    .bind(item.job_id.as_uuid())
    .bind(item.media_id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn find_by_idempotency_key(
    transaction: &mut Transaction<'_, Postgres>,
    application_id: ApplicationId,
    operation_scope: &str,
    idempotency_key: &str,
) -> Result<Option<AsyncJob>, RepositoryError> {
    let row = sqlx::query(
        "SELECT * FROM async_jobs WHERE application_id = $1 AND operation_scope = $2 \
         AND idempotency_key = $3 FOR UPDATE",
    )
    .bind(application_id.as_uuid())
    .bind(operation_scope)
    .bind(idempotency_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?;
    row.map(row_to_job).transpose()
}

async fn locked_job(
    transaction: &mut Transaction<'_, Postgres>,
    job_id: AsyncJobId,
) -> Result<AsyncJob, RepositoryError> {
    let row = sqlx::query("SELECT * FROM async_jobs WHERE id = $1 FOR UPDATE")
        .bind(job_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
    row_to_job(row)
}

fn row_to_job(row: PgRow) -> Result<AsyncJob, RepositoryError> {
    let action = row
        .try_get::<Json<AsyncJobAction>, _>("action_payload")
        .map_err(database_error)?
        .0;
    let action_type: String = row.try_get("action_type").map_err(database_error)?;
    if action_name(&action) != action_type {
        return Err(RepositoryError::Invariant(
            "async job action type does not match payload".into(),
        ));
    }
    AsyncJob::from_persistence(PersistedAsyncJob {
        id: AsyncJobId::from_uuid(row.try_get("id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        operation_scope: row.try_get("operation_scope").map_err(database_error)?,
        idempotency_key: row.try_get("idempotency_key").map_err(database_error)?,
        request_hash: row.try_get("request_hash").map_err(database_error)?,
        request_id: row.try_get("request_id").map_err(database_error)?,
        action,
        state: parse_state(&row.try_get::<String, _>("state").map_err(database_error)?)?,
        total_items: as_u32(row.try_get("total_items").map_err(database_error)?)?,
        succeeded_items: as_u32(row.try_get("succeeded_items").map_err(database_error)?)?,
        failed_items: as_u32(row.try_get("failed_items").map_err(database_error)?)?,
        attempt_count: as_u32(row.try_get("attempt_count").map_err(database_error)?)?,
        max_attempts: as_u32(row.try_get("max_attempts").map_err(database_error)?)?,
        next_attempt_at: row.try_get("next_attempt_at").map_err(database_error)?,
        lease_token: row
            .try_get::<Option<Uuid>, _>("lease_token")
            .map_err(database_error)?
            .map(|token| token.to_string()),
        leased_until: row.try_get("leased_until").map_err(database_error)?,
        error_summary: row.try_get("error_summary").map_err(database_error)?,
        started_at: row.try_get("started_at").map_err(database_error)?,
        completed_at: row.try_get("completed_at").map_err(database_error)?,
        failed_at: row.try_get("failed_at").map_err(database_error)?,
        cancelled_at: row.try_get("cancelled_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
    .map_err(invariant)
}

fn row_to_item(row: PgRow) -> Result<AsyncJobItemResult, RepositoryError> {
    Ok(AsyncJobItemResult {
        job_id: AsyncJobId::from_uuid(row.try_get("job_id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        media_id: MediaId::from_uuid(row.try_get("media_id").map_err(database_error)?),
        ordinal: as_u32(row.try_get("ordinal").map_err(database_error)?)?,
        state: parse_item_state(&row.try_get::<String, _>("state").map_err(database_error)?)?,
        attempt_count: as_u32(row.try_get("attempt_count").map_err(database_error)?)?,
        result: row
            .try_get::<Option<Json<Value>>, _>("result")
            .map_err(database_error)?
            .map(|value| value.0),
        error_code: row.try_get("error_code").map_err(database_error)?,
        error_summary: row.try_get("error_summary").map_err(database_error)?,
        started_at: row.try_get("started_at").map_err(database_error)?,
        completed_at: row.try_get("completed_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn validate_item(
    job: &AsyncJob,
    item: &AsyncJobItemResult,
    completed_at: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let item_started_at = item.started_at.map(postgres_time);
    let item_completed_at = item.completed_at.map(postgres_time);
    let item_updated_at = postgres_time(item.updated_at);
    if item.job_id != job.id()
        || item.application_id != job.application_id()
        || item.attempt_count != job.attempt_count()
        || !matches!(
            item.state,
            AsyncJobItemState::Succeeded | AsyncJobItemState::Failed
        )
        || item_completed_at.is_none_or(|item_completed_at| item_completed_at > completed_at)
        || item_started_at
            .zip(item_completed_at)
            .is_none_or(|(started_at, item_completed_at)| {
                item_completed_at < started_at || item_updated_at != item_completed_at
            })
    {
        return Err(RepositoryError::Invariant(
            "async job item result violates job identity or state".into(),
        ));
    }
    let valid_error = match item.state {
        AsyncJobItemState::Failed => item.error_code.is_some() && item.error_summary.is_some(),
        AsyncJobItemState::Succeeded => item.error_code.is_none() && item.error_summary.is_none(),
        AsyncJobItemState::Pending | AsyncJobItemState::Cancelled => false,
    };
    if !valid_error {
        return Err(RepositoryError::Invariant(
            "async job item error shape is invalid".into(),
        ));
    }
    Ok(())
}

fn owns_lease(job: &AsyncJob, lease_token: &str, now: OffsetDateTime) -> bool {
    job.state() == AsyncJobState::Running
        && job.lease_token() == Some(lease_token)
        && job.leased_until().is_some_and(|until| until > now)
}

const fn action_name(action: &AsyncJobAction) -> &'static str {
    match action {
        AsyncJobAction::UpdateTtlSeconds { .. } => "update_ttl_seconds",
        AsyncJobAction::UpdateVisibility { .. } => "update_visibility",
        AsyncJobAction::Delete => "delete",
    }
}

const fn state_name(state: AsyncJobState) -> &'static str {
    match state {
        AsyncJobState::Pending => "pending",
        AsyncJobState::Running => "running",
        AsyncJobState::Completed => "completed",
        AsyncJobState::Failed => "failed",
        AsyncJobState::Cancelled => "cancelled",
    }
}

fn parse_state(value: &str) -> Result<AsyncJobState, RepositoryError> {
    match value {
        "pending" => Ok(AsyncJobState::Pending),
        "running" => Ok(AsyncJobState::Running),
        "completed" => Ok(AsyncJobState::Completed),
        "failed" => Ok(AsyncJobState::Failed),
        "cancelled" => Ok(AsyncJobState::Cancelled),
        _ => Err(RepositoryError::Invariant(
            "persisted async job state is invalid".into(),
        )),
    }
}

const fn item_state_name(state: AsyncJobItemState) -> &'static str {
    match state {
        AsyncJobItemState::Pending => "pending",
        AsyncJobItemState::Succeeded => "succeeded",
        AsyncJobItemState::Failed => "failed",
        AsyncJobItemState::Cancelled => "cancelled",
    }
}

fn parse_item_state(value: &str) -> Result<AsyncJobItemState, RepositoryError> {
    match value {
        "pending" => Ok(AsyncJobItemState::Pending),
        "succeeded" => Ok(AsyncJobItemState::Succeeded),
        "failed" => Ok(AsyncJobItemState::Failed),
        "cancelled" => Ok(AsyncJobItemState::Cancelled),
        _ => Err(RepositoryError::Invariant(
            "persisted async job item state is invalid".into(),
        )),
    }
}

fn optional_lease_uuid(value: Option<&str>) -> Result<Option<Uuid>, RepositoryError> {
    value
        .map(|value| {
            Uuid::parse_str(value)
                .map_err(|_| RepositoryError::Invariant("async job lease is not a UUID".into()))
        })
        .transpose()
}

fn as_i32(value: u32, field: &str) -> Result<i32, RepositoryError> {
    i32::try_from(value)
        .map_err(|_| RepositoryError::Invariant(format!("{field} exceeds PostgreSQL INTEGER")))
}

fn count_as_u32(value: i64) -> Result<u32, RepositoryError> {
    u32::try_from(value)
        .map_err(|_| RepositoryError::Invariant("async job item count is invalid".into()))
}

fn invariant(error: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Invariant(error.to_string())
}
