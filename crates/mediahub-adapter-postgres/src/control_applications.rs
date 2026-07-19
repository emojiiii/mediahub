// Application repository implementation.

#[async_trait]
impl ApplicationRepository for PostgresRepository {
    async fn create_application(
        &self,
        application_id: ApplicationId,
        user_id: UserId,
        name: &str,
        app_id: &str,
        quota_bytes: u64,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let now = postgres_time(now);
        sqlx::query(
            "INSERT INTO applications \
             (id, user_id, name, app_id, quota_bytes, used_bytes, reserved_bytes, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, 0, 0, $6, $6)",
        )
        .bind(application_id.as_uuid())
        .bind(user_id.as_uuid())
        .bind(name)
        .bind(app_id)
        .bind(as_i64(quota_bytes)?)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(control_write_error)?;
        Ok(())
    }

    async fn default_application_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Option<ApplicationSummary>, RepositoryError> {
        find_application(
            &self.pool,
            "SELECT id, name, app_id, quota_bytes, used_bytes, reserved_bytes \
             FROM applications WHERE user_id = $1 ORDER BY created_at, id LIMIT 1",
            user_id.as_uuid(),
        )
        .await
    }

    async fn find_application_by_id(
        &self,
        application_id: ApplicationId,
    ) -> Result<Option<ApplicationSummary>, RepositoryError> {
        let row = sqlx::query(
            "SELECT id, name, app_id, quota_bytes, used_bytes, reserved_bytes \
             FROM applications WHERE id = $1",
        )
        .bind(application_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_application).transpose()
    }

    async fn find_application_by_app_id(
        &self,
        app_id: &str,
    ) -> Result<Option<ApplicationSummary>, RepositoryError> {
        let row = sqlx::query(
            "SELECT id, name, app_id, quota_bytes, used_bytes, reserved_bytes \
             FROM applications WHERE app_id = $1",
        )
        .bind(app_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_application).transpose()
    }

    async fn list_applications_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ApplicationSummary>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT id, name, app_id, quota_bytes, used_bytes, reserved_bytes \
             FROM applications WHERE user_id = $1 ORDER BY created_at, id",
        )
        .bind(user_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_application).collect()
    }

    async fn application_for_user_by_app_id(
        &self,
        user_id: UserId,
        app_id: &str,
    ) -> Result<Option<ApplicationSummary>, RepositoryError> {
        let row = sqlx::query(
            "SELECT id, name, app_id, quota_bytes, used_bytes, reserved_bytes \
             FROM applications WHERE user_id = $1 AND app_id = $2",
        )
        .bind(user_id.as_uuid())
        .bind(app_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_application).transpose()
    }

    async fn application_for_user_by_id(
        &self,
        user_id: UserId,
        application_id: ApplicationId,
    ) -> Result<Option<ApplicationSummary>, RepositoryError> {
        let row = sqlx::query(
            "SELECT id, name, app_id, quota_bytes, used_bytes, reserved_bytes \
             FROM applications WHERE user_id = $1 AND id = $2",
        )
        .bind(user_id.as_uuid())
        .bind(application_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_application).transpose()
    }

    async fn update_application_name_for_user(
        &self,
        user_id: UserId,
        app_id: &str,
        name: &str,
        updated_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "UPDATE applications SET name = $1, updated_at = $2 \
             WHERE user_id = $3 AND app_id = $4",
        )
        .bind(name)
        .bind(postgres_time(updated_at))
        .bind(user_id.as_uuid())
        .bind(app_id)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn delete_application_for_user(
        &self,
        user_id: UserId,
        app_id: &str,
    ) -> Result<bool, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query(
            "SELECT id, \
                    EXISTS(SELECT 1 FROM buckets WHERE application_id = applications.id) AS has_buckets, \
                    EXISTS(SELECT 1 FROM media WHERE application_id = applications.id) AS has_media \
             FROM applications WHERE user_id = $1 AND app_id = $2 FOR UPDATE",
        )
        .bind(user_id.as_uuid())
        .bind(app_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(false);
        };
        if row
            .try_get::<bool, _>("has_buckets")
            .map_err(database_error)?
            || row
                .try_get::<bool, _>("has_media")
                .map_err(database_error)?
        {
            return Err(RepositoryError::Conflict);
        }
        let application_id = row.try_get::<Uuid, _>("id").map_err(database_error)?;
        sqlx::query(
            "DELETE FROM replay_nonces WHERE access_key_id IN \
             (SELECT access_key_id FROM access_keys WHERE application_id = $1)",
        )
        .bind(application_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        sqlx::query("DELETE FROM access_keys WHERE application_id = $1")
            .bind(application_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        sqlx::query("DELETE FROM audit_logs WHERE application_id = $1")
            .bind(application_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        let result = sqlx::query("DELETE FROM applications WHERE id = $1 AND user_id = $2")
            .bind(application_id)
            .bind(user_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(control_write_error)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }
}

