// Access-key repository implementation.

#[async_trait]
impl AccessKeyRepository for PostgresRepository {
    async fn create_access_key(&self, access_key: &NewAccessKey) -> Result<(), RepositoryError> {
        sqlx::query(
            "INSERT INTO access_keys \
             (id, application_id, access_key_id, secret_ciphertext, secret_key_version, \
              secret_last_four, name, permissions, expires_at, revoked_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, $10)",
        )
        .bind(parse_uuid(&access_key.id, "access key id")?)
        .bind(access_key.application_id.as_uuid())
        .bind(&access_key.access_key_id)
        .bind(&access_key.secret_ciphertext)
        .bind(as_i32(access_key.secret_key_version, "access key version")?)
        .bind(&access_key.secret_last_four)
        .bind(&access_key.name)
        .bind(Json(&access_key.permissions))
        .bind(access_key.expires_at.map(postgres_time))
        .bind(postgres_time(access_key.created_at))
        .execute(&self.pool)
        .await
        .map_err(control_write_error)?;
        Ok(())
    }

    async fn list_access_keys(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<AccessKeyRecord>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT * FROM access_keys WHERE application_id = $1 ORDER BY created_at DESC, id DESC",
        )
        .bind(application_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_access_key).collect()
    }

    async fn find_active_access_key(
        &self,
        access_key_id: &str,
        now: OffsetDateTime,
    ) -> Result<Option<AccessKeyRecord>, RepositoryError> {
        let row = sqlx::query(
            "SELECT * FROM access_keys WHERE access_key_id = $1 AND revoked_at IS NULL \
             AND (expires_at IS NULL OR expires_at > $2)",
        )
        .bind(access_key_id)
        .bind(postgres_time(now))
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_access_key).transpose()
    }

    async fn find_access_key(
        &self,
        access_key_id: &str,
    ) -> Result<Option<AccessKeyRecord>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM access_keys WHERE access_key_id = $1")
            .bind(access_key_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_access_key).transpose()
    }

    async fn update_access_key(
        &self,
        access_key_id: &str,
        application_id: ApplicationId,
        name: &str,
        permissions: &[String],
        expires_at: Option<OffsetDateTime>,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "UPDATE access_keys SET name = $1, permissions = $2, expires_at = $3 \
             WHERE access_key_id = $4 AND application_id = $5 AND revoked_at IS NULL",
        )
        .bind(name)
        .bind(Json(permissions))
        .bind(expires_at.map(postgres_time))
        .bind(access_key_id)
        .bind(application_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn revoke_access_key(
        &self,
        access_key_id: &str,
        application_id: ApplicationId,
        revoked_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        let result = sqlx::query(
            "UPDATE access_keys SET revoked_at = COALESCE(revoked_at, $1) \
             WHERE access_key_id = $2 AND application_id = $3",
        )
        .bind(postgres_time(revoked_at))
        .bind(access_key_id)
        .bind(application_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn record_replay_nonce(
        &self,
        access_key_id: &str,
        nonce: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        sqlx::query("DELETE FROM replay_nonces WHERE expires_at <= $1")
            .bind(postgres_time(now))
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        sqlx::query(
            "INSERT INTO replay_nonces (access_key_id, nonce, expires_at, created_at) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(access_key_id)
        .bind(nonce)
        .bind(postgres_time(expires_at))
        .bind(postgres_time(now))
        .execute(&mut *transaction)
        .await
        .map_err(control_write_error)?;
        transaction.commit().await.map_err(database_error)
    }
}

