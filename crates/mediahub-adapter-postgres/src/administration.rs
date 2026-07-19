use async_trait::async_trait;
use mediahub_app::{
    AdminApplicationSummary, AdminBootstrapOutcome, AdminJobSummary, AdminMetricsSnapshot,
    AdminRepository, AdminStorageSummary, AdminSystemSettings, AdminUserSummary, AuditEvent,
    AuditRepository, QuotaSnapshot, RepositoryError, SecretKeyVersionRepository,
};
use mediahub_core::{ApplicationId, AsyncJobId, OffsetDateTime, UserId};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow, types::Json};
use uuid::Uuid;

use crate::{
    PostgresRepository,
    codec::{as_i64, as_u32, as_u64, database_error, postgres_time},
};

const ADMIN_ADVISORY_LOCK: i64 = 557_019_177_321;

#[async_trait]
impl AuditRepository for PostgresRepository {
    async fn record_audit(&self, event: &AuditEvent) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        insert_audit(&mut transaction, event).await?;
        transaction.commit().await.map_err(database_error)
    }

    async fn list_audit(
        &self,
        application_id: ApplicationId,
        limit: usize,
    ) -> Result<Vec<AuditEvent>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT * FROM audit_logs WHERE application_id = $1 \
             ORDER BY created_at DESC, id DESC LIMIT $2",
        )
        .bind(application_id.as_uuid())
        .bind(as_i64(limit as u64)?)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_audit).collect()
    }
}

#[async_trait]
impl AdminRepository for PostgresRepository {
    async fn bootstrap_admin(
        &self,
        email_normalized: &str,
        completed_at: OffsetDateTime,
    ) -> Result<AdminBootstrapOutcome, RepositoryError> {
        let completed_at = postgres_time(completed_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        advisory_admin_lock(&mut transaction).await?;
        if sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM deployment_bootstrap \
             WHERE bootstrap_key = 'initial_admin')",
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?
        {
            transaction.commit().await.map_err(database_error)?;
            return Ok(AdminBootstrapOutcome::AlreadyCompleted);
        }
        if sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users WHERE system_role = 'admin'")
            .fetch_one(&mut *transaction)
            .await
            .map_err(database_error)?
            != 0
        {
            return Err(RepositoryError::Conflict);
        }
        let row = sqlx::query(
            "SELECT u.id, a.id AS application_id FROM users AS u \
             JOIN applications AS a ON a.user_id = u.id \
             WHERE u.email_normalized = $1 AND u.status = 'active' \
               AND u.email_verified_at IS NOT NULL \
             ORDER BY a.created_at, a.id LIMIT 1 FOR UPDATE OF u",
        )
        .bind(email_normalized)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
        let user_id = UserId::from_uuid(row.try_get("id").map_err(database_error)?);
        let application_id =
            ApplicationId::from_uuid(row.try_get("application_id").map_err(database_error)?);
        let result = sqlx::query(
            "UPDATE users SET system_role = 'admin', updated_at = $1 \
             WHERE id = $2 AND system_role = 'user' AND status = 'active'",
        )
        .bind(completed_at)
        .bind(user_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }
        insert_audit(
            &mut transaction,
            &AuditEvent {
                id: format!("admin-bootstrap:{}", Uuid::now_v7()),
                application_id,
                actor_type: "system".into(),
                actor_id: "deployment_bootstrap".into(),
                action: "user.system_role_bootstrapped".into(),
                target_type: "user".into(),
                target_id: user_id.to_string(),
                request_id: "deployment_bootstrap".into(),
                summary: serde_json::json!({ "system_role": "admin" }),
                created_at: completed_at,
            },
        )
        .await?;
        sqlx::query(
            "INSERT INTO deployment_bootstrap (bootstrap_key, user_id, completed_at) \
             VALUES ('initial_admin', $1, $2)",
        )
        .bind(user_id.as_uuid())
        .bind(completed_at)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(AdminBootstrapOutcome::Completed(user_id))
    }

