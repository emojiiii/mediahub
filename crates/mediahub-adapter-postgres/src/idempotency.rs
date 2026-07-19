use mediahub_app::{
    CompletedIdempotencyResponse, IdempotencyClaim, IdempotencyContext, RepositoryError,
};
use mediahub_core::{ApplicationId, Bucket, OffsetDateTime, UploadSession};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

use crate::{
    PostgresRepository,
    codec::{database_error, postgres_time},
    media::insert_bucket,
    upload_session::create_in_transaction,
};

impl PostgresRepository {
    pub async fn claim_idempotency_key(
        &self,
        application_id: ApplicationId,
        operation_scope: &str,
        idempotency_key: &str,
        request_hash: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<IdempotencyClaim, RepositoryError> {
        let now = postgres_time(now);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        sqlx::query("DELETE FROM idempotency_keys WHERE expires_at <= $1")
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        let inserted = sqlx::query(
            "INSERT INTO idempotency_keys (id, application_id, operation_scope, \
             idempotency_key, request_hash, status, expires_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, 'in_progress', $6, $7) \
             ON CONFLICT (application_id, operation_scope, idempotency_key) DO NOTHING",
        )
        .bind(Uuid::new_v4())
        .bind(application_id.as_uuid())
        .bind(operation_scope)
        .bind(idempotency_key)
        .bind(request_hash)
        .bind(postgres_time(expires_at))
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if inserted.rows_affected() == 1 {
            transaction.commit().await.map_err(database_error)?;
            return Ok(IdempotencyClaim::Claimed);
        }

        let row = sqlx::query(
            "SELECT request_hash, status, response_status, response_payload, resource_id \
             FROM idempotency_keys WHERE application_id = $1 AND operation_scope = $2 \
             AND idempotency_key = $3 FOR UPDATE",
        )
        .bind(application_id.as_uuid())
        .bind(operation_scope)
        .bind(idempotency_key)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or_else(|| {
            RepositoryError::Invariant("conflicting idempotency key disappeared".into())
        })?;
        let stored_hash: String = row.try_get("request_hash").map_err(database_error)?;
        let claim = if stored_hash != request_hash {
            IdempotencyClaim::Conflict
        } else {
            match row
                .try_get::<String, _>("status")
                .map_err(database_error)?
                .as_str()
            {
                "in_progress" => IdempotencyClaim::InProgress,
                "completed" => IdempotencyClaim::Completed(completed_response(&row)?),
                _ => {
                    return Err(RepositoryError::Invariant(
                        "idempotency key status is invalid".into(),
                    ));
                }
            }
        };
        transaction.commit().await.map_err(database_error)?;
        Ok(claim)
    }

    pub async fn complete_idempotency_key(
        &self,
        application_id: ApplicationId,
        operation_scope: &str,
        idempotency_key: &str,
        request_hash: &str,
        response: &CompletedIdempotencyResponse,
        completed_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let context = IdempotencyContext {
            application_id,
            operation_scope: operation_scope.to_owned(),
            key: idempotency_key.to_owned(),
            request_hash: request_hash.to_owned(),
        };
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        complete_in_transaction(&mut transaction, &context, response, completed_at).await?;
        transaction.commit().await.map_err(database_error)
    }

    pub async fn create_bucket_and_complete_idempotency(
        &self,
        bucket: &Bucket,
        idempotency: &IdempotencyContext,
        response: &CompletedIdempotencyResponse,
        completed_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        if bucket.application_id() != idempotency.application_id {
            return Err(RepositoryError::Invariant(
                "bucket and idempotency application do not match".into(),
            ));
        }
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        insert_bucket(&mut transaction, bucket).await?;
        complete_in_transaction(&mut transaction, idempotency, response, completed_at).await?;
        transaction.commit().await.map_err(database_error)
    }

    pub async fn create_upload_session_and_complete_idempotency(
        &self,
        session: &UploadSession,
        idempotency: &IdempotencyContext,
        response: &CompletedIdempotencyResponse,
        completed_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        if session.application_id() != idempotency.application_id {
            return Err(RepositoryError::Invariant(
                "upload session and idempotency application do not match".into(),
            ));
        }
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        create_in_transaction(&mut transaction, session).await?;
        complete_in_transaction(&mut transaction, idempotency, response, completed_at).await?;
        transaction.commit().await.map_err(database_error)
    }

    pub async fn release_idempotency_key(
        &self,
        idempotency: &IdempotencyContext,
    ) -> Result<(), RepositoryError> {
        sqlx::query(
            "DELETE FROM idempotency_keys WHERE application_id = $1 AND operation_scope = $2 \
             AND idempotency_key = $3 AND request_hash = $4 AND status = 'in_progress'",
        )
        .bind(idempotency.application_id.as_uuid())
        .bind(&idempotency.operation_scope)
        .bind(&idempotency.key)
        .bind(&idempotency.request_hash)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(())
    }
}

async fn complete_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    idempotency: &IdempotencyContext,
    response: &CompletedIdempotencyResponse,
    completed_at: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let completed = sqlx::query(
        "UPDATE idempotency_keys SET status = 'completed', response_status = $1, \
         response_payload = $2, resource_id = $3, completed_at = $4 \
         WHERE application_id = $5 AND operation_scope = $6 AND idempotency_key = $7 \
         AND request_hash = $8 AND status = 'in_progress'",
    )
    .bind(i32::from(response.status))
    .bind(&response.payload)
    .bind(&response.resource_id)
    .bind(postgres_time(completed_at))
    .bind(idempotency.application_id.as_uuid())
    .bind(&idempotency.operation_scope)
    .bind(&idempotency.key)
    .bind(&idempotency.request_hash)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if completed.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

fn completed_response(
    row: &sqlx::postgres::PgRow,
) -> Result<CompletedIdempotencyResponse, RepositoryError> {
    let status = row
        .try_get::<Option<i32>, _>("response_status")
        .map_err(database_error)?
        .ok_or_else(|| {
            RepositoryError::Invariant("completed idempotency response is missing a status".into())
        })?
        .try_into()
        .map_err(|_| {
            RepositoryError::Invariant("completed idempotency response status is invalid".into())
        })?;
    let payload = row
        .try_get::<Option<String>, _>("response_payload")
        .map_err(database_error)?
        .ok_or_else(|| {
            RepositoryError::Invariant("completed idempotency response is missing a payload".into())
        })?;
    Ok(CompletedIdempotencyResponse {
        status,
        payload,
        resource_id: row.try_get("resource_id").map_err(database_error)?,
    })
}
