// PostgreSQL control-plane repository wiring.

use async_trait::async_trait;
use mediahub_app::{
    AccessKeyRecord, AccessKeyRepository, ApplicationRepository, ApplicationSummary,
    AuthRepository, NewAccessKey, OneTimeTokenPurpose, QuotaSnapshot, RepositoryError,
    SessionRecord, UserAccount,
};
use mediahub_core::{ApplicationId, Bucket, OffsetDateTime, UserId};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow, types::Json};
use uuid::Uuid;

use crate::{
    PostgresRepository,
    codec::{as_i64, as_u32, database_error, postgres_time},
};

include!("control_auth.rs");
include!("control_applications.rs");
include!("control_access_keys.rs");
include!("control_helpers.rs");

impl PostgresRepository {
    /// Creates the registration aggregate in one transaction. Keeping this
    /// boundary in the repository prevents a partially-created account when
    /// any of the application, bucket, or verification-token inserts fails.
    #[allow(clippy::too_many_arguments)]
    pub async fn register_user(
        &self,
        user_id: UserId,
        email_normalized: &str,
        password_hash: &str,
        application_id: ApplicationId,
        application_name: &str,
        app_id: &str,
        quota_bytes: u64,
        bucket: &Bucket,
        token_hash: &str,
        token_expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        if bucket.application_id() != application_id {
            return Err(RepositoryError::Invariant(
                "registration bucket does not belong to the new application".into(),
            ));
        }
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let now = postgres_time(now);
        sqlx::query(
            "INSERT INTO users (id, email_normalized, password_hash, status, created_at, updated_at) \
             VALUES ($1, $2, $3, 'pending_verification', $4, $4)",
        )
        .bind(user_id.as_uuid())
        .bind(email_normalized)
        .bind(password_hash)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(control_write_error)?;
        sqlx::query(
            "INSERT INTO applications \
             (id, user_id, name, app_id, quota_bytes, used_bytes, reserved_bytes, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, 0, 0, $6, $6)",
        )
        .bind(application_id.as_uuid())
        .bind(user_id.as_uuid())
        .bind(application_name)
        .bind(app_id)
        .bind(as_i64(quota_bytes)?)
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(control_write_error)?;
        crate::media::insert_bucket(&mut transaction, bucket).await?;
        sqlx::query(
            "INSERT INTO one_time_tokens \
             (id, user_id, purpose, token_hash, expires_at, consumed_at, created_at) \
             VALUES ($1, $2, 'verify_email', $3, $4, NULL, $5)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id.as_uuid())
        .bind(token_hash)
        .bind(postgres_time(token_expires_at))
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(control_write_error)?;
        transaction.commit().await.map_err(database_error)
    }
}
