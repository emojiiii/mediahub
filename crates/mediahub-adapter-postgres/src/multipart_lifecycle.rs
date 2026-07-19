// Multipart upload lifecycle operations.

#[async_trait]
impl S3MultipartRepository for PostgresRepository {
    async fn create_multipart_upload(
        &self,
        upload: NewS3MultipartUpload,
    ) -> Result<S3MultipartUpload, RepositoryError> {
        upload.validate()?;
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        lock_object_identity(
            &mut transaction,
            upload.application_id,
            upload.bucket_id,
            &upload.object_key,
        )
        .await?;
        let application_exists = sqlx::query_scalar::<_, uuid::Uuid>(
            "SELECT id FROM applications WHERE id = $1 FOR UPDATE",
        )
        .bind(upload.application_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .is_some();
        if !application_exists {
            return Err(RepositoryError::NotFound);
        }
        let active_uploads = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM s3_multipart_uploads WHERE application_id = $1 \
             AND state IN ('pending', 'completing')",
        )
        .bind(upload.application_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if active_uploads
            >= i64::try_from(MAX_S3_MULTIPART_ACTIVE_UPLOADS_PER_APPLICATION).map_err(|_| {
                RepositoryError::Invariant("multipart active upload limit is too large".into())
            })?
        {
            return Err(RepositoryError::QuotaExceeded);
        }
        let bucket_exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM buckets WHERE id = $1 AND application_id = $2)",
        )
        .bind(upload.bucket_id.as_uuid())
        .bind(upload.application_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if !bucket_exists {
            return Err(RepositoryError::NotFound);
        }
        let object_is_claimed = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM media WHERE application_id = $1 AND bucket_id = $2 \
             AND object_key = $3) OR EXISTS(SELECT 1 FROM upload_sessions \
             WHERE application_id = $1 AND bucket_id = $2 AND object_key = $3 \
             AND state = 'pending')",
        )
        .bind(upload.application_id.as_uuid())
        .bind(upload.bucket_id.as_uuid())
        .bind(&upload.object_key)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if object_is_claimed {
            return Err(RepositoryError::Conflict);
        }
        let created_at = postgres_time(upload.created_at);
        let expires_at = postgres_time(upload.expires_at);
        let row = sqlx::query(
            "INSERT INTO s3_multipart_uploads (upload_id, application_id, bucket_id, object_key, \
             content_type, visibility_override, state, expires_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, 'pending', $7, $8, $8) RETURNING *",
        )
        .bind(upload.upload_id)
        .bind(upload.application_id.as_uuid())
        .bind(upload.bucket_id.as_uuid())
        .bind(upload.object_key)
        .bind(upload.content_type)
        .bind(upload.visibility_override.map(visibility_name))
        .bind(expires_at)
        .bind(created_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let upload = row_to_multipart_upload(row)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(upload)
    }

    async fn find_multipart_upload(
        &self,
        upload_id: &str,
    ) -> Result<Option<S3MultipartUpload>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM s3_multipart_uploads WHERE upload_id = $1")
            .bind(upload_id)
            .fetch_optional(self.pool())
            .await
            .map_err(database_error)?;
        row.map(row_to_multipart_upload).transpose()
    }

    async fn put_multipart_part(
        &self,
        upload_id: &str,
        part: NewS3MultipartPart,
        maximum_upload_size: u64,
        now: OffsetDateTime,
    ) -> Result<S3MultipartPartPut, RepositoryError> {
        part.validate()?;
        if maximum_upload_size == 0 {
            return Err(RepositoryError::Invariant(
                "multipart maximum upload size must be positive".into(),
            ));
        }
        let now = postgres_time(now);
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let mut upload = lock_upload(&mut transaction, upload_id).await?;
        if upload.state != S3MultipartUploadState::Pending {
            transaction.commit().await.map_err(database_error)?;
            return Ok(S3MultipartPartPut::NotPending(upload));
        }
        if upload.expires_at <= now {
            abort_and_release_parts(&mut transaction, &upload, now).await?;
            upload = lock_upload(&mut transaction, upload_id).await?;
            let storage_keys = list_storage_keys(&mut transaction, upload_id).await?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(S3MultipartPartPut::Expired {
                upload,
                storage_keys,
            });
        }
        let previous = sqlx::query(
            "SELECT size_bytes, storage_key FROM s3_multipart_parts \
             WHERE upload_id = $1 AND part_number = $2",
        )
        .bind(upload_id)
        .bind(i32::from(part.part_number))
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?;
        let previous_size = previous
            .as_ref()
            .map(|row| row.try_get::<i64, _>("size_bytes"))
            .transpose()
            .map_err(database_error)?
            .unwrap_or(0);
        let previous_storage_key = previous
            .as_ref()
            .map(|row| row.try_get::<String, _>("storage_key"))
            .transpose()
            .map_err(database_error)?;
        let current_size = multipart_reserved_bytes(&mut transaction, upload_id).await?;
        let new_part_size = as_i64(part.size)?;
        let new_total = current_size
            .checked_sub(previous_size)
            .and_then(|size| size.checked_add(new_part_size))
            .ok_or_else(|| RepositoryError::Invariant("multipart size overflow".into()))?;
        if new_total > as_i64(maximum_upload_size)? {
            return Err(RepositoryError::QuotaExceeded);
        }
        adjust_reserved_bytes(
            &mut transaction,
            upload.application_id,
            new_part_size - previous_size,
        )
        .await?;
        let row = sqlx::query(
            "INSERT INTO s3_multipart_parts (upload_id, part_number, size_bytes, sha256, etag, \
             storage_key, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $7) \
             ON CONFLICT (upload_id, part_number) DO UPDATE SET size_bytes = EXCLUDED.size_bytes, \
             sha256 = EXCLUDED.sha256, etag = EXCLUDED.etag, storage_key = EXCLUDED.storage_key, \
             updated_at = EXCLUDED.updated_at RETURNING *",
        )
        .bind(upload_id)
        .bind(i32::from(part.part_number))
        .bind(as_i64(part.size)?)
        .bind(part.sha256)
        .bind(part.etag)
        .bind(&part.storage_key)
        .bind(now)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let stored = row_to_multipart_part(row)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(S3MultipartPartPut::Stored {
            part: stored,
            replaced_storage_key: previous_storage_key.filter(|key| key != &part.storage_key),
        })
    }

    async fn list_multipart_parts(
        &self,
        upload_id: &str,
    ) -> Result<Vec<S3MultipartPart>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT * FROM s3_multipart_parts WHERE upload_id = $1 ORDER BY part_number",
        )
        .bind(upload_id)
        .fetch_all(self.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_multipart_part).collect()
    }

    async fn claim_multipart_completion(
        &self,
        upload_id: &str,
        manifest: &[CompletedS3MultipartPart],
        completion_token: &str,
        lease_until: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<S3MultipartCompletionClaim, RepositoryError> {
        if completion_token.is_empty() || lease_until <= now {
            return Err(RepositoryError::Invariant(
                "multipart completion token must be non-empty and lease must be in the future"
                    .into(),
            ));
        }
        let now = postgres_time(now);
        let lease_until = postgres_time(lease_until);
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let mut upload = lock_upload(&mut transaction, upload_id).await?;
        let taking_over = upload.state == S3MultipartUploadState::Completing;
        match upload.state {
            S3MultipartUploadState::Completed => {
                return Ok(S3MultipartCompletionClaim::AlreadyCompleted(upload));
            }
            S3MultipartUploadState::Aborted => {
                return Ok(S3MultipartCompletionClaim::Aborted(upload));
            }
            S3MultipartUploadState::Completing
                if upload
                    .completion_lease_until
                    .is_some_and(|until| until > now) =>
            {
                return Ok(S3MultipartCompletionClaim::InProgress(upload));
            }
            S3MultipartUploadState::Pending | S3MultipartUploadState::Completing => {}
        }
        if upload.expires_at <= now {
            abort_and_release_parts(&mut transaction, &upload, now).await?;
            upload = lock_upload(&mut transaction, upload_id).await?;
            let storage_keys = list_storage_keys(&mut transaction, upload_id).await?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(S3MultipartCompletionClaim::Expired {
                upload,
                storage_keys,
            });
        }
        if taking_over {
            let stored_manifest = load_completion_manifest(&mut transaction, upload_id).await?;
            if stored_manifest != manifest {
                transaction.commit().await.map_err(database_error)?;
                return Ok(S3MultipartCompletionClaim::InProgress(upload));
            }
        }
        let parts = list_parts(&mut transaction, upload_id).await?;
        let selected = match validate_manifest(manifest, &parts) {
            Ok(selected) => selected,
            Err(error) => return Ok(S3MultipartCompletionClaim::InvalidManifest(error)),
        };
        let selected_numbers = selected
            .iter()
            .map(|part| part.part_number)
            .collect::<BTreeSet<_>>();
        let unused_storage_keys = parts
            .iter()
            .filter(|part| !selected_numbers.contains(&part.part_number))
            .map(|part| part.storage_key.clone())
            .collect();
        let total_size = selected.iter().try_fold(0_u64, |total, part| {
            total.checked_add(part.size).ok_or_else(|| {
                RepositoryError::Invariant("multipart total size exceeds u64".into())
            })
        })?;
        let manifest_json = serde_json::to_value(manifest)
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        sqlx::query(
            "UPDATE s3_multipart_uploads SET state = 'completing', completion_token = $1, \
             completion_lease_until = $2, completion_manifest = $3, updated_at = $4 \
             WHERE upload_id = $5",
        )
        .bind(completion_token)
        .bind(lease_until)
        .bind(Json(manifest_json))
        .bind(now)
        .bind(upload_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        upload = lock_upload(&mut transaction, upload_id).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(S3MultipartCompletionClaim::Claimed(S3MultipartManifest {
            upload,
            parts: selected,
            total_size,
            unused_storage_keys,
        }))
    }

    async fn create_uploading_for_multipart(
        &self,
        upload_id: &str,
        completion_token: &str,
        media: Media,
    ) -> Result<(), RepositoryError> {
        if completion_token.is_empty() || media.state() != MediaState::Uploading {
            return Err(RepositoryError::Invariant(
                "multipart completion requires an uploading Media and non-empty token".into(),
            ));
        }
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        lock_object_identity(
            &mut transaction,
            media.application_id(),
            media.bucket_id(),
            media.object_key(),
        )
        .await?;
        let upload = lock_upload(&mut transaction, upload_id).await?;
        validate_completion_owner(&mut transaction, &upload, completion_token).await?;
        if upload.application_id != media.application_id()
            || upload.bucket_id != media.bucket_id()
            || upload.object_key != media.object_key()
        {
            return Err(RepositoryError::Conflict);
        }
        let manifest = load_completion_manifest(&mut transaction, upload_id).await?;
        let parts = list_parts(&mut transaction, upload_id).await?;
        let selected = validate_manifest(&manifest, &parts).map_err(|error| {
            RepositoryError::Invariant(format!(
                "persisted multipart manifest is invalid: {error:?}"
            ))
        })?;
        let selected_size = selected.iter().try_fold(0_u64, |total, part| {
            total
                .checked_add(part.size)
                .ok_or_else(|| RepositoryError::Invariant("multipart size overflow".into()))
        })?;
        if selected_size != media.size() {
            return Err(RepositoryError::Invariant(
                "multipart Media size does not match the claimed manifest".into(),
            ));
        }
        let pending_session = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM upload_sessions WHERE application_id = $1 \
             AND bucket_id = $2 AND object_key = $3 AND state = 'pending')",
        )
        .bind(media.application_id().as_uuid())
        .bind(media.bucket_id().as_uuid())
        .bind(media.object_key())
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if pending_session {
            return Err(RepositoryError::Conflict);
        }
        insert_media(&mut transaction, &media).await?;
        transaction.commit().await.map_err(database_error)
    }

    async fn abort_uploading_for_multipart(
        &self,
        upload_id: &str,
        completion_token: &str,
        media_id: MediaId,
    ) -> Result<(), RepositoryError> {
        if completion_token.is_empty() {
            return Err(RepositoryError::Invariant(
                "multipart completion token must not be empty".into(),
            ));
        }
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let upload = lock_upload(&mut transaction, upload_id).await?;
        validate_completion_owner(&mut transaction, &upload, completion_token).await?;
        let row = sqlx::query("SELECT * FROM media WHERE id = $1 FOR UPDATE")
            .bind(media_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?;
        let Some(row) = row else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(());
        };
        let media = row_to_media(row)?;
        if media.state() != MediaState::Uploading
            || media.application_id() != upload.application_id
            || media.bucket_id() != upload.bucket_id
            || media.object_key() != upload.object_key
        {
            return Err(RepositoryError::Conflict);
        }
        sqlx::query("DELETE FROM media WHERE id = $1")
            .bind(media_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)
    }

    async fn commit_upload_for_multipart(
        &self,
        upload_id: &str,
        completion_token: &str,
        media_id: MediaId,
        committed_at: OffsetDateTime,
        event: OutboxEvent,
    ) -> Result<Media, RepositoryError> {
        if completion_token.is_empty() {
            return Err(RepositoryError::Invariant(
                "multipart completion token must not be empty".into(),
            ));
        }
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let upload = lock_upload(&mut transaction, upload_id).await?;
        validate_completion_owner(&mut transaction, &upload, completion_token).await?;
        if upload.application_id != event.application_id {
            return Err(RepositoryError::Conflict);
        }
        let media =
            commit_upload_in_transaction(&mut transaction, media_id, committed_at, &event).await?;
        if media.application_id() != upload.application_id
            || media.bucket_id() != upload.bucket_id
            || media.object_key() != upload.object_key
        {
            return Err(RepositoryError::Conflict);
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(media)
    }

    async fn finish_multipart_completion(
        &self,
        upload_id: &str,
        completion_token: &str,
        media_id: MediaId,
        final_etag: &str,
        now: OffsetDateTime,
    ) -> Result<S3MultipartCompletionFinish, RepositoryError> {
        if completion_token.is_empty() || final_etag.is_empty() {
            return Err(RepositoryError::Invariant(
                "multipart completion token and final etag must not be empty".into(),
            ));
        }
        let now = postgres_time(now);
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let mut upload = lock_upload(&mut transaction, upload_id).await?;
        match upload.state {
            S3MultipartUploadState::Completed => {
                return Ok(S3MultipartCompletionFinish::AlreadyCompleted(upload));
            }
            S3MultipartUploadState::Completing => {}
            S3MultipartUploadState::Pending | S3MultipartUploadState::Aborted => {
                return Ok(S3MultipartCompletionFinish::NotCompleting(upload));
            }
        }
        let owns_claim = sqlx::query_scalar::<_, bool>(
            "SELECT completion_token = $1 FROM s3_multipart_uploads WHERE upload_id = $2",
        )
        .bind(completion_token)
        .bind(upload_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if !owns_claim {
            return Ok(S3MultipartCompletionFinish::OwnershipLost(upload));
        }
        let media_row = sqlx::query("SELECT * FROM media WHERE id = $1 FOR UPDATE")
            .bind(media_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
            .ok_or(RepositoryError::NotFound)?;
        let media = row_to_media(media_row)?;
        if media.state() != MediaState::Active
            || media.application_id() != upload.application_id
            || media.bucket_id() != upload.bucket_id
            || media.object_key() != upload.object_key
            || media.etag() != final_etag
        {
            return Err(RepositoryError::Conflict);
        }
        let all_parts_size = multipart_reserved_bytes(&mut transaction, upload_id).await?;
        let selected_size = as_i64(media.size())?;
        let unused_size = all_parts_size.checked_sub(selected_size).ok_or_else(|| {
            RepositoryError::Invariant(
                "multipart part reservation is smaller than completed Media".into(),
            )
        })?;
        adjust_reserved_bytes(&mut transaction, upload.application_id, -unused_size).await?;
        sqlx::query(
            "UPDATE s3_multipart_uploads SET state = 'completed', completion_token = NULL, \
             completion_lease_until = NULL, media_id = $1, final_etag = $2, completed_at = $3, \
             updated_at = $3 WHERE upload_id = $4",
        )
        .bind(media_id.as_uuid())
        .bind(final_etag)
        .bind(now)
        .bind(upload_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        upload = lock_upload(&mut transaction, upload_id).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(S3MultipartCompletionFinish::Completed(upload))
    }

    async fn release_multipart_completion(
        &self,
        upload_id: &str,
        completion_token: &str,
        now: OffsetDateTime,
    ) -> Result<S3MultipartCompletionRelease, RepositoryError> {
        let now = postgres_time(now);
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let mut upload = lock_upload(&mut transaction, upload_id).await?;
        match upload.state {
            S3MultipartUploadState::Pending => {
                return Ok(S3MultipartCompletionRelease::AlreadyPending(upload));
            }
            S3MultipartUploadState::Completed | S3MultipartUploadState::Aborted => {
                return Ok(S3MultipartCompletionRelease::Terminal(upload));
            }
            S3MultipartUploadState::Completing => {}
        }
        let released = sqlx::query(
            "UPDATE s3_multipart_uploads SET state = 'pending', completion_token = NULL, \
             completion_lease_until = NULL, completion_manifest = NULL, updated_at = $1 \
             WHERE upload_id = $2 AND completion_token = $3",
        )
        .bind(now)
        .bind(upload_id)
        .bind(completion_token)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if released.rows_affected() == 0 {
            return Ok(S3MultipartCompletionRelease::OwnershipLost(upload));
        }
        upload = lock_upload(&mut transaction, upload_id).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(S3MultipartCompletionRelease::Released(upload))
    }

    async fn abort_multipart_upload(
        &self,
        upload_id: &str,
        now: OffsetDateTime,
    ) -> Result<S3MultipartAbort, RepositoryError> {
        let now = postgres_time(now);
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let mut upload = lock_upload(&mut transaction, upload_id).await?;
        match upload.state {
            S3MultipartUploadState::Completing => {
                return Ok(S3MultipartAbort::Completing(upload));
            }
            S3MultipartUploadState::Completed => {
                return Ok(S3MultipartAbort::Completed(upload));
            }
            S3MultipartUploadState::Aborted => {
                let storage_keys = list_storage_keys(&mut transaction, upload_id).await?;
                transaction.commit().await.map_err(database_error)?;
                return Ok(S3MultipartAbort::AlreadyAborted {
                    upload,
                    storage_keys,
                });
            }
            S3MultipartUploadState::Pending => {}
        }
        abort_and_release_parts(&mut transaction, &upload, now).await?;
        upload = lock_upload(&mut transaction, upload_id).await?;
        let storage_keys = list_storage_keys(&mut transaction, upload_id).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(S3MultipartAbort::Aborted {
            upload,
            storage_keys,
        })
    }

    async fn expire_multipart_uploads(
        &self,
        now: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3MultipartExpiredUpload>, RepositoryError> {
        if !(1..=MAX_S3_MULTIPART_EXPIRY_LIMIT).contains(&limit) {
            return Err(RepositoryError::Invariant(
                "multipart expiry limit must be between 1 and 1000".into(),
            ));
        }
        let now = postgres_time(now);
        let limit = as_i64(limit as u64)?;
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT upload_id FROM s3_multipart_uploads WHERE \
             (expires_at <= $1 AND (state = 'pending' OR (state = 'completing' \
             AND completion_lease_until <= $1))) OR (state IN ('completed', 'aborted') \
             AND EXISTS(SELECT 1 FROM s3_multipart_parts \
             WHERE s3_multipart_parts.upload_id = s3_multipart_uploads.upload_id)) \
             ORDER BY CASE WHEN state IN ('pending', 'completing') THEN 0 ELSE 1 END, \
             expires_at, upload_id FOR UPDATE SKIP LOCKED LIMIT $2",
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;
        let mut expired = Vec::with_capacity(rows.len());
        for row in rows {
            let upload_id = row
                .try_get::<String, _>("upload_id")
                .map_err(database_error)?;
            let mut upload = lock_upload(&mut transaction, &upload_id).await?;
            if matches!(
                upload.state,
                S3MultipartUploadState::Pending | S3MultipartUploadState::Completing
            ) {
                abort_and_release_parts(&mut transaction, &upload, now).await?;
                upload = lock_upload(&mut transaction, &upload_id).await?;
            }
            let storage_keys = list_storage_keys(&mut transaction, &upload_id).await?;
            expired.push(S3MultipartExpiredUpload {
                upload,
                storage_keys,
            });
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(expired)
    }

    async fn clear_multipart_parts(&self, upload_id: &str) -> Result<usize, RepositoryError> {
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let upload = lock_upload(&mut transaction, upload_id).await?;
        if !matches!(
            upload.state,
            S3MultipartUploadState::Completed | S3MultipartUploadState::Aborted
        ) {
            return Err(RepositoryError::Conflict);
        }
        let deleted = sqlx::query("DELETE FROM s3_multipart_parts WHERE upload_id = $1")
            .bind(upload_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        let deleted = usize::try_from(deleted.rows_affected()).map_err(|_| {
            RepositoryError::Invariant("deleted multipart part count exceeds usize".into())
        })?;
        transaction.commit().await.map_err(database_error)?;
        Ok(deleted)
    }
}

