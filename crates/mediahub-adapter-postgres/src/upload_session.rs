use async_trait::async_trait;
use mediahub_app::{
    OutboxEvent, RepositoryError, UploadSessionCancellation, UploadSessionCompletion,
    UploadSessionExpiration, UploadSessionRepository,
};
use mediahub_core::{
    Media, MediaState, OffsetDateTime, UploadSession, UploadSessionId, UploadSessionState,
};
use serde_json::Value;
use sqlx::{Postgres, Row, Transaction, types::Json};

use crate::{
    PostgresRepository,
    codec::{
        as_i64, database_error, postgres_time, row_to_media, row_to_upload_session,
        upload_state_name, visibility_name,
    },
    media::{insert_media, lock_object_identity},
    outbox::insert_outbox,
};

#[async_trait]
impl UploadSessionRepository for PostgresRepository {
    async fn create_upload_session(&self, session: UploadSession) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        create_in_transaction(&mut transaction, &session).await?;
        transaction.commit().await.map_err(database_error)
    }

    async fn find_upload_session(
        &self,
        upload_session_id: UploadSessionId,
    ) -> Result<Option<UploadSession>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM upload_sessions WHERE id = $1")
            .bind(upload_session_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_upload_session).transpose()
    }

    async fn complete_upload_session(
        &self,
        upload_session_id: UploadSessionId,
        media: Media,
        completed_at: OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<UploadSessionCompletion, RepositoryError> {
        let requested_completed_at = postgres_time(completed_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query("SELECT * FROM upload_sessions WHERE id = $1 FOR UPDATE")
            .bind(upload_session_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
            .ok_or(RepositoryError::NotFound)?;
        let database_now = sqlx::query_scalar::<_, OffsetDateTime>("SELECT clock_timestamp()")
            .fetch_one(&mut *transaction)
            .await
            .map_err(database_error)?;
        let completed_at = postgres_time(database_now.max(requested_completed_at));
        let mut session = row_to_upload_session(row)?;
        match session.state() {
            UploadSessionState::Completed => {
                let media = load_completed_media(&mut transaction, &session).await?;
                transaction.commit().await.map_err(database_error)?;
                return Ok(UploadSessionCompletion::AlreadyCompleted(media));
            }
            UploadSessionState::Cancelled => return Ok(UploadSessionCompletion::Cancelled),
            UploadSessionState::Expired => return Ok(UploadSessionCompletion::Expired),
            UploadSessionState::Pending => {}
        }
        if session.is_expired_at(completed_at) {
            session.expire(completed_at).map_err(invariant)?;
            release_reservation(&mut transaction, &session).await?;
            update_state(&mut transaction, &session).await?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(UploadSessionCompletion::Expired);
        }
        if media.id() != session.media_id()
            || media.application_id() != session.application_id()
            || media.bucket_id() != session.bucket_id()
            || media.object_key() != session.object_key()
            || media.size() != session.expected_size()
            || media.state() != MediaState::Uploading
            || event.application_id != session.application_id()
        {
            return Err(RepositoryError::Invariant(
                "completed media does not match upload session".into(),
            ));
        }

        let mut completed_media = media;
        completed_media
            .transition_to(MediaState::Active, completed_at)
            .map_err(invariant)?;
        session.complete(completed_at).map_err(invariant)?;
        let bytes = as_i64(session.reserved_bytes())?;
        let quota = sqlx::query(
            "UPDATE applications \
             SET reserved_bytes = reserved_bytes - $1, used_bytes = used_bytes + $1 \
             WHERE id = $2 AND reserved_bytes >= $1",
        )
        .bind(bytes)
        .bind(session.application_id().as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if quota.rows_affected() != 1 {
            return Err(RepositoryError::Invariant(
                "upload session has no matching quota reservation".into(),
            ));
        }
        insert_media(&mut transaction, &completed_media).await?;
        update_state(&mut transaction, &session).await?;
        insert_outbox(&mut transaction, &event).await?;
        let completed_media = load_completed_media(&mut transaction, &session).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(UploadSessionCompletion::Completed(completed_media))
    }

    async fn completed_upload_media(
        &self,
        upload_session_id: UploadSessionId,
    ) -> Result<Option<Media>, RepositoryError> {
        let row = sqlx::query(
            "SELECT media.* FROM upload_sessions \
             JOIN media ON media.id = upload_sessions.media_id \
             WHERE upload_sessions.id = $1 AND upload_sessions.state = 'completed'",
        )
        .bind(upload_session_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_media).transpose()
    }

    async fn cancel_upload_session(
        &self,
        upload_session_id: UploadSessionId,
        cancelled_at: OffsetDateTime,
    ) -> Result<UploadSessionCancellation, RepositoryError> {
        let cancelled_at = postgres_time(cancelled_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query("SELECT * FROM upload_sessions WHERE id = $1 FOR UPDATE")
            .bind(upload_session_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
            .ok_or(RepositoryError::NotFound)?;
        let mut session = row_to_upload_session(row)?;
        let outcome = match session.state() {
            UploadSessionState::Completed => UploadSessionCancellation::Completed,
            UploadSessionState::Expired => UploadSessionCancellation::Expired,
            UploadSessionState::Cancelled => UploadSessionCancellation::AlreadyCancelled(session),
            UploadSessionState::Pending => {
                session.cancel(cancelled_at).map_err(invariant)?;
                release_reservation(&mut transaction, &session).await?;
                update_state(&mut transaction, &session).await?;
                UploadSessionCancellation::Cancelled(session)
            }
        };
        transaction.commit().await.map_err(database_error)?;
        Ok(outcome)
    }

    async fn expire_upload_session(
        &self,
        upload_session_id: UploadSessionId,
        expired_at: OffsetDateTime,
    ) -> Result<UploadSessionExpiration, RepositoryError> {
        let expired_at = postgres_time(expired_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let outcome =
            expire_in_transaction(&mut transaction, upload_session_id, expired_at).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(outcome)
    }

    async fn expire_upload_sessions(
        &self,
        expired_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<UploadSession>, RepositoryError> {
        let expired_at = postgres_time(expired_at);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = as_i64(limit as u64)?;
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT id FROM upload_sessions \
             WHERE (state = 'pending' AND session_expires_at <= $1) \
                OR (state IN ('completed', 'expired', 'cancelled') \
                    AND session_expires_at <= $1 \
                    AND storage_cleanup_completed_at IS NULL) \
             ORDER BY CASE WHEN state = 'pending' THEN 0 ELSE 1 END, \
                      CASE WHEN state = 'pending' THEN session_expires_at ELSE updated_at END, id \
             FOR UPDATE SKIP LOCKED LIMIT $2",
        )
        .bind(expired_at)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;
        let mut expired = Vec::with_capacity(rows.len());
        for row in rows {
            let id = UploadSessionId::from_uuid(row.try_get("id").map_err(database_error)?);
            let disposition = expire_in_transaction(&mut transaction, id, expired_at).await?;
            sqlx::query("UPDATE upload_sessions SET updated_at = $1 WHERE id = $2")
                .bind(expired_at)
                .bind(id.as_uuid())
                .execute(&mut *transaction)
                .await
                .map_err(database_error)?;
            match disposition {
                UploadSessionExpiration::Expired(session)
                | UploadSessionExpiration::AlreadyExpired(session) => expired.push(session),
                UploadSessionExpiration::Cancelled => {
                    let row = sqlx::query("SELECT * FROM upload_sessions WHERE id = $1")
                        .bind(id.as_uuid())
                        .fetch_one(&mut *transaction)
                        .await
                        .map_err(database_error)?;
                    expired.push(row_to_upload_session(row)?);
                }
                UploadSessionExpiration::Completed => {
                    let row = sqlx::query("SELECT * FROM upload_sessions WHERE id = $1")
                        .bind(id.as_uuid())
                        .fetch_one(&mut *transaction)
                        .await
                        .map_err(database_error)?;
                    expired.push(row_to_upload_session(row)?);
                }
                UploadSessionExpiration::NotDue => {}
            }
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(expired)
    }

    async fn complete_upload_session_cleanup(
        &self,
        upload_session_id: UploadSessionId,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "UPDATE upload_sessions SET storage_cleanup_completed_at = CURRENT_TIMESTAMP, \
             updated_at = CURRENT_TIMESTAMP \
             WHERE id = $1 AND state IN ('completed', 'expired', 'cancelled') \
               AND storage_cleanup_completed_at IS NULL",
        )
        .bind(upload_session_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }
}

pub(crate) async fn create_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    session: &UploadSession,
) -> Result<(), RepositoryError> {
    if session.state() != UploadSessionState::Pending {
        return Err(RepositoryError::Invariant(
            "only pending upload sessions can be created".into(),
        ));
    }
    lock_object_identity(
        transaction,
        session.application_id(),
        session.bucket_id(),
        session.object_key(),
    )
    .await?;
    let object_is_reserved = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM media \
         WHERE application_id = $1 AND bucket_id = $2 AND object_key = $3) \
         OR EXISTS(SELECT 1 FROM s3_multipart_uploads WHERE application_id = $1 \
         AND bucket_id = $2 AND object_key = $3 AND state IN ('pending', 'completing'))",
    )
    .bind(session.application_id().as_uuid())
    .bind(session.bucket_id().as_uuid())
    .bind(session.object_key())
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;
    if object_is_reserved {
        return Err(RepositoryError::Conflict);
    }
    let bytes = as_i64(session.reserved_bytes())?;
    let reserved = sqlx::query(
        "UPDATE applications SET reserved_bytes = reserved_bytes + $1 \
         WHERE id = $2 AND quota_bytes - used_bytes - reserved_bytes >= $1",
    )
    .bind(bytes)
    .bind(session.application_id().as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if reserved.rows_affected() != 1 {
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM applications WHERE id = $1)",
        )
        .bind(session.application_id().as_uuid())
        .fetch_one(&mut **transaction)
        .await
        .map_err(database_error)?;
        return Err(if exists {
            RepositoryError::QuotaExceeded
        } else {
            RepositoryError::NotFound
        });
    }
    insert_upload_session(transaction, session).await
}

async fn insert_upload_session(
    transaction: &mut Transaction<'_, Postgres>,
    session: &UploadSession,
) -> Result<(), RepositoryError> {
    let value = session.to_persisted();
    sqlx::query(
        "INSERT INTO upload_sessions (id, media_id, application_id, bucket_id, object_key, \
         original_name, display_name, extension, expected_size_bytes, expected_mime, \
         storage_backend, storage_key, visibility_override, media_expires_at, user_metadata, \
         ai_metadata, session_expires_at, state, completed_at, cancelled_at, expired_at, \
         created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                 $14, $15, $16, $17, $18, $19, $20, $21, $22, $23)",
    )
    .bind(value.id.as_uuid())
    .bind(value.media_id.as_uuid())
    .bind(value.application_id.as_uuid())
    .bind(value.bucket_id.as_uuid())
    .bind(value.object_key)
    .bind(value.original_name)
    .bind(value.display_name)
    .bind(value.extension)
    .bind(as_i64(value.expected_size)?)
    .bind(value.expected_mime)
    .bind(value.storage_backend)
    .bind(value.storage_key)
    .bind(value.visibility_override.map(visibility_name))
    .bind(value.media_expires_at)
    .bind(Json(Value::Object(value.client_metadata.user().clone())))
    .bind(Json(Value::Object(value.client_metadata.ai().clone())))
    .bind(value.session_expires_at)
    .bind(upload_state_name(value.state))
    .bind(value.completed_at)
    .bind(value.cancelled_at)
    .bind(value.expired_at)
    .bind(value.created_at)
    .bind(value.updated_at)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn release_reservation(
    transaction: &mut Transaction<'_, Postgres>,
    session: &UploadSession,
) -> Result<(), RepositoryError> {
    let bytes = as_i64(session.reserved_bytes())?;
    let result = sqlx::query(
        "UPDATE applications SET reserved_bytes = reserved_bytes - $1 \
         WHERE id = $2 AND reserved_bytes >= $1",
    )
    .bind(bytes)
    .bind(session.application_id().as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Invariant(
            "upload session has no matching quota reservation".into(),
        ))
    }
}

async fn update_state(
    transaction: &mut Transaction<'_, Postgres>,
    session: &UploadSession,
) -> Result<(), RepositoryError> {
    let value = session.to_persisted();
    let result = sqlx::query(
        "UPDATE upload_sessions SET state = $1, completed_at = $2, cancelled_at = $3, \
         expired_at = $4, updated_at = $5 WHERE id = $6 AND state = 'pending'",
    )
    .bind(upload_state_name(value.state))
    .bind(value.completed_at)
    .bind(value.cancelled_at)
    .bind(value.expired_at)
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

async fn expire_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    upload_session_id: UploadSessionId,
    expired_at: OffsetDateTime,
) -> Result<UploadSessionExpiration, RepositoryError> {
    let row = sqlx::query("SELECT * FROM upload_sessions WHERE id = $1 FOR UPDATE")
        .bind(upload_session_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
    let mut session = row_to_upload_session(row)?;
    match session.state() {
        UploadSessionState::Completed => return Ok(UploadSessionExpiration::Completed),
        UploadSessionState::Cancelled => return Ok(UploadSessionExpiration::Cancelled),
        UploadSessionState::Expired => {
            return Ok(UploadSessionExpiration::AlreadyExpired(session));
        }
        UploadSessionState::Pending => {}
    }
    if !session.is_expired_at(expired_at) {
        return Ok(UploadSessionExpiration::NotDue);
    }
    session.expire(expired_at).map_err(invariant)?;
    release_reservation(transaction, &session).await?;
    update_state(transaction, &session).await?;
    Ok(UploadSessionExpiration::Expired(session))
}

async fn load_completed_media(
    transaction: &mut Transaction<'_, Postgres>,
    session: &UploadSession,
) -> Result<Media, RepositoryError> {
    let row = sqlx::query("SELECT * FROM media WHERE id = $1")
        .bind(session.media_id().as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?
        .ok_or_else(|| {
            RepositoryError::Invariant("completed upload session has no media".into())
        })?;
    row_to_media(row)
}

fn invariant(error: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Invariant(error.to_string())
}
