// HTTP response DTOs and conversions.

#[derive(Clone, Serialize)]
struct StatusResponse {
    status: &'static str,
}
#[derive(Serialize)]
struct CapabilitiesResponse {
    deployment_profile: &'static str,
    storage: [&'static str; 2],
    s3_gateway: bool,
    image_processing: bool,
    video_processing: bool,
    resumable_upload: bool,
    archive_restore: bool,
}
#[derive(Serialize)]
struct RegistrationResponse {
    email: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_token: Option<String>,
}
#[derive(Serialize)]
struct AuthStatusResponse {
    status: &'static str,
}
#[derive(Serialize)]
struct ForgotPasswordResponse {
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    reset_token: Option<String>,
}

#[derive(Serialize)]
struct ResendVerificationResponse {
    message: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification_token: Option<String>,
}
#[derive(Serialize)]
struct SessionResponse {
    id: String,
    expires_at: OffsetDateTime,
    last_seen_at: OffsetDateTime,
    created_ip: Option<String>,
    last_seen_ip: Option<String>,
    user_agent_summary: Option<String>,
    created_at: OffsetDateTime,
    is_current: bool,
}
impl From<SessionRecord> for SessionResponse {
    fn from(session: SessionRecord) -> Self {
        Self {
            id: session.id,
            expires_at: session.expires_at,
            last_seen_at: session.last_seen_at,
            created_ip: session.created_ip,
            last_seen_ip: session.last_seen_ip,
            user_agent_summary: session.user_agent_summary,
            created_at: session.created_at,
            is_current: session.is_current,
        }
    }
}
#[derive(Serialize)]
struct MeResponse {
    email: String,
    system_role: String,
    app_id: String,
    application_id: String,
    quota_bytes: u64,
    used_bytes: u64,
    reserved_bytes: u64,
}
impl MeResponse {
    fn new(
        email: String,
        application_id: ApplicationId,
        app_id: String,
        quota_bytes: u64,
        used_bytes: u64,
        reserved_bytes: u64,
    ) -> Self {
        Self {
            email,
            system_role: "user".into(),
            app_id,
            application_id: application_id.to_string(),
            quota_bytes,
            used_bytes,
            reserved_bytes,
        }
    }
    fn from_user_and_application(user: &UserAccount, application: &ApplicationSummary) -> Self {
        let mut response = Self::new(
            user.email_normalized.clone(),
            application.id,
            application.app_id.clone(),
            application.quota.quota_bytes,
            application.quota.used_bytes,
            application.quota.reserved_bytes,
        );
        response.system_role.clone_from(&user.system_role);
        response
    }
}

#[derive(Debug, Serialize)]
struct AdminUserResponse {
    id: String,
    email: String,
    email_verified_at: Option<OffsetDateTime>,
    status: String,
    system_role: String,
    last_login_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<AdminUserSummary> for AdminUserResponse {
    fn from(user: AdminUserSummary) -> Self {
        Self {
            id: user.id.to_string(),
            email: user.email_normalized,
            email_verified_at: user.email_verified_at,
            status: user.status,
            system_role: user.system_role,
            last_login_at: user.last_login_at,
            created_at: user.created_at,
            updated_at: user.updated_at,
        }
    }
}

#[derive(Serialize)]
struct AdminApplicationResponse {
    id: String,
    owner_user_id: String,
    app_id: String,
    name: String,
    quota_bytes: u64,
    used_bytes: u64,
    reserved_bytes: u64,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<AdminApplicationSummary> for AdminApplicationResponse {
    fn from(application: AdminApplicationSummary) -> Self {
        Self {
            id: application.id.to_string(),
            owner_user_id: application.owner_user_id.to_string(),
            app_id: application.app_id,
            name: application.name,
            quota_bytes: application.quota.quota_bytes,
            used_bytes: application.quota.used_bytes,
            reserved_bytes: application.quota.reserved_bytes,
            created_at: application.created_at,
            updated_at: application.updated_at,
        }
    }
}

#[derive(Serialize)]
struct AdminJobResponse {
    id: String,
    application_id: String,
    action: String,
    state: String,
    total_items: u64,
    succeeded_items: u64,
    failed_items: u64,
    attempt_count: u64,
    max_attempts: u64,
    error_summary: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<AdminJobSummary> for AdminJobResponse {
    fn from(job: AdminJobSummary) -> Self {
        Self {
            id: job.id.to_string(),
            application_id: job.application_id.to_string(),
            action: job.action,
            state: job.state,
            total_items: job.total_items,
            succeeded_items: job.succeeded_items,
            failed_items: job.failed_items,
            attempt_count: job.attempt_count,
            max_attempts: job.max_attempts,
            error_summary: job.error_summary,
            created_at: job.created_at,
            updated_at: job.updated_at,
        }
    }
}

#[derive(Serialize)]
struct AdminStorageResponse {
    quota_bytes: u64,
    used_bytes: u64,
    reserved_bytes: u64,
    media_objects: u64,
    variant_bytes: u64,
    variants: u64,
    disk_total_bytes: u64,
    disk_available_bytes: u64,
}

impl From<AdminStorageSummary> for AdminStorageResponse {
    fn from(summary: AdminStorageSummary) -> Self {
        Self {
            quota_bytes: summary.quota_bytes,
            used_bytes: summary.used_bytes,
            reserved_bytes: summary.reserved_bytes,
            media_objects: summary.media_objects,
            variant_bytes: summary.variant_bytes,
            variants: summary.variants,
            disk_total_bytes: 0,
            disk_available_bytes: 0,
        }
    }
}

#[derive(Debug, Serialize)]
struct AdminSettingsResponse {
    download_bytes_per_second: Option<u64>,
    updated_at: OffsetDateTime,
}

impl From<AdminSystemSettings> for AdminSettingsResponse {
    fn from(settings: AdminSystemSettings) -> Self {
        Self {
            download_bytes_per_second: settings.download_bytes_per_second,
            updated_at: settings.updated_at,
        }
    }
}

#[derive(Serialize)]
struct AdminAuditResponse {
    id: String,
    application_id: String,
    actor_type: String,
    actor_id: String,
    action: String,
    target_type: String,
    target_id: String,
    request_id: String,
    summary: serde_json::Value,
    created_at: OffsetDateTime,
}

impl From<AuditEvent> for AdminAuditResponse {
    fn from(event: AuditEvent) -> Self {
        Self {
            id: event.id,
            application_id: event.application_id.to_string(),
            actor_type: event.actor_type,
            actor_id: event.actor_id,
            action: event.action,
            target_type: event.target_type,
            target_id: event.target_id,
            request_id: event.request_id,
            summary: event.summary,
            created_at: event.created_at,
        }
    }
}

#[derive(Serialize)]
struct ApplicationResponse {
    id: String,
    app_id: String,
    name: String,
    quota_bytes: u64,
    used_bytes: u64,
    reserved_bytes: u64,
}
impl From<ApplicationSummary> for ApplicationResponse {
    fn from(application: ApplicationSummary) -> Self {
        Self {
            id: application.id.to_string(),
            app_id: application.app_id,
            name: application.name,
            quota_bytes: application.quota.quota_bytes,
            used_bytes: application.quota.used_bytes,
            reserved_bytes: application.quota.reserved_bytes,
        }
    }
}
#[derive(Serialize)]
struct BucketResponse {
    id: String,
    name: String,
    visibility: Visibility,
    default_ttl_seconds: Option<u64>,
    max_object_size: Option<u64>,
    allowed_mime_types: Vec<String>,
    lifecycle_rules: Vec<LifecycleRule>,
}
impl From<Bucket> for BucketResponse {
    fn from(bucket: Bucket) -> Self {
        let policy = bucket.policy();
        Self {
            id: bucket.id().to_string(),
            name: bucket.name().to_owned(),
            visibility: policy.visibility(),
            default_ttl_seconds: policy.default_ttl_seconds(),
            max_object_size: policy.max_object_size(),
            allowed_mime_types: policy.allowed_mime_types().map(str::to_owned).collect(),
            lifecycle_rules: policy.lifecycle_rules().to_vec(),
        }
    }
}
#[derive(Serialize)]
struct MediaResponse {
    id: String,
    bucket_id: String,
    object_key: String,
    display_name: String,
    state: mediahub_core::MediaState,
    mime: String,
    size_bytes: u64,
    sha256: String,
    revision: u64,
    width: Option<u32>,
    height: Option<u32>,
    visibility: Option<Visibility>,
    expires_at: Option<OffsetDateTime>,
    metadata: serde_json::Value,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

#[derive(Serialize)]
struct MediaListResponse {
    items: Vec<MediaResponse>,
    common_prefixes: Vec<String>,
    next_cursor: Option<String>,
}

#[derive(Serialize)]
struct AccessKeyResponse {
    access_key_id: String,
    name: String,
    permissions: Vec<String>,
    secret_last_four: String,
    expires_at: Option<OffsetDateTime>,
    revoked_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
}

impl From<AccessKeyRecord> for AccessKeyResponse {
    fn from(access_key: AccessKeyRecord) -> Self {
        Self {
            access_key_id: access_key.access_key_id,
            name: access_key.name,
            permissions: access_key.permissions,
            secret_last_four: access_key.secret_last_four,
            expires_at: access_key.expires_at,
            revoked_at: access_key.revoked_at,
            created_at: access_key.created_at,
        }
    }
}

#[derive(Serialize)]
struct CreateAccessKeyResponse {
    app_id: String,
    access_key_id: String,
    secret_access_key: String,
    expires_at: Option<OffsetDateTime>,
}

#[derive(Serialize)]
struct WebhookResponse {
    id: String,
    url: String,
    events: Vec<String>,
    enabled: bool,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}
impl From<WebhookEndpoint> for WebhookResponse {
    fn from(endpoint: WebhookEndpoint) -> Self {
        Self {
            id: endpoint.id,
            url: endpoint.url,
            events: endpoint.subscribed_events,
            enabled: endpoint.enabled,
            created_at: endpoint.created_at,
            updated_at: endpoint.updated_at,
        }
    }
}

#[derive(Serialize)]
struct WebhookDeliveryHistoryResponse {
    event_id: String,
    endpoint_id: String,
    event_type: String,
    attempt_count: u32,
    status: &'static str,
    last_response_status: Option<u16>,
    last_error: Option<String>,
    next_attempt_at: Option<OffsetDateTime>,
    delivered_at: Option<OffsetDateTime>,
    dead_lettered_at: Option<OffsetDateTime>,
    replay_count: u32,
    last_replayed_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<WebhookDeliveryHistoryItem> for WebhookDeliveryHistoryResponse {
    fn from(item: WebhookDeliveryHistoryItem) -> Self {
        Self {
            event_id: item.event_id,
            endpoint_id: item.endpoint_id,
            event_type: item.event_type,
            attempt_count: item.attempt_count,
            status: webhook_delivery_status_name(item.status),
            last_response_status: item.last_response_status,
            last_error: item.last_error,
            next_attempt_at: item.next_attempt_at,
            delivered_at: item.delivered_at,
            dead_lettered_at: item.dead_lettered_at,
            replay_count: item.replay_count,
            last_replayed_at: item.last_replayed_at,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}

#[derive(Serialize)]
struct WebhookDeliveryListResponse {
    items: Vec<WebhookDeliveryHistoryResponse>,
    next_cursor: Option<String>,
}

#[derive(Serialize)]
struct CreateWebhookResponse {
    endpoint: WebhookResponse,
    secret: String,
}

#[derive(Serialize)]
struct UpdateWebhookResponse {
    endpoint: WebhookResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
}

#[derive(Serialize)]
struct SignedMediaUrlResponse {
    url: String,
    expires_at: OffsetDateTime,
}

#[derive(Serialize)]
struct CreateUploadSessionResponse {
    upload_id: String,
    media_id: String,
    bucket_id: String,
    object_key: String,
    expected_size: u64,
    expected_mime: String,
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    expires_at: OffsetDateTime,
}

#[derive(Serialize)]
struct UploadTargetResponse {
    method: String,
    url: String,
    headers: BTreeMap<String, String>,
    expires_at: OffsetDateTime,
}

impl UploadTargetResponse {
    fn from_target(state: &AppState, session: &UploadSession, target: UploadTarget) -> Self {
        Self {
            method: target.method,
            url: client_upload_target_url(state, session, target.url, target.expires_at),
            headers: target.headers,
            expires_at: target.expires_at,
        }
    }
}

fn client_upload_target_url(
    state: &AppState,
    session: &UploadSession,
    target_url: String,
    expires_at: OffsetDateTime,
) -> String {
    if session.storage_backend() == "local" {
        let token = state
            .media_url_signer
            .sign_upload_content(session.id(), expires_at);
        format!("{target_url}?token={token}")
    } else {
        target_url
    }
}

#[derive(Serialize)]
struct UploadSessionResponse {
    upload_id: String,
    media_id: String,
    bucket_id: String,
    object_key: String,
    expected_size: u64,
    expected_mime: String,
    state: UploadSessionState,
    expires_at: OffsetDateTime,
    completed_at: Option<OffsetDateTime>,
    cancelled_at: Option<OffsetDateTime>,
    expired_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    #[serde(skip_serializing_if = "Option::is_none")]
    upload_target: Option<UploadTargetResponse>,
}

impl UploadSessionResponse {
    fn from_session(
        session: &mediahub_core::UploadSession,
        upload_target: Option<UploadTargetResponse>,
    ) -> Self {
        Self {
            upload_id: session.id().to_string(),
            media_id: session.media_id().to_string(),
            bucket_id: session.bucket_id().to_string(),
            object_key: session.object_key().to_owned(),
            expected_size: session.expected_size(),
            expected_mime: session.expected_mime().to_owned(),
            state: session.state(),
            expires_at: session.session_expires_at(),
            completed_at: session.completed_at(),
            cancelled_at: session.cancelled_at(),
            expired_at: session.expired_at(),
            created_at: session.created_at(),
            updated_at: session.updated_at(),
            upload_target,
        }
    }
}

#[derive(Serialize)]
struct CompleteUploadSessionResponse {
    upload_id: String,
    event_id: String,
    already_completed: bool,
    media: MediaResponse,
}

#[derive(Serialize)]
struct AuditResponse {
    id: String,
    actor_type: String,
    actor_id: String,
    action: String,
    target_type: String,
    target_id: String,
    request_id: String,
    summary: serde_json::Value,
    created_at: OffsetDateTime,
}

impl From<AuditEvent> for AuditResponse {
    fn from(event: AuditEvent) -> Self {
        Self {
            id: event.id,
            actor_type: event.actor_type,
            actor_id: event.actor_id,
            action: event.action,
            target_type: event.target_type,
            target_id: event.target_id,
            request_id: event.request_id,
            summary: event.summary,
            created_at: event.created_at,
        }
    }
}

#[derive(Serialize)]
struct AsyncJobResponse {
    id: String,
    application_id: String,
    operation_scope: String,
    request_id: Option<String>,
    action: AsyncJobAction,
    state: mediahub_core::AsyncJobState,
    total_items: u32,
    succeeded_items: u32,
    failed_items: u32,
    attempt_count: u32,
    max_attempts: u32,
    next_attempt_at: Option<OffsetDateTime>,
    error_summary: Option<String>,
    started_at: Option<OffsetDateTime>,
    completed_at: Option<OffsetDateTime>,
    failed_at: Option<OffsetDateTime>,
    cancelled_at: Option<OffsetDateTime>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
}

impl From<mediahub_core::AsyncJob> for AsyncJobResponse {
    fn from(job: mediahub_core::AsyncJob) -> Self {
        Self {
            id: job.id().to_string(),
            application_id: job.application_id().to_string(),
            operation_scope: job.operation_scope().to_owned(),
            request_id: job.request_id().map(str::to_owned),
            action: job.action().clone(),
            state: job.state(),
            total_items: job.total_items(),
            succeeded_items: job.succeeded_items(),
            failed_items: job.failed_items(),
            attempt_count: job.attempt_count(),
            max_attempts: job.max_attempts(),
            next_attempt_at: job.next_attempt_at(),
            error_summary: job.error_summary().map(str::to_owned),
            started_at: job.started_at(),
            completed_at: job.completed_at(),
            failed_at: job.failed_at(),
            cancelled_at: job.cancelled_at(),
            created_at: job.created_at(),
            updated_at: job.updated_at(),
        }
    }
}

#[derive(Serialize)]
struct AsyncJobDetailsResponse {
    job: AsyncJobResponse,
    item_results: Vec<AsyncJobItemResult>,
}

impl From<mediahub_app::AsyncJobDetails> for AsyncJobDetailsResponse {
    fn from(details: mediahub_app::AsyncJobDetails) -> Self {
        Self {
            job: details.job.into(),
            item_results: details.item_results,
        }
    }
}

#[derive(Serialize)]
struct AsyncJobReceiptResponse {
    job: AsyncJobResponse,
    already_existed: bool,
}

impl From<mediahub_app::AsyncJobReceipt> for AsyncJobReceiptResponse {
    fn from(receipt: mediahub_app::AsyncJobReceipt) -> Self {
        Self {
            job: receipt.job.into(),
            already_existed: receipt.already_existed,
        }
    }
}

