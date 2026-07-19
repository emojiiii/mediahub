// Authentication and session repository implementation.

#[async_trait]
impl AuthRepository for PostgresRepository {
    async fn create_user(
        &self,
        user_id: UserId,
        email_normalized: &str,
        password_hash: &str,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let now = postgres_time(now);
        sqlx::query(
            "INSERT INTO users \
             (id, email_normalized, password_hash, status, created_at, updated_at) \
             VALUES ($1, $2, $3, 'pending_verification', $4, $4)",
        )
        .bind(user_id.as_uuid())
        .bind(email_normalized)
        .bind(password_hash)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(control_write_error)?;
        Ok(())
    }

    async fn find_user_by_email(
        &self,
        email_normalized: &str,
    ) -> Result<Option<UserAccount>, RepositoryError> {
        let row = sqlx::query(
            "SELECT id, email_normalized, password_hash, email_verified_at, status, system_role, \
                    last_login_at, created_at, updated_at \
             FROM users WHERE email_normalized = $1",
        )
        .bind(email_normalized)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_user).transpose()
    }

    async fn create_session(
        &self,
        user_id: UserId,
        token_hash: &str,
        csrf_token_hash: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        <Self as AuthRepository>::create_session_with_context(
            self,
            user_id,
            token_hash,
            csrf_token_hash,
            expires_at,
            now,
            None,
            None,
        )
        .await
    }

    async fn create_session_with_context(
        &self,
        user_id: UserId,
        token_hash: &str,
        csrf_token_hash: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
        created_ip: Option<&str>,
        user_agent_summary: Option<&str>,
    ) -> Result<(), RepositoryError> {
        let now = postgres_time(now);
        sqlx::query(
            "INSERT INTO sessions \
             (id, user_id, token_hash, csrf_token_hash, expires_at, last_seen_at, created_ip, \
              last_seen_ip, user_agent_summary, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $7, $8, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id.as_uuid())
        .bind(token_hash)
        .bind(csrf_token_hash)
        .bind(postgres_time(expires_at))
        .bind(now)
        .bind(created_ip)
        .bind(user_agent_summary)
        .execute(&self.pool)
        .await
        .map_err(control_write_error)?;
        Ok(())
    }

    async fn find_user_by_session_hash(
        &self,
        token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<Option<UserAccount>, RepositoryError> {
        let row = sqlx::query(
            "SELECT u.id, u.email_normalized, u.password_hash, u.email_verified_at, u.status, \
                    u.system_role, u.last_login_at, u.created_at, u.updated_at \
             FROM sessions s INNER JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = $1 AND s.revoked_at IS NULL AND s.expires_at > $2 \
               AND u.status = 'active'",
        )
        .bind(token_hash)
        .bind(postgres_time(now))
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_user).transpose()
    }

    async fn record_user_login(
        &self,
        user_id: UserId,
        logged_in_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let logged_in_at = postgres_time(logged_in_at);
        let result = sqlx::query(
            "UPDATE users SET last_login_at = $1, updated_at = $1 \
             WHERE id = $2 AND status = 'active'",
        )
        .bind(logged_in_at)
        .bind(user_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        affected_one(result.rows_affected())
    }

    async fn valid_session_csrf(
        &self,
        token_hash: &str,
        csrf_token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        let valid = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM sessions \
             WHERE token_hash = $1 AND csrf_token_hash = $2 \
               AND revoked_at IS NULL AND expires_at > $3)",
        )
        .bind(token_hash)
        .bind(csrf_token_hash)
        .bind(postgres_time(now))
        .fetch_one(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(valid)
    }

    async fn delete_session_by_hash(&self, token_hash: &str) -> Result<(), RepositoryError> {
        sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(token_hash)
            .execute(&self.pool)
            .await
            .map_err(database_error)?;
        Ok(())
    }

    async fn create_one_time_token(
        &self,
        user_id: UserId,
        purpose: OneTimeTokenPurpose,
        token_hash: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let now = postgres_time(now);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        sqlx::query(
            "UPDATE one_time_tokens SET consumed_at = $1 \
             WHERE user_id = $2 AND purpose = $3 AND consumed_at IS NULL",
        )
        .bind(now)
        .bind(user_id.as_uuid())
        .bind(purpose.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        sqlx::query(
            "INSERT INTO one_time_tokens \
             (id, user_id, purpose, token_hash, expires_at, consumed_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, NULL, $6)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id.as_uuid())
        .bind(purpose.as_str())
        .bind(token_hash)
        .bind(postgres_time(expires_at))
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(control_write_error)?;
        transaction.commit().await.map_err(database_error)
    }

    async fn consume_email_verification_token(
        &self,
        token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        let now = postgres_time(now);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let user_id = consume_one_time_token(
            &mut transaction,
            token_hash,
            OneTimeTokenPurpose::VerifyEmail,
            now,
        )
        .await?;
        let Some(user_id) = user_id else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(false);
        };
        let result = sqlx::query(
            "UPDATE users SET email_verified_at = COALESCE(email_verified_at, $1), \
                    status = CASE WHEN status = 'pending_verification' THEN 'active' ELSE status END, \
                    updated_at = $1 \
             WHERE id = $2 AND status != 'deleted'",
        )
        .bind(now)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Invariant(
                "verification token references an unavailable user".into(),
            ));
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(true)
    }

    async fn consume_password_reset_token(
        &self,
        token_hash: &str,
        password_hash: &str,
        now: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        let now = postgres_time(now);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let user_id = consume_one_time_token(
            &mut transaction,
            token_hash,
            OneTimeTokenPurpose::ResetPassword,
            now,
        )
        .await?;
        let Some(user_id) = user_id else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(false);
        };
        let result = sqlx::query(
            "UPDATE users SET password_hash = $1, updated_at = $2 \
             WHERE id = $3 AND status != 'deleted'",
        )
        .bind(password_hash)
        .bind(now)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Invariant(
                "password reset token references an unavailable user".into(),
            ));
        }
        sqlx::query(
            "UPDATE sessions SET revoked_at = $1 WHERE user_id = $2 AND revoked_at IS NULL",
        )
        .bind(now)
        .bind(user_id)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(true)
    }

    async fn list_active_sessions(
        &self,
        user_id: UserId,
        current_token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<Vec<SessionRecord>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT id, token_hash, expires_at, last_seen_at, created_ip, last_seen_ip, \
                    user_agent_summary, created_at \
             FROM sessions WHERE user_id = $1 AND revoked_at IS NULL AND expires_at > $2 \
             ORDER BY created_at DESC, id DESC",
        )
        .bind(user_id.as_uuid())
        .bind(postgres_time(now))
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter()
            .map(|row| row_to_session(row, current_token_hash))
            .collect()
    }

    async fn revoke_session(
        &self,
        user_id: UserId,
        session_id: &str,
        current_token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<Option<bool>, RepositoryError> {
        let Ok(session_id) = Uuid::parse_str(session_id) else {
            return Ok(None);
        };
        let row = sqlx::query(
            "UPDATE sessions SET revoked_at = $1 \
             WHERE id = $2 AND user_id = $3 AND revoked_at IS NULL RETURNING token_hash",
        )
        .bind(postgres_time(now))
        .bind(session_id)
        .bind(user_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(|row| {
            row.try_get::<String, _>("token_hash")
                .map(|hash| hash == current_token_hash)
                .map_err(database_error)
        })
        .transpose()
    }

    async fn revoke_all_sessions(
        &self,
        user_id: UserId,
        now: OffsetDateTime,
    ) -> Result<u64, RepositoryError> {
        let result = sqlx::query(
            "UPDATE sessions SET revoked_at = $1 WHERE user_id = $2 AND revoked_at IS NULL",
        )
        .bind(postgres_time(now))
        .bind(user_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected())
    }
}