    async fn list_admin_users(
        &self,
        limit: usize,
    ) -> Result<Vec<AdminUserSummary>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT id, email_normalized, email_verified_at, status, system_role, \
                    last_login_at, created_at, updated_at \
             FROM users ORDER BY created_at DESC, id DESC LIMIT $1",
        )
        .bind(as_i64(limit as u64)?)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_admin_user).collect()
    }

    async fn transition_user_status(
        &self,
        actor: UserId,
        target: UserId,
        requested_status: &str,
        request_id: &str,
        changed_at: OffsetDateTime,
    ) -> Result<AdminUserSummary, RepositoryError> {
        if !matches!(requested_status, "active" | "suspended") {
            return Err(RepositoryError::Invariant(
                "admin user status must be active or suspended".into(),
            ));
        }
        let changed_at = postgres_time(changed_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        advisory_admin_lock(&mut transaction).await?;
        let row = sqlx::query(
            "SELECT u.id, u.email_normalized, u.email_verified_at, u.status, u.system_role, \
                    u.last_login_at, u.created_at, u.updated_at, a.id AS application_id \
             FROM users AS u \
             JOIN LATERAL (SELECT id FROM applications WHERE user_id = u.id \
                           ORDER BY created_at, id LIMIT 1) AS a ON TRUE \
             WHERE u.id = $1 FOR UPDATE OF u",
        )
        .bind(target.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
        let current_status: String = row.try_get("status").map_err(database_error)?;
        if current_status == requested_status {
            let user = row_to_admin_user(row)?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(user);
        }
        let verified_at: Option<OffsetDateTime> =
            row.try_get("email_verified_at").map_err(database_error)?;
        if !matches!(
            (current_status.as_str(), requested_status),
            ("active", "suspended") | ("suspended", "active")
        ) || (requested_status == "active" && verified_at.is_none())
        {
            return Err(RepositoryError::Conflict);
        }
        let system_role: String = row.try_get("system_role").map_err(database_error)?;
        if system_role == "admin"
            && requested_status == "suspended"
            && sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM users WHERE system_role = 'admin' AND status = 'active'",
            )
            .fetch_one(&mut *transaction)
            .await
            .map_err(database_error)?
                <= 1
        {
            return Err(RepositoryError::Conflict);
        }
        let application_id =
            ApplicationId::from_uuid(row.try_get("application_id").map_err(database_error)?);
        let result = sqlx::query(
            "UPDATE users SET status = $1, updated_at = $2 \
             WHERE id = $3 AND status = $4",
        )
        .bind(requested_status)
        .bind(changed_at)
        .bind(target.as_uuid())
        .bind(&current_status)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }
        if requested_status == "suspended" {
            sqlx::query(
                "UPDATE sessions SET revoked_at = $1 \
                 WHERE user_id = $2 AND revoked_at IS NULL",
            )
            .bind(changed_at)
            .bind(target.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        }
        insert_audit(
            &mut transaction,
            &AuditEvent {
                id: format!("admin-user-status:{}", Uuid::now_v7()),
                application_id,
                actor_type: "user".into(),
                actor_id: actor.to_string(),
                action: "user.status_changed".into(),
                target_type: "user".into(),
                target_id: target.to_string(),
                request_id: request_id.to_owned(),
                summary: serde_json::json!({
                    "previous_status": current_status,
                    "status": requested_status,
                }),
                created_at: changed_at,
            },
        )
        .await?;
        let row = sqlx::query(
            "SELECT id, email_normalized, email_verified_at, status, system_role, \
                    last_login_at, created_at, updated_at FROM users WHERE id = $1",
        )
        .bind(target.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let user = row_to_admin_user(row)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(user)
    }

    async fn list_admin_applications(
        &self,
        limit: usize,
    ) -> Result<Vec<AdminApplicationSummary>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT id, user_id, name, app_id, quota_bytes, used_bytes, reserved_bytes, \
                    created_at, updated_at FROM applications \
             ORDER BY created_at DESC, id DESC LIMIT $1",
        )
        .bind(as_i64(limit as u64)?)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_admin_application).collect()
    }

    async fn update_application_quota(
        &self,
        actor: UserId,
        application_id: ApplicationId,
        quota_bytes: u64,
        request_id: &str,
        changed_at: OffsetDateTime,
    ) -> Result<AdminApplicationSummary, RepositoryError> {
        let changed_at = postgres_time(changed_at);
        let quota_bytes_i64 = as_i64(quota_bytes)?;
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query(
            "SELECT id, user_id, name, app_id, quota_bytes, used_bytes, reserved_bytes, \
                    created_at, updated_at FROM applications WHERE id = $1 FOR UPDATE",
        )
        .bind(application_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
        let previous_quota = row_u64(&row, "quota_bytes")?;
        let used_bytes = row_u64(&row, "used_bytes")?;
        let reserved_bytes = row_u64(&row, "reserved_bytes")?;
        if quota_bytes < used_bytes.saturating_add(reserved_bytes) {
            return Err(RepositoryError::Conflict);
        }
        if quota_bytes == previous_quota {
            let application = row_to_admin_application(row)?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(application);
        }
        sqlx::query("UPDATE applications SET quota_bytes = $1, updated_at = $2 WHERE id = $3")
            .bind(quota_bytes_i64)
            .bind(changed_at)
            .bind(application_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        insert_audit(
            &mut transaction,
            &AuditEvent {
                id: format!("admin-application-quota:{}", Uuid::now_v7()),
                application_id,
                actor_type: "user".into(),
                actor_id: actor.to_string(),
                action: "application.quota_changed".into(),
                target_type: "application".into(),
                target_id: application_id.to_string(),
                request_id: request_id.to_owned(),
                summary: serde_json::json!({
                    "previous_quota_bytes": previous_quota,
                    "quota_bytes": quota_bytes,
                    "used_bytes": used_bytes,
                    "reserved_bytes": reserved_bytes,
                }),
                created_at: changed_at,
            },
        )
        .await?;
        let row = sqlx::query(
            "SELECT id, user_id, name, app_id, quota_bytes, used_bytes, reserved_bytes, \
                    created_at, updated_at FROM applications WHERE id = $1",
        )
        .bind(application_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let application = row_to_admin_application(row)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(application)
    }

    async fn list_admin_jobs(&self, limit: usize) -> Result<Vec<AdminJobSummary>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT id, application_id, action_type, state, total_items, succeeded_items, \
                    failed_items, attempt_count, max_attempts, error_summary, created_at, updated_at \
             FROM async_jobs ORDER BY created_at DESC, id DESC LIMIT $1",
        )
        .bind(as_i64(limit as u64)?)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_admin_job).collect()
    }

    async fn admin_storage_summary(&self) -> Result<AdminStorageSummary, RepositoryError> {
        storage_summary(self).await
    }

    async fn admin_system_settings(&self) -> Result<AdminSystemSettings, RepositoryError> {
        let row = sqlx::query(
            "SELECT download_bytes_per_second, updated_at FROM system_settings WHERE singleton = TRUE",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(database_error)?;
        row_to_admin_system_settings(row)
    }

    async fn update_admin_system_settings(
        &self,
        actor: UserId,
        download_bytes_per_second: Option<u64>,
        request_id: &str,
        changed_at: OffsetDateTime,
    ) -> Result<AdminSystemSettings, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        advisory_admin_lock(&mut transaction).await?;
        let download_bytes_per_second = download_bytes_per_second.map(as_i64).transpose()?;
        let row = sqlx::query(
            "UPDATE system_settings SET download_bytes_per_second = $1, updated_by = $2, \
                    updated_request_id = $3, updated_at = $4 WHERE singleton = TRUE \
             RETURNING download_bytes_per_second, updated_at",
        )
        .bind(download_bytes_per_second)
        .bind(actor.as_uuid())
        .bind(request_id)
        .bind(changed_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let settings = row_to_admin_system_settings(row)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(settings)
    }

    async fn admin_metrics_snapshot(&self) -> Result<AdminMetricsSnapshot, RepositoryError> {
        let storage = storage_summary(self).await?;
        let row = sqlx::query(
            "SELECT \
                (SELECT COUNT(*) FROM async_jobs WHERE state = 'pending') AS pending_jobs, \
                (SELECT COUNT(*) FROM async_jobs WHERE state = 'running') AS running_jobs, \
                (SELECT COUNT(*) FROM outbox_events WHERE delivered_at IS NULL) AS pending_outbox, \
                (SELECT COUNT(*) FROM webhook_deliveries \
                    WHERE delivered_at IS NULL AND dead_lettered_at IS NULL) AS pending_webhook_deliveries, \
                (SELECT COUNT(*) FROM media WHERE state = 'delete_pending') AS pending_deletions",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(AdminMetricsSnapshot {
            storage,
            pending_jobs: row_u64(&row, "pending_jobs")?,
            running_jobs: row_u64(&row, "running_jobs")?,
            pending_outbox: row_u64(&row, "pending_outbox")?,
            pending_webhook_deliveries: row_u64(&row, "pending_webhook_deliveries")?,
            pending_deletions: row_u64(&row, "pending_deletions")?,
        })
    }

    async fn list_admin_audit(&self, limit: usize) -> Result<Vec<AuditEvent>, RepositoryError> {
        let rows =
            sqlx::query("SELECT * FROM audit_logs ORDER BY created_at DESC, id DESC LIMIT $1")
                .bind(as_i64(limit as u64)?)
                .fetch_all(&self.pool)
                .await
                .map_err(database_error)?;
        rows.into_iter().map(row_to_audit).collect()
    }
}

#[async_trait]
impl SecretKeyVersionRepository for PostgresRepository {
    async fn referenced_secret_key_versions(&self) -> Result<Vec<u32>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT DISTINCT secret_key_version FROM access_keys \
             UNION SELECT DISTINCT secret_key_version FROM webhook_endpoints \
             ORDER BY secret_key_version",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter()
            .map(|row| as_u32(row.try_get("secret_key_version").map_err(database_error)?))
            .collect()
    }
}

