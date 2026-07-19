// Bucket persistence operations.

impl PostgresRepository {
    pub async fn create_bucket(&self, bucket: &Bucket) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        insert_bucket(&mut transaction, bucket).await?;
        transaction.commit().await.map_err(database_error)
    }

    pub async fn find_bucket_by_name(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<Bucket>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM buckets WHERE application_id = $1 AND name = $2")
            .bind(application_id.as_uuid())
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_bucket).transpose()
    }

    pub async fn list_buckets(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<Bucket>, RepositoryError> {
        let rows = sqlx::query("SELECT * FROM buckets WHERE application_id = $1 ORDER BY name")
            .bind(application_id.as_uuid())
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        rows.into_iter().map(row_to_bucket).collect()
    }

    pub async fn update_bucket_policy(
        &self,
        application_id: ApplicationId,
        name: &str,
        policy: &BucketPolicy,
        updated_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        let allowed_mime_types = policy.allowed_mime_types().collect::<Vec<_>>();
        let result = sqlx::query(
            "UPDATE buckets SET visibility = $1, default_ttl_seconds = $2, \
             max_object_bytes = $3, allowed_mime_types = $4, lifecycle_policy = $5, \
             updated_at = $6 WHERE application_id = $7 AND name = $8",
        )
        .bind(visibility_name(policy.visibility()))
        .bind(policy.default_ttl_seconds().map(as_i64).transpose()?)
        .bind(policy.max_object_size().map(as_i64).transpose()?)
        .bind(Json(allowed_mime_types))
        .bind(Json(policy.lifecycle_rules()))
        .bind(postgres_time(updated_at))
        .bind(application_id.as_uuid())
        .bind(name)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn delete_empty_bucket(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<bool, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query(
            "SELECT id FROM buckets WHERE application_id = $1 AND name = $2 FOR UPDATE",
        )
        .bind(application_id.as_uuid())
        .bind(name)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(false);
        };
        let bucket_id: uuid::Uuid = row.try_get("id").map_err(database_error)?;
        let has_primary_contents = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM media WHERE bucket_id = $1) \
             OR EXISTS(SELECT 1 FROM upload_sessions WHERE bucket_id = $1)",
        )
        .bind(bucket_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if has_primary_contents {
            return Err(RepositoryError::Conflict);
        }
        let has_multipart_contents = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM s3_multipart_uploads upload \
             WHERE upload.bucket_id = $1 AND (upload.state IN ('pending', 'completing') \
             OR EXISTS(SELECT 1 FROM s3_multipart_parts part \
             WHERE part.upload_id = upload.upload_id)))",
        )
        .bind(bucket_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if has_multipart_contents {
            return Err(RepositoryError::Conflict);
        }
        sqlx::query(
            "DELETE FROM s3_multipart_uploads WHERE bucket_id = $1 \
             AND state IN ('completed', 'aborted') AND NOT EXISTS \
             (SELECT 1 FROM s3_multipart_parts \
             WHERE s3_multipart_parts.upload_id = s3_multipart_uploads.upload_id)",
        )
        .bind(bucket_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        let result = sqlx::query("DELETE FROM buckets WHERE id = $1 AND application_id = $2")
            .bind(bucket_id)
            .bind(application_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }
}
