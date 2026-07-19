use async_trait::async_trait;
use mediahub_core::{ApplicationId, OffsetDateTime, UserId};

use crate::{QuotaSnapshot, RepositoryError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserAccount {
    pub id: UserId,
    pub email_normalized: String,
    pub password_hash: String,
    pub email_verified_at: Option<OffsetDateTime>,
    pub status: String,
    pub system_role: String,
    pub last_login_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneTimeTokenPurpose {
    VerifyEmail,
    ResetPassword,
}

impl OneTimeTokenPurpose {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::VerifyEmail => "verify_email",
            Self::ResetPassword => "reset_password",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRecord {
    pub id: String,
    pub expires_at: OffsetDateTime,
    pub last_seen_at: OffsetDateTime,
    pub created_ip: Option<String>,
    pub last_seen_ip: Option<String>,
    pub user_agent_summary: Option<String>,
    pub created_at: OffsetDateTime,
    pub is_current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApplicationSummary {
    pub id: ApplicationId,
    pub name: String,
    pub app_id: String,
    pub quota: QuotaSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewAccessKey {
    pub id: String,
    pub application_id: ApplicationId,
    pub access_key_id: String,
    pub secret_ciphertext: String,
    pub secret_key_version: u32,
    pub secret_last_four: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessKeyRecord {
    pub id: String,
    pub application_id: ApplicationId,
    pub access_key_id: String,
    pub secret_ciphertext: String,
    pub secret_key_version: u32,
    pub secret_last_four: String,
    pub name: String,
    pub permissions: Vec<String>,
    pub expires_at: Option<OffsetDateTime>,
    pub revoked_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

#[async_trait]
pub trait AuthRepository: Send + Sync {
    async fn create_user(
        &self,
        user_id: UserId,
        email_normalized: &str,
        password_hash: &str,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    async fn find_user_by_email(
        &self,
        email_normalized: &str,
    ) -> Result<Option<UserAccount>, RepositoryError>;

    async fn create_session(
        &self,
        user_id: UserId,
        token_hash: &str,
        csrf_token_hash: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    #[allow(clippy::too_many_arguments)]
    async fn create_session_with_context(
        &self,
        user_id: UserId,
        token_hash: &str,
        csrf_token_hash: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
        created_ip: Option<&str>,
        user_agent_summary: Option<&str>,
    ) -> Result<(), RepositoryError>;

    async fn find_user_by_session_hash(
        &self,
        token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<Option<UserAccount>, RepositoryError>;

    async fn record_user_login(
        &self,
        user_id: UserId,
        logged_in_at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    async fn valid_session_csrf(
        &self,
        token_hash: &str,
        csrf_token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<bool, RepositoryError>;

    async fn delete_session_by_hash(&self, token_hash: &str) -> Result<(), RepositoryError>;

    async fn create_one_time_token(
        &self,
        user_id: UserId,
        purpose: OneTimeTokenPurpose,
        token_hash: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    async fn consume_email_verification_token(
        &self,
        token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<bool, RepositoryError>;

    async fn consume_password_reset_token(
        &self,
        token_hash: &str,
        password_hash: &str,
        now: OffsetDateTime,
    ) -> Result<bool, RepositoryError>;

    async fn list_active_sessions(
        &self,
        user_id: UserId,
        current_token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<Vec<SessionRecord>, RepositoryError>;

    async fn revoke_session(
        &self,
        user_id: UserId,
        session_id: &str,
        current_token_hash: &str,
        now: OffsetDateTime,
    ) -> Result<Option<bool>, RepositoryError>;

    async fn revoke_all_sessions(
        &self,
        user_id: UserId,
        now: OffsetDateTime,
    ) -> Result<u64, RepositoryError>;
}

#[async_trait]
pub trait ApplicationRepository: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn create_application(
        &self,
        application_id: ApplicationId,
        user_id: UserId,
        name: &str,
        app_id: &str,
        quota_bytes: u64,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError>;

    async fn default_application_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Option<ApplicationSummary>, RepositoryError>;

    async fn find_application_by_id(
        &self,
        application_id: ApplicationId,
    ) -> Result<Option<ApplicationSummary>, RepositoryError>;

    async fn find_application_by_app_id(
        &self,
        app_id: &str,
    ) -> Result<Option<ApplicationSummary>, RepositoryError>;

    async fn list_applications_for_user(
        &self,
        user_id: UserId,
    ) -> Result<Vec<ApplicationSummary>, RepositoryError>;

    async fn application_for_user_by_app_id(
        &self,
        user_id: UserId,
        app_id: &str,
    ) -> Result<Option<ApplicationSummary>, RepositoryError>;

    async fn application_for_user_by_id(
        &self,
        user_id: UserId,
        application_id: ApplicationId,
    ) -> Result<Option<ApplicationSummary>, RepositoryError>;

    async fn update_application_name_for_user(
        &self,
        user_id: UserId,
        app_id: &str,
        name: &str,
        updated_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError>;

    async fn delete_application_for_user(
        &self,
        user_id: UserId,
        app_id: &str,
    ) -> Result<bool, RepositoryError>;
}

#[async_trait]
pub trait AccessKeyRepository: Send + Sync {
    async fn create_access_key(&self, access_key: &NewAccessKey) -> Result<(), RepositoryError>;

    async fn list_access_keys(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<AccessKeyRecord>, RepositoryError>;

    async fn find_active_access_key(
        &self,
        access_key_id: &str,
        now: OffsetDateTime,
    ) -> Result<Option<AccessKeyRecord>, RepositoryError>;

    async fn find_access_key(
        &self,
        access_key_id: &str,
    ) -> Result<Option<AccessKeyRecord>, RepositoryError>;

    async fn update_access_key(
        &self,
        access_key_id: &str,
        application_id: ApplicationId,
        name: &str,
        permissions: &[String],
        expires_at: Option<OffsetDateTime>,
    ) -> Result<bool, RepositoryError>;

    async fn revoke_access_key(
        &self,
        access_key_id: &str,
        application_id: ApplicationId,
        revoked_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError>;

    async fn record_replay_nonce(
        &self,
        access_key_id: &str,
        nonce: &str,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<(), RepositoryError>;
}
