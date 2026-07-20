#![allow(dead_code)]

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::ToSchema;

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
}

#[derive(Serialize, Deserialize, ToSchema)]
pub enum Permission {
    #[serde(rename = "application:read")]
    ApplicationRead,
    #[serde(rename = "bucket:list")]
    BucketList,
    #[serde(rename = "bucket:manage")]
    BucketManage,
    #[serde(rename = "media:list")]
    MediaList,
    #[serde(rename = "media:read")]
    MediaRead,
    #[serde(rename = "media:upload")]
    MediaUpload,
    #[serde(rename = "media:update")]
    MediaUpdate,
    #[serde(rename = "media:delete")]
    MediaDelete,
    #[serde(rename = "webhook:manage")]
    WebhookManage,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AsyncJobAction {
    UpdateTtlSeconds {
        #[schema(minimum = 1)]
        ttl_seconds: Option<u64>,
    },
    UpdateVisibility {
        visibility: Visibility,
    },
    Delete,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum LifecycleRule {
    ExpireAfter {
        id: String,
        enabled: bool,
        prefix: String,
        duration_seconds: u64,
    },
    KeepLatest {
        id: String,
        enabled: bool,
        prefix: String,
        count: u32,
    },
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationStatus {
    PendingVerification,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum WebhookDeliveryStatus {
    Pending,
    Delivered,
    DeadLettered,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SystemRole {
    User,
    Admin,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    PendingVerification,
    Active,
    Suspended,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum UploadSessionState {
    Pending,
    Completed,
    Cancelled,
    Expired,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AsyncJobState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AsyncJobItemState {
    Pending,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuditActorType {
    User,
    AccessKey,
    System,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AccessKey {
    pub access_key_id: String,
    pub name: String,
    pub permissions: Vec<Permission>,
    pub secret_last_four: String,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminApplication {
    pub id: String,
    pub owner_user_id: String,
    pub app_id: String,
    pub name: String,
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub reserved_bytes: u64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminUpdateApplicationQuota {
    #[schema(maximum = 9_223_372_036_854_775_807_u64)]
    pub quota_bytes: u64,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminAudit {
    pub id: String,
    pub application_id: String,
    #[schema(inline)]
    pub actor_type: AuditActorType,
    pub actor_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub request_id: String,
    pub summary: BTreeMap<String, Value>,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminJob {
    pub id: String,
    pub application_id: String,
    pub action: String,
    #[schema(inline)]
    pub state: AsyncJobState,
    pub total_items: u64,
    pub succeeded_items: u64,
    pub failed_items: u64,
    pub attempt_count: u64,
    pub max_attempts: u64,
    pub error_summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminStorage {
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub reserved_bytes: u64,
    pub media_objects: u64,
    pub variant_bytes: u64,
    pub variants: u64,
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminSettings {
    /// Per-download bandwidth limit in bytes per second; null means unlimited.
    #[schema(required = true, minimum = 1_048_576_u64, maximum = 1_073_741_824_u64)]
    pub download_bytes_per_second: Option<u64>,
    #[schema(format = DateTime)]
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminUpdateSettings {
    /// Per-download bandwidth limit in bytes per second; null means unlimited.
    #[schema(required = true, minimum = 1_048_576_u64, maximum = 1_073_741_824_u64)]
    pub download_bytes_per_second: Option<u64>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminUpdateUserStatus {
    pub status: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AdminUser {
    pub id: String,
    pub email: String,
    pub email_verified_at: Option<String>,
    #[schema(inline)]
    pub status: UserStatus,
    #[schema(inline)]
    pub system_role: SystemRole,
    pub last_login_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Application {
    pub id: String,
    pub app_id: String,
    pub name: String,
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub reserved_bytes: u64,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AsyncJob {
    pub id: String,
    pub application_id: String,
    pub operation_scope: String,
    #[schema(required = true)]
    pub request_id: Option<String>,
    pub action: AsyncJobAction,
    pub state: AsyncJobState,
    pub total_items: u64,
    pub succeeded_items: u64,
    pub failed_items: u64,
    pub attempt_count: u64,
    pub max_attempts: u64,
    #[schema(required = true, format = DateTime)]
    pub next_attempt_at: Option<String>,
    #[schema(required = true)]
    pub error_summary: Option<String>,
    #[schema(required = true, format = DateTime)]
    pub started_at: Option<String>,
    #[schema(required = true, format = DateTime)]
    pub completed_at: Option<String>,
    #[schema(required = true, format = DateTime)]
    pub failed_at: Option<String>,
    #[schema(required = true, format = DateTime)]
    pub cancelled_at: Option<String>,
    #[schema(format = DateTime)]
    pub created_at: String,
    #[schema(format = DateTime)]
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AsyncJobItemResult {
    pub job_id: String,
    pub application_id: String,
    pub media_id: String,
    pub ordinal: u32,
    pub state: AsyncJobItemState,
    pub attempt_count: u32,
    #[schema(required = true)]
    pub result: Option<Value>,
    #[schema(required = true)]
    pub error_code: Option<String>,
    #[schema(required = true)]
    pub error_summary: Option<String>,
    #[schema(required = true, format = DateTime)]
    pub started_at: Option<String>,
    #[schema(required = true, format = DateTime)]
    pub completed_at: Option<String>,
    #[schema(format = DateTime)]
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AsyncJobDetails {
    pub job: AsyncJob,
    pub item_results: Vec<AsyncJobItemResult>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AsyncJobReceipt {
    pub job: AsyncJob,
    pub already_existed: bool,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditEvent {
    pub id: String,
    pub actor_type: String,
    pub actor_id: String,
    pub action: String,
    pub target_type: String,
    pub target_id: String,
    pub request_id: String,
    pub summary: Value,
    pub created_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthStatus {
    pub status: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchItemError {
    pub code: String,
    pub message: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchItemResult {
    pub media_id: String,
    pub state: String,
    pub result: Option<Value>,
    #[schema(inline)]
    pub error: Option<BatchItemError>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchMediaRequest {
    pub action: AsyncJobAction,
    #[schema(min_items = 1, max_items = 1000)]
    pub media_ids: Vec<uuid::Uuid>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchMediaResponse {
    pub results: Vec<BatchItemResult>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Bucket {
    pub id: String,
    pub name: String,
    pub visibility: Visibility,
    pub default_ttl_seconds: Option<u64>,
    pub max_object_size: Option<u64>,
    pub allowed_mime_types: Vec<String>,
    pub lifecycle_rules: Vec<LifecycleRule>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub deployment_profile: String,
    pub storage: Vec<String>,
    pub s3_gateway: bool,
    pub image_processing: bool,
    pub video_processing: bool,
    pub resumable_upload: bool,
    pub archive_restore: bool,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteUploadSession {
    pub sha256: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAccessKey {
    pub name: String,
    pub permissions: Vec<Permission>,
    pub expires_at: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateAccessKeyResponse {
    pub app_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub expires_at: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateApplication {
    pub name: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateBucket {
    pub name: String,
    pub visibility: Option<Visibility>,
    pub default_ttl_seconds: Option<u64>,
    pub max_object_size: Option<u64>,
    #[schema(nullable = false)]
    pub allowed_mime_types: Option<Vec<String>>,
    #[schema(nullable = false)]
    pub lifecycle_rules: Option<Vec<LifecycleRule>>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUploadSession {
    pub bucket: String,
    pub object_key: Option<String>,
    pub original_name: Option<String>,
    pub display_name: Option<String>,
    pub extension: Option<String>,
    #[schema(minimum = 1, maximum = 2_147_483_648_u64)]
    pub expected_size: u64,
    pub content_type: String,
    pub visibility: Option<Visibility>,
    pub ttl_seconds: Option<u64>,
    pub metadata: Option<Value>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadTarget {
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub expires_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateUploadSessionResponse {
    pub upload_id: String,
    pub media_id: String,
    pub bucket_id: String,
    pub object_key: String,
    pub expected_size: u64,
    pub expected_mime: String,
    pub method: String,
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub expires_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWebhook {
    pub url: String,
    pub events: Vec<String>,
    #[serde(default)]
    #[schema(nullable = false, default = true)]
    pub enabled: Option<bool>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Webhook {
    pub id: String,
    pub url: String,
    pub events: Vec<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateWebhookResponse {
    pub endpoint: Webhook,
    pub secret: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Credentials {
    pub email: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ErrorDetail {
    pub code: String,
    pub message: String,
    pub request_id: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Error {
    #[schema(inline)]
    pub error: ErrorDetail,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ForgotPassword {
    pub email: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ForgotPasswordResponse {
    pub message: String,
    #[schema(required = false)]
    pub reset_token: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Me {
    pub email: String,
    #[schema(inline)]
    pub system_role: SystemRole,
    pub app_id: String,
    pub application_id: String,
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub reserved_bytes: u64,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Media {
    pub id: String,
    pub bucket_id: String,
    pub object_key: String,
    pub display_name: String,
    pub state: String,
    pub mime: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub revision: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub visibility: Option<Visibility>,
    pub expires_at: Option<String>,
    pub metadata: Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MediaPage {
    pub items: Vec<Media>,
    pub common_prefixes: Vec<String>,
    #[schema(required)]
    pub next_cursor: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OneTimeToken {
    pub token: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RegistrationResponse {
    pub email: String,
    #[schema(inline)]
    pub status: RegistrationStatus,
    #[schema(required = false)]
    pub verification_token: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResendVerificationResponse {
    pub message: String,
    #[schema(required = false)]
    pub verification_token: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ResetPassword {
    pub token: String,
    pub password: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub id: String,
    pub expires_at: String,
    pub last_seen_at: String,
    pub created_ip: Option<String>,
    pub last_seen_ip: Option<String>,
    pub user_agent_summary: Option<String>,
    pub created_at: String,
    pub is_current: bool,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedMediaUrl {
    pub url: String,
    pub expires_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateAccessKey {
    pub name: Option<String>,
    pub permissions: Option<Vec<Permission>>,
    pub expires_at: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateApplication {
    pub name: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateBucket {
    pub visibility: Option<Visibility>,
    pub default_ttl_seconds: Option<u64>,
    pub max_object_size: Option<u64>,
    pub allowed_mime_types: Option<Vec<String>>,
    pub lifecycle_rules: Option<Vec<LifecycleRule>>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateMedia {
    pub display_name: Option<String>,
    pub visibility: Option<Visibility>,
    pub ttl_seconds: Option<u64>,
    pub metadata: Option<Value>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateWebhook {
    pub url: Option<String>,
    pub events: Option<Vec<String>>,
    pub enabled: Option<bool>,
    #[serde(default)]
    #[schema(nullable = false, default = false)]
    pub rotate_secret: Option<bool>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateWebhookResponse {
    pub endpoint: Webhook,
    pub secret: Option<String>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadMedia {
    pub bucket: String,
    pub object_key: Option<String>,
    pub display_name: Option<String>,
    pub visibility: Option<Visibility>,
    pub ttl_seconds: Option<u64>,
    pub metadata: Option<Value>,
    pub file: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UploadSession {
    pub upload_id: String,
    pub media_id: String,
    pub bucket_id: String,
    pub object_key: String,
    pub expected_size: u64,
    pub expected_mime: String,
    #[schema(inline)]
    pub state: UploadSessionState,
    pub expires_at: String,
    #[schema(required)]
    pub completed_at: Option<String>,
    #[schema(required)]
    pub cancelled_at: Option<String>,
    #[schema(required)]
    pub expired_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub upload_target: Option<UploadTarget>,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteUploadSessionResponse {
    pub upload_id: String,
    pub event_id: String,
    pub already_completed: bool,
    pub media: Media,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WebhookDelivery {
    pub event_id: String,
    pub endpoint_id: String,
    pub event_type: String,
    pub attempt_count: u32,
    #[schema(inline)]
    pub status: WebhookDeliveryStatus,
    pub last_response_status: Option<u16>,
    pub last_error: Option<String>,
    pub next_attempt_at: Option<String>,
    pub delivered_at: Option<String>,
    pub dead_lettered_at: Option<String>,
    pub replay_count: u32,
    pub last_replayed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WebhookDeliveryPage {
    pub items: Vec<WebhookDelivery>,
    #[schema(required)]
    pub next_cursor: Option<String>,
}
