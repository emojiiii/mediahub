//! PostgreSQL implementation of MediaHub's runtime-independent repository ports.

mod administration;
mod async_job;
mod codec;
mod control_plane;
mod data_plane;
mod idempotency;
mod media;
mod outbox;
mod s3_multipart;
mod upload_session;
mod variant;
mod webhook;

use std::time::Duration;

use mediahub_app::RepositoryError;
use sqlx::{PgPool, postgres::PgPoolOptions};

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

#[derive(Clone, Debug)]
pub struct PostgresRepository {
    pool: PgPool,
}

impl PostgresRepository {
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn connect(database_url: &str) -> Result<Self, RepositoryError> {
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .acquire_timeout(Duration::from_secs(10))
            .connect(database_url)
            .await
            .map_err(codec::database_error)?;
        Ok(Self::new(pool))
    }

    pub async fn migrate(&self) -> Result<(), RepositoryError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|error| RepositoryError::Unavailable(error.to_string()))
    }

    #[must_use]
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn migration_uses_postgres_concurrency_primitives() {
        let repository = include_str!("../migrations/0001_repository_profile.sql");
        let control_plane = include_str!("../migrations/0002_control_plane.sql");
        let runtime = include_str!("../migrations/0003_control_plane_runtime.sql");
        let variant_formats = include_str!("../migrations/0004_remove_avif_variants.sql");
        let system_settings = include_str!("../migrations/0005_system_settings.sql");
        let s3_multipart = include_str!("../migrations/0006_s3_multipart.sql");
        let ordinary_upload_fencing =
            include_str!("../migrations/0010_ordinary_upload_fencing.sql");
        assert!(repository.contains("JSONB"));
        assert!(repository.contains("TIMESTAMPTZ"));
        assert!(repository.contains("async_jobs_claimable_idx"));
        assert!(repository.contains("variants_cleanup_idx"));
        assert!(repository.contains("last_response_status"));
        assert!(repository.contains("webhook_deliveries_history_idx"));
        assert!(control_plane.contains("CREATE TABLE sessions"));
        assert!(control_plane.contains("CREATE TABLE one_time_tokens"));
        assert!(control_plane.contains("CREATE TABLE access_keys"));
        assert!(control_plane.contains("CREATE TABLE replay_nonces"));
        assert!(control_plane.contains("CREATE TABLE idempotency_keys"));
        assert!(control_plane.contains("CREATE TABLE audit_logs"));
        assert!(control_plane.contains("CREATE TABLE deployment_bootstrap"));
        assert!(control_plane.contains("users_role_status_created_idx"));
        assert!(runtime.contains("ALTER COLUMN id TYPE TEXT"));
        assert!(runtime.contains("history_id BIGINT GENERATED ALWAYS AS IDENTITY"));
        assert!(variant_formats.contains("CHECK (format IN ('jpeg', 'png', 'webp')) NOT VALID"));
        assert!(variant_formats.contains("VALIDATE CONSTRAINT variants_format_check"));
        assert!(system_settings.contains("CREATE TABLE system_settings"));
        assert!(
            system_settings.contains("download_bytes_per_second BETWEEN 1048576 AND 1073741824")
        );
        assert!(s3_multipart.contains("CREATE TABLE s3_multipart_uploads"));
        assert!(
            s3_multipart.contains("state IN ('pending', 'completing', 'completed', 'aborted')")
        );
        assert!(s3_multipart.contains("part_number BETWEEN 1 AND 10000"));
        assert!(s3_multipart.contains("completion_lease_until"));
        assert!(s3_multipart.contains("s3_multipart_expiry_idx"));
        assert!(ordinary_upload_fencing.contains("upload_lease_token"));
        assert!(ordinary_upload_fencing.contains("mediahub_normalize_upload_lease"));
        assert!(ordinary_upload_fencing.contains("media_upload_reconciliation_idx"));
    }

    #[test]
    fn data_plane_sql_keeps_native_types_locks_and_atomic_boundaries() {
        let media = [
            include_str!("media.rs"),
            include_str!("media_buckets.rs"),
            include_str!("media_queries.rs"),
            include_str!("media_mutations.rs"),
            include_str!("media_support.rs"),
        ]
        .concat();
        let idempotency = include_str!("idempotency.rs");
        assert!(media.contains("FOR UPDATE"));
        assert!(media.contains("QueryBuilder::<Postgres>"));
        assert!(media.contains("jsonb_array_length(lifecycle_policy)"));
        assert!(media.contains("user_metadata = '{}'::jsonb"));
        assert!(media.contains("DELETE FROM variants WHERE media_id = $1"));
        assert!(idempotency.contains("ON CONFLICT (application_id, operation_scope"));
        assert!(idempotency.contains("create_in_transaction(&mut transaction, session)"));
        assert!(idempotency.contains("insert_bucket(&mut transaction, bucket)"));
        assert!(idempotency.contains("status = 'completed'"));
        let s3_multipart = [
            include_str!("s3_multipart.rs"),
            include_str!("multipart_lifecycle.rs"),
            include_str!("multipart_helpers.rs"),
        ]
        .concat();
        assert!(s3_multipart.contains("FOR UPDATE SKIP LOCKED LIMIT $2"));
    }
}