async fn advisory_admin_lock(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<(), RepositoryError> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(ADMIN_ADVISORY_LOCK)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    Ok(())
}

async fn insert_audit(
    transaction: &mut Transaction<'_, Postgres>,
    event: &AuditEvent,
) -> Result<(), RepositoryError> {
    sqlx::query(
        "INSERT INTO audit_logs \
         (id, application_id, actor_type, actor_id, action, target_type, target_id, \
          request_id, summary, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(&event.id)
    .bind(event.application_id.as_uuid())
    .bind(&event.actor_type)
    .bind(&event.actor_id)
    .bind(&event.action)
    .bind(&event.target_type)
    .bind(&event.target_id)
    .bind(&event.request_id)
    .bind(Json(event.summary.clone()))
    .bind(postgres_time(event.created_at))
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn storage_summary(
    repository: &PostgresRepository,
) -> Result<AdminStorageSummary, RepositoryError> {
    let row = sqlx::query(
        "SELECT \
            COALESCE((SELECT SUM(quota_bytes) FROM applications), 0)::BIGINT AS quota_bytes, \
            COALESCE((SELECT SUM(used_bytes) FROM applications), 0)::BIGINT AS used_bytes, \
            COALESCE((SELECT SUM(reserved_bytes) FROM applications), 0)::BIGINT AS reserved_bytes, \
            (SELECT COUNT(*) FROM media WHERE state != 'deleted') AS media_objects, \
            COALESCE((SELECT SUM(size_bytes) FROM variants WHERE status = 'ready'), 0)::BIGINT AS variant_bytes, \
            (SELECT COUNT(*) FROM variants WHERE status = 'ready') AS variants",
    )
    .fetch_one(&repository.pool)
    .await
    .map_err(database_error)?;
    Ok(AdminStorageSummary {
        quota_bytes: row_u64(&row, "quota_bytes")?,
        used_bytes: row_u64(&row, "used_bytes")?,
        reserved_bytes: row_u64(&row, "reserved_bytes")?,
        media_objects: row_u64(&row, "media_objects")?,
        variant_bytes: row_u64(&row, "variant_bytes")?,
        variants: row_u64(&row, "variants")?,
    })
}

fn row_to_audit(row: PgRow) -> Result<AuditEvent, RepositoryError> {
    Ok(AuditEvent {
        id: row.try_get("id").map_err(database_error)?,
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        actor_type: row.try_get("actor_type").map_err(database_error)?,
        actor_id: row.try_get("actor_id").map_err(database_error)?,
        action: row.try_get("action").map_err(database_error)?,
        target_type: row.try_get("target_type").map_err(database_error)?,
        target_id: row.try_get("target_id").map_err(database_error)?,
        request_id: row.try_get("request_id").map_err(database_error)?,
        summary: row
            .try_get::<Json<serde_json::Value>, _>("summary")
            .map_err(database_error)?
            .0,
        created_at: row.try_get("created_at").map_err(database_error)?,
    })
}

fn row_to_admin_user(row: PgRow) -> Result<AdminUserSummary, RepositoryError> {
    Ok(AdminUserSummary {
        id: UserId::from_uuid(row.try_get("id").map_err(database_error)?),
        email_normalized: row.try_get("email_normalized").map_err(database_error)?,
        email_verified_at: row.try_get("email_verified_at").map_err(database_error)?,
        status: row.try_get("status").map_err(database_error)?,
        system_role: row.try_get("system_role").map_err(database_error)?,
        last_login_at: row.try_get("last_login_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_admin_application(row: PgRow) -> Result<AdminApplicationSummary, RepositoryError> {
    Ok(AdminApplicationSummary {
        id: ApplicationId::from_uuid(row.try_get("id").map_err(database_error)?),
        owner_user_id: UserId::from_uuid(row.try_get("user_id").map_err(database_error)?),
        name: row.try_get("name").map_err(database_error)?,
        app_id: row.try_get("app_id").map_err(database_error)?,
        quota: QuotaSnapshot {
            quota_bytes: row_u64(&row, "quota_bytes")?,
            used_bytes: row_u64(&row, "used_bytes")?,
            reserved_bytes: row_u64(&row, "reserved_bytes")?,
        },
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_admin_system_settings(row: PgRow) -> Result<AdminSystemSettings, RepositoryError> {
    Ok(AdminSystemSettings {
        download_bytes_per_second: row
            .try_get::<Option<i64>, _>("download_bytes_per_second")
            .map_err(database_error)?
            .map(as_u64)
            .transpose()?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_admin_job(row: PgRow) -> Result<AdminJobSummary, RepositoryError> {
    Ok(AdminJobSummary {
        id: AsyncJobId::from_uuid(row.try_get("id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        action: row.try_get("action_type").map_err(database_error)?,
        state: row.try_get("state").map_err(database_error)?,
        total_items: row_i32_u64(&row, "total_items")?,
        succeeded_items: row_i32_u64(&row, "succeeded_items")?,
        failed_items: row_i32_u64(&row, "failed_items")?,
        attempt_count: row_i32_u64(&row, "attempt_count")?,
        max_attempts: row_i32_u64(&row, "max_attempts")?,
        error_summary: row.try_get("error_summary").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_u64(row: &PgRow, column: &str) -> Result<u64, RepositoryError> {
    as_u64(row.try_get(column).map_err(database_error)?)
}

fn row_i32_u64(row: &PgRow, column: &str) -> Result<u64, RepositoryError> {
    Ok(u64::from(as_u32(
        row.try_get(column).map_err(database_error)?,
    )?))
}
