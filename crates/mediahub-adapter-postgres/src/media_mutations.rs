// Media upload, state, and mutation operations.

impl PostgresRepository {
    pub async fn finalize_delete(
        &self,
        media_id: MediaId,
        deleted_at: OffsetDateTime,
    ) -> Result<Media, RepositoryError> {
        let deleted_at = postgres_time(deleted_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query("SELECT * FROM media WHERE id = $1 FOR UPDATE")
            .bind(media_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
            .ok_or(RepositoryError::NotFound)?;
        let mut media = row_to_media(row)?;
        if media.state() == MediaState::Deleted {
            transaction.commit().await.map_err(database_error)?;
            return Ok(media);
        }
        if media.state() != MediaState::DeletePending {
            return Err(RepositoryError::Conflict);
        }
        media
            .transition_to(MediaState::Deleted, deleted_at)
            .map_err(invariant)?;
        let persisted = media.to_persisted();
        let size = as_i64(media.size())?;
        let updated = sqlx::query(
            "UPDATE media SET state = 'deleted', revision = $1, deleted_at = $2, \
             updated_at = $3, original_name = NULL, display_name = 'deleted', extension = NULL, \
             user_metadata = '{}'::jsonb, ai_metadata = '{}'::jsonb \
             WHERE id = $4 AND state = 'delete_pending'",
        )
        .bind(as_i64(persisted.revision)?)
        .bind(persisted.deleted_at)
        .bind(persisted.updated_at)
        .bind(media_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if updated.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }
        let quota = sqlx::query(
            "UPDATE applications SET used_bytes = GREATEST(used_bytes - $1, 0) WHERE id = $2",
        )
        .bind(size)
        .bind(media.application_id().as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if quota.rows_affected() != 1 {
            return Err(RepositoryError::Invariant(
                "media application is missing during deletion".into(),
            ));
        }
        sqlx::query("DELETE FROM variants WHERE media_id = $1")
            .bind(media_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        let event = OutboxEvent {
            id: format!("media.deleted:{media_id}"),
            application_id: media.application_id(),
            event_type: "media.deleted".into(),
            aggregate_id: media_id.to_string(),
            payload: serde_json::json!({ "media_id": media_id.to_string() }),
            created_at: deleted_at,
            delivered_at: None,
            next_attempt_at: Some(deleted_at),
            attempt_count: 0,
        };
        insert_outbox(&mut transaction, &event).await?;
        let tombstone = sqlx::query("SELECT * FROM media WHERE id = $1")
            .bind(media_id.as_uuid())
            .fetch_one(&mut *transaction)
            .await
            .map_err(database_error)
            .and_then(row_to_media)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(tombstone)
    }
}

pub(crate) async fn insert_bucket(
    transaction: &mut Transaction<'_, Postgres>,
    bucket: &Bucket,
) -> Result<(), RepositoryError> {
    let policy = bucket.policy();
    let allowed_mime_types = policy.allowed_mime_types().collect::<Vec<_>>();
    sqlx::query(
        "INSERT INTO buckets (id, application_id, name, visibility, default_ttl_seconds, \
         max_object_bytes, allowed_mime_types, lifecycle_policy, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(bucket.id().as_uuid())
    .bind(bucket.application_id().as_uuid())
    .bind(bucket.name())
    .bind(visibility_name(policy.visibility()))
    .bind(policy.default_ttl_seconds().map(as_i64).transpose()?)
    .bind(policy.max_object_size().map(as_i64).transpose()?)
    .bind(Json(allowed_mime_types))
    .bind(Json(policy.lifecycle_rules()))
    .bind(postgres_time(bucket.created_at()))
    .bind(postgres_time(bucket.updated_at()))
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

fn positive_limit(limit: usize, message: &str) -> Result<i64, RepositoryError> {
    if limit == 0 {
        return Err(RepositoryError::Invariant(message.into()));
    }
    i64::try_from(limit).map_err(|_| RepositoryError::Invariant(message.into()))
}

#[async_trait]
impl BucketRepository for PostgresRepository {
    async fn find_by_id(&self, bucket_id: BucketId) -> Result<Option<Bucket>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM buckets WHERE id = $1")
            .bind(bucket_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_bucket).transpose()
    }
}

#[async_trait]
impl MediaRepository for PostgresRepository {
    async fn find_by_object_key(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        object_key: &str,
    ) -> Result<Option<Media>, RepositoryError> {
        let row = sqlx::query(
            "SELECT * FROM media \
             WHERE application_id = $1 AND bucket_id = $2 AND object_key = $3",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_id.as_uuid())
        .bind(object_key)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_media).transpose()
    }

    async fn reserve_quota(
        &self,
        application_id: ApplicationId,
        bytes: u64,
    ) -> Result<(), RepositoryError> {
        let bytes = as_i64(bytes)?;
        let result = sqlx::query(
            "UPDATE applications SET reserved_bytes = reserved_bytes + $1 \
             WHERE id = $2 AND quota_bytes - used_bytes - reserved_bytes >= $1",
        )
        .bind(bytes)
        .bind(application_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 1 {
            return Ok(());
        }
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM applications WHERE id = $1)",
        )
        .bind(application_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(database_error)?;
        Err(if exists {
            RepositoryError::QuotaExceeded
        } else {
            RepositoryError::NotFound
        })
    }

    async fn create_uploading(&self, media: Media) -> Result<(), RepositoryError> {
        if media.state() != MediaState::Uploading {
            return Err(RepositoryError::Invariant(
                "only uploading media can be created".into(),
            ));
        }
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        lock_object_identity(
            &mut transaction,
            media.application_id(),
            media.bucket_id(),
            media.object_key(),
        )
        .await?;
        let object_is_reserved = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM upload_sessions WHERE application_id = $1 \
             AND bucket_id = $2 AND object_key = $3 AND state = 'pending') \
             OR EXISTS(SELECT 1 FROM s3_multipart_uploads WHERE application_id = $1 \
             AND bucket_id = $2 AND object_key = $3 AND state IN ('pending', 'completing'))",
        )
        .bind(media.application_id().as_uuid())
        .bind(media.bucket_id().as_uuid())
        .bind(media.object_key())
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if object_is_reserved {
            return Err(RepositoryError::Conflict);
        }
        insert_media(&mut transaction, &media).await?;
        transaction.commit().await.map_err(database_error)
    }

    async fn commit_upload(
        &self,
        media_id: MediaId,
        committed_at: mediahub_core::OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<Media, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let persisted =
            commit_upload_in_transaction(&mut transaction, media_id, committed_at, &event).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(persisted)
    }

    async fn abort_upload(&self, media_id: MediaId) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query(
            "SELECT application_id, state, size_bytes FROM media WHERE id = $1 FOR UPDATE",
        )
        .bind(media_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(());
        };
        if row.try_get::<String, _>("state").map_err(database_error)? != "uploading" {
            transaction.commit().await.map_err(database_error)?;
            return Ok(());
        }
        let application_id: uuid::Uuid = row.try_get("application_id").map_err(database_error)?;
        let size: i64 = row.try_get("size_bytes").map_err(database_error)?;
        sqlx::query("DELETE FROM media WHERE id = $1")
            .bind(media_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        let released = sqlx::query(
            "UPDATE applications SET reserved_bytes = reserved_bytes - $1 \
             WHERE id = $2 AND reserved_bytes >= $1",
        )
        .bind(size)
        .bind(application_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if released.rows_affected() != 1 {
            return Err(RepositoryError::Invariant(
                "aborted upload has no matching quota reservation".into(),
            ));
        }
        transaction.commit().await.map_err(database_error)
    }

    async fn release_quota(
        &self,
        application_id: ApplicationId,
        bytes: u64,
    ) -> Result<(), RepositoryError> {
        let bytes = as_i64(bytes)?;
        let result = sqlx::query(
            "UPDATE applications SET reserved_bytes = reserved_bytes - $1 \
             WHERE id = $2 AND reserved_bytes >= $1",
        )
        .bind(bytes)
        .bind(application_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 1 {
            return Ok(());
        }
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM applications WHERE id = $1)",
        )
        .bind(application_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(database_error)?;
        Err(if exists {
            RepositoryError::Invariant("quota reservation is smaller than release".into())
        } else {
            RepositoryError::NotFound
        })
    }

    async fn update_media(
        &self,
        media: Media,
        expected_revision: u64,
        event: OutboxEvent,
    ) -> Result<(), RepositoryError> {
        if event.application_id != media.application_id()
            || event.aggregate_id != media.id().to_string()
        {
            return Err(RepositoryError::Invariant(
                "media update event does not match media identity".into(),
            ));
        }
        let persisted = media.to_persisted();
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let result = sqlx::query(
            "UPDATE media SET display_name = $1, visibility_override = $2, \
                 user_metadata = $3, ai_metadata = $4, revision = $5, \
                 expires_at = $6, updated_at = $7 \
             WHERE id = $8 AND application_id = $9 AND state = 'active' AND revision = $10",
        )
        .bind(persisted.display_name)
        .bind(persisted.visibility_override.map(visibility_name))
        .bind(Json(Value::Object(
            persisted.client_metadata.user().clone(),
        )))
        .bind(Json(Value::Object(persisted.client_metadata.ai().clone())))
        .bind(as_i64(persisted.revision)?)
        .bind(persisted.expire_at)
        .bind(persisted.updated_at)
        .bind(persisted.id.as_uuid())
        .bind(persisted.application_id.as_uuid())
        .bind(as_i64(expected_revision)?)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }
        insert_outbox(&mut transaction, &event).await?;
        transaction.commit().await.map_err(database_error)
    }

    async fn schedule_delete(
        &self,
        media_id: MediaId,
        deleted_at: mediahub_core::OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<Media, RepositoryError> {
        let deleted_at = postgres_time(deleted_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query("SELECT * FROM media WHERE id = $1 FOR UPDATE")
            .bind(media_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
            .ok_or(RepositoryError::NotFound)?;
        let mut media = row_to_media(row)?;
        if media.state() == MediaState::DeletePending {
            transaction.commit().await.map_err(database_error)?;
            return Ok(media);
        }
        if media.state() != MediaState::Active || media.application_id() != event.application_id {
            return Err(RepositoryError::Conflict);
        }
        media
            .transition_to(MediaState::DeletePending, deleted_at)
            .map_err(invariant)?;
        let result = sqlx::query(
            "UPDATE media SET state = 'delete_pending', revision = $1, updated_at = $2 \
             WHERE id = $3 AND state = 'active'",
        )
        .bind(as_i64(media.revision())?)
        .bind(media.updated_at())
        .bind(media_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }
        insert_outbox(&mut transaction, &event).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(media)
    }
}

pub(crate) async fn commit_upload_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    media_id: MediaId,
    committed_at: OffsetDateTime,
    event: &OutboxEvent,
) -> Result<Media, RepositoryError> {
    let committed_at = postgres_time(committed_at);
    let row = sqlx::query("SELECT * FROM media WHERE id = $1 FOR UPDATE")
        .bind(media_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
    let mut media = row_to_media(row)?;
    if media.state() == MediaState::Active {
        let event_exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM outbox_events WHERE id = $1)",
        )
        .bind(&event.id)
        .fetch_one(&mut **transaction)
        .await
        .map_err(database_error)?;
        if event_exists {
            return Ok(media);
        }
    }
    if media.state() != MediaState::Uploading || media.application_id() != event.application_id {
        return Err(RepositoryError::Conflict);
    }
    media
        .transition_to(MediaState::Active, committed_at)
        .map_err(invariant)?;
    let size = as_i64(media.size())?;
    let quota = sqlx::query(
        "UPDATE applications \
         SET reserved_bytes = reserved_bytes - $1, used_bytes = used_bytes + $1 \
         WHERE id = $2 AND reserved_bytes >= $1",
    )
    .bind(size)
    .bind(media.application_id().as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if quota.rows_affected() != 1 {
        return Err(RepositoryError::Invariant(
            "upload has no matching quota reservation".into(),
        ));
    }
    update_media_state(transaction, &media).await?;
    insert_outbox(transaction, event).await?;
    sqlx::query("SELECT * FROM media WHERE id = $1")
        .bind(media_id.as_uuid())
        .fetch_one(&mut **transaction)
        .await
        .map_err(database_error)
        .and_then(row_to_media)
}

pub(crate) async fn insert_media(
    transaction: &mut Transaction<'_, Postgres>,
    media: &Media,
) -> Result<(), RepositoryError> {
    let persisted = media.to_persisted();
    sqlx::query(
        "INSERT INTO media (id, application_id, bucket_id, object_key, original_name, \
         display_name, extension, storage_key, storage_backend, state, visibility_override, \
         content_type, size_bytes, sha256, width, height, duration_ms, user_metadata, \
         ai_metadata, metadata_version, revision, expires_at, archived_at, deleted_at, \
         created_at, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, \
                 $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26)",
    )
    .bind(persisted.id.as_uuid())
    .bind(persisted.application_id.as_uuid())
    .bind(persisted.bucket_id.as_uuid())
    .bind(persisted.object_key)
    .bind(persisted.original_name)
    .bind(persisted.display_name)
    .bind(persisted.extension)
    .bind(persisted.storage_key)
    .bind(persisted.storage_backend)
    .bind(media_state_name(persisted.state))
    .bind(persisted.visibility_override.map(visibility_name))
    .bind(persisted.system_metadata.mime)
    .bind(as_i64(persisted.system_metadata.size)?)
    .bind(persisted.system_metadata.sha256)
    .bind(persisted.system_metadata.width.map(i64::from))
    .bind(persisted.system_metadata.height.map(i64::from))
    .bind(
        persisted
            .system_metadata
            .duration_ms
            .map(as_i64)
            .transpose()?,
    )
    .bind(Json(Value::Object(
        persisted.client_metadata.user().clone(),
    )))
    .bind(Json(Value::Object(persisted.client_metadata.ai().clone())))
    .bind(i32::try_from(persisted.metadata_version).map_err(|_| {
        RepositoryError::Invariant("metadata version exceeds PostgreSQL INTEGER".into())
    })?)
    .bind(as_i64(persisted.revision)?)
    .bind(persisted.expire_at)
    .bind(persisted.archived_at)
    .bind(persisted.deleted_at)
    .bind(persisted.created_at)
    .bind(persisted.updated_at)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

pub(crate) async fn lock_object_identity(
    transaction: &mut Transaction<'_, Postgres>,
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: &str,
) -> Result<(), RepositoryError> {
    let identity = format!("{application_id}\n{bucket_id}\n{object_key}");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(identity)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    Ok(())
}

async fn update_media_state(
    transaction: &mut Transaction<'_, Postgres>,
    media: &Media,
) -> Result<(), RepositoryError> {
    let persisted: PersistedMedia = media.to_persisted();
    let result = sqlx::query(
        "UPDATE media SET state = $1, revision = $2, archived_at = $3, \
         deleted_at = $4, updated_at = $5 WHERE id = $6",
    )
    .bind(media_state_name(persisted.state))
    .bind(as_i64(persisted.revision)?)
    .bind(persisted.archived_at)
    .bind(persisted.deleted_at)
    .bind(persisted.updated_at)
    .bind(persisted.id.as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

