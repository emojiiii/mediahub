use async_trait::async_trait;
use mediahub_core::{ApplicationId, AsyncJobId, OffsetDateTime, UserId};

use crate::{QuotaSnapshot, RepositoryError};

pub const DEFAULT_DOWNLOAD_BYTES_PER_SECOND: u64 = 32 * 1024 * 1024;
pub const MIN_DOWNLOAD_BYTES_PER_SECOND: u64 = 1024 * 1024;
pub const MAX_DOWNLOAD_BYTES_PER_SECOND: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminBootstrapOutcome {
    Completed(UserId),
    AlreadyCompleted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminUserSummary {
    pub id: UserId,
    pub email_normalized: String,
    pub email_verified_at: Option<OffsetDateTime>,
    pub status: String,
    pub system_role: String,
    pub last_login_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminApplicationSummary {
    pub id: ApplicationId,
    pub owner_user_id: UserId,
    pub name: String,
    pub app_id: String,
    pub quota: QuotaSnapshot,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdminJobSummary {
    pub id: AsyncJobId,
    pub application_id: ApplicationId,
    pub action: String,
    pub state: String,
    pub total_items: u64,
    pub succeeded_items: u64,
    pub failed_items: u64,
    pub attempt_count: u64,
    pub max_attempts: u64,
    pub error_summary: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdminStorageSummary {
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub reserved_bytes: u64,
    pub media_objects: u64,
    pub variant_bytes: u64,
    pub variants: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdminMetricsSnapshot {
    pub storage: AdminStorageSummary,
    pub pending_jobs: u64,
    pub running_jobs: u64,
    pub pending_outbox: u64,
    pub pending_webhook_deliveries: u64,
    pub pending_deletions: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdminSystemSettings {
    pub download_bytes_per_second: Option<u64>,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AuditEvent {
    pub id: String,
    pub application_id: ApplicationId,
    pub actor_type: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub request_id: String,
    pub summary: serde_json::Value,
    pub created_at: OffsetDateTime,
}

#[async_trait]
pub trait AuditRepository: Send + Sync {
    async fn record_audit(&self, event: &AuditEvent) -> Result<(), RepositoryError>;

    async fn list_audit(
        &self,
        application_id: ApplicationId,
        limit: usize,
    ) -> Result<Vec<AuditEvent>, RepositoryError>;
}

#[async_trait]
pub trait AdminRepository: Send + Sync {
    async fn bootstrap_admin(
        &self,
        email_normalized: &str,
        completed_at: OffsetDateTime,
    ) -> Result<AdminBootstrapOutcome, RepositoryError>;

    async fn list_admin_users(
        &self,
        limit: usize,
    ) -> Result<Vec<AdminUserSummary>, RepositoryError>;

    async fn transition_user_status(
        &self,
        actor: UserId,
        target: UserId,
        requested_status: &str,
        request_id: &str,
        changed_at: OffsetDateTime,
    ) -> Result<AdminUserSummary, RepositoryError>;

    async fn list_admin_applications(
        &self,
        limit: usize,
    ) -> Result<Vec<AdminApplicationSummary>, RepositoryError>;

    async fn update_application_quota(
        &self,
        actor: UserId,
        application_id: ApplicationId,
        quota_bytes: u64,
        request_id: &str,
        changed_at: OffsetDateTime,
    ) -> Result<AdminApplicationSummary, RepositoryError>;

    async fn list_admin_jobs(&self, limit: usize) -> Result<Vec<AdminJobSummary>, RepositoryError>;

    async fn admin_storage_summary(&self) -> Result<AdminStorageSummary, RepositoryError>;

    async fn admin_system_settings(&self) -> Result<AdminSystemSettings, RepositoryError>;

    async fn update_admin_system_settings(
        &self,
        actor: UserId,
        download_bytes_per_second: Option<u64>,
        request_id: &str,
        changed_at: OffsetDateTime,
    ) -> Result<AdminSystemSettings, RepositoryError>;

    async fn admin_metrics_snapshot(&self) -> Result<AdminMetricsSnapshot, RepositoryError>;

    async fn list_admin_audit(&self, limit: usize) -> Result<Vec<AuditEvent>, RepositoryError>;
}

#[async_trait]
pub trait SecretKeyVersionRepository: Send + Sync {
    async fn referenced_secret_key_versions(&self) -> Result<Vec<u32>, RepositoryError>;
}
