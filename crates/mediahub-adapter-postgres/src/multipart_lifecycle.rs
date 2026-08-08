// Multipart upload lifecycle operations.

#[async_trait]
impl S3MultipartRepository for PostgresRepository {
    async fn create_multipart_upload(
        &self,
        upload: NewS3MultipartUpload,
    ) -> Result<S3MultipartUpload, RepositoryError> {
        upload.validate()?;
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
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
        let active_limit = i64::try_from(MAX_S3_MULTIPART_ACTIVE_UPLOADS_PER_APPLICATION)
            .map_err(|_| {
                RepositoryError::Invariant("multipart active upload limit is too large".into())
            })?;
        if active_uploads >= active_limit {
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
        let created_at = postgres_time(upload.created_at);
        let expires_at = postgres_time(upload.expires_at);
        let row = sqlx::query(
            "INSERT INTO s3_multipart_uploads (upload_id, application_id, bucket_id, object_key, \
             content_type, user_metadata, storage_backend, state, expires_at, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, 'pending', $8, $9, $9) RETURNING *",
        )
        .bind(upload.upload_id)
        .bind(upload.application_id.as_uuid())
        .bind(upload.bucket_id.as_uuid())
        .bind(upload.object_key)
        .bind(upload.content_type)
        .bind(Json(upload.user_metadata))
        .bind(upload.storage_backend)
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
            return Ok(S3MultipartPartPut::NotPending(upload));
        }
        if upload.expires_at <= now {
            abort_upload_and_enqueue_cleanup(&mut transaction, &upload, None, now).await?;
            upload = lock_upload(&mut transaction, upload_id).await?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(S3MultipartPartPut::Expired(upload));
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
        let current_size = multipart_stored_bytes(&mut transaction, upload_id).await?;
        let new_total = current_size
            .checked_sub(previous_size)
            .and_then(|size| size.checked_add(as_i64(part.size).ok()?))
            .ok_or_else(|| RepositoryError::Invariant("multipart size overflow".into()))?;
        if new_total > as_i64(maximum_upload_size)? {
            return Err(RepositoryError::QuotaExceeded);
        }
        if let Some(replaced_key) = previous_storage_key
            .filter(|storage_key| storage_key != &part.storage_key)
        {
            enqueue_multipart_storage_keys(
                &mut transaction,
                &upload,
                None,
                [replaced_key],
                now,
            )
            .await?;
        }
        let row = sqlx::query(
            "INSERT INTO s3_multipart_parts (upload_id, part_number, size_bytes, sha256, md5, etag, \
             storage_key, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8) \
             ON CONFLICT (upload_id, part_number) DO UPDATE SET size_bytes = EXCLUDED.size_bytes, \
             sha256 = EXCLUDED.sha256, md5 = EXCLUDED.md5, etag = EXCLUDED.etag, \
             storage_key = EXCLUDED.storage_key, updated_at = EXCLUDED.updated_at RETURNING *",
        )
        .bind(upload_id)
        .bind(i32::from(part.part_number))
        .bind(as_i64(part.size)?)
        .bind(part.sha256)
        .bind(part.md5)
        .bind(part.etag)
        .bind(part.storage_key)
        .bind(now)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let stored = row_to_multipart_part(row)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(S3MultipartPartPut::Stored(stored))
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
            abort_upload_and_enqueue_cleanup(&mut transaction, &upload, None, now).await?;
            upload = lock_upload(&mut transaction, upload_id).await?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(S3MultipartCompletionClaim::Expired(upload));
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
        let total_size = selected_total_size(&selected)?;
        sqlx::query(
            "UPDATE s3_multipart_uploads SET state = 'completing', completion_token = $1, \
             completion_lease_until = $2, completion_manifest = $3, updated_at = $4 \
             WHERE upload_id = $5 AND state IN ('pending', 'completing')",
        )
        .bind(completion_token)
        .bind(lease_until)
        .bind(Json(manifest.to_vec()))
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

    async fn attach_multipart_upload_intent(
        &self,
        upload_id: &str,
        completion_token: &str,
        intent_id: UploadIntentId,
        now: OffsetDateTime,
    ) -> Result<S3MultipartUpload, RepositoryError> {
        let now = postgres_time(now);
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let upload = lock_upload(&mut transaction, upload_id).await?;
        validate_completion_owner(&mut transaction, &upload, completion_token, now).await?;
        let manifest = load_completion_manifest(&mut transaction, upload_id).await?;
        let parts = list_parts(&mut transaction, upload_id).await?;
        let selected = validate_manifest(&manifest, &parts).map_err(|error| {
            RepositoryError::Invariant(format!(
                "persisted multipart manifest is invalid: {error:?}"
            ))
        })?;
        let total_size = selected_total_size(&selected)?;
        let intent = lock_upload_intent(&mut transaction, intent_id).await?;
        validate_attachable_intent(&upload, &intent, completion_token, total_size, now)?;
        let changed = sqlx::query(
            "UPDATE s3_multipart_uploads SET upload_intent_id = $1, updated_at = $2 \
             WHERE upload_id = $3 AND state = 'completing' AND completion_token = $4 \
             AND completion_lease_until > $2 \
             AND (upload_intent_id IS NULL OR upload_intent_id = $1)",
        )
        .bind(intent_id.as_uuid())
        .bind(now)
        .bind(upload_id)
        .bind(completion_token)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if changed.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }
        let attached = lock_upload(&mut transaction, upload_id).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(attached)
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
        let owns_claim = sqlx::query_scalar::<_, bool>(
            "SELECT completion_token = $1 AND completion_lease_until > $2 \
             FROM s3_multipart_uploads WHERE upload_id = $3",
        )
        .bind(completion_token)
        .bind(now)
        .bind(upload_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if !owns_claim {
            return Ok(S3MultipartCompletionRelease::OwnershipLost(upload));
        }
        abort_attached_intent(&mut transaction, &upload, Some(completion_token), now).await?;
        let released = sqlx::query(
            "UPDATE s3_multipart_uploads SET state = 'pending', completion_token = NULL, \
             completion_lease_until = NULL, completion_manifest = NULL, upload_intent_id = NULL, \
             updated_at = $1 WHERE upload_id = $2 AND completion_token = $3 \
             AND completion_lease_until > $1",
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
                return Ok(S3MultipartAbort::AlreadyAborted(upload));
            }
            S3MultipartUploadState::Pending => {}
        }
        abort_upload_and_enqueue_cleanup(&mut transaction, &upload, None, now).await?;
        upload = lock_upload(&mut transaction, upload_id).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(S3MultipartAbort::Aborted(upload))
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
            "SELECT upload_id FROM s3_multipart_uploads WHERE expires_at <= $1 \
             AND (state = 'pending' OR (state = 'completing' AND completion_lease_until <= $1)) \
             ORDER BY expires_at, upload_id FOR UPDATE SKIP LOCKED LIMIT $2",
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
            let upload = lock_upload(&mut transaction, &upload_id).await?;
            abort_upload_and_enqueue_cleanup(&mut transaction, &upload, None, now).await?;
            let upload = lock_upload(&mut transaction, &upload_id).await?;
            expired.push(S3MultipartExpiredUpload { upload });
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(expired)
    }
}