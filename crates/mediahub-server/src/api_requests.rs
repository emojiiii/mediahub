// HTTP request DTOs and request parsing.

// HTTP request and response DTOs plus conversion helpers.

struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

struct ParsedUpload {
    bucket: String,
    object_key: String,
    original_name: Option<String>,
    display_name: String,
    extension: Option<String>,
    mime: String,
    content: Vec<u8>,
    visibility_override: Option<Visibility>,
    ttl_seconds: Option<u64>,
    metadata: ClientMetadata,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterRequest {
    email: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OneTimeTokenRequest {
    token: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    email: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForgotPasswordRequest {
    email: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetPasswordRequest {
    token: String,
    password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateApplicationRequest {
    name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateApplicationRequest {
    name: String,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct AdminListQuery {
    limit: Option<usize>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdminUpdateUserStatusRequest {
    status: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdminUpdateApplicationQuotaRequest {
    quota_bytes: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdminUpdateSettingsRequest {
    #[serde(default, deserialize_with = "deserialize_nullable_option")]
    download_bytes_per_second: Option<Option<u64>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateBucketRequest {
    name: String,
    #[serde(default)]
    visibility: Option<Visibility>,
    #[serde(default)]
    default_ttl_seconds: Option<u64>,
    #[serde(default)]
    max_object_size: Option<u64>,
    #[serde(default)]
    allowed_mime_types: Vec<String>,
    #[serde(default)]
    lifecycle_rules: Vec<LifecycleRule>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateBucketRequest {
    #[serde(default)]
    visibility: Option<Visibility>,
    #[serde(default, deserialize_with = "deserialize_nullable_option")]
    default_ttl_seconds: Option<Option<u64>>,
    #[serde(default, deserialize_with = "deserialize_nullable_option")]
    max_object_size: Option<Option<u64>>,
    #[serde(default)]
    allowed_mime_types: Option<Vec<String>>,
    #[serde(default)]
    lifecycle_rules: Option<Vec<LifecycleRule>>,
}
impl UpdateBucketRequest {
    const fn has_changes(&self) -> bool {
        self.visibility.is_some()
            || self.default_ttl_seconds.is_some()
            || self.max_object_size.is_some()
            || self.allowed_mime_types.is_some()
            || self.lifecycle_rules.is_some()
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateMediaRequest {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default, deserialize_with = "deserialize_nullable_option")]
    visibility: Option<Option<Visibility>>,
    #[serde(default, deserialize_with = "deserialize_nullable_option")]
    ttl_seconds: Option<Option<u64>>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BatchMediaRequest {
    action: AsyncJobAction,
    media_ids: Vec<String>,
}

#[derive(Serialize)]
struct BatchMediaResponse {
    results: Vec<BatchItemResponse>,
}

#[derive(Serialize)]
struct BatchItemResponse {
    media_id: String,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<BatchItemErrorResponse>,
}

#[derive(Serialize)]
struct BatchItemErrorResponse {
    code: &'static str,
    message: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateUploadSessionHttpRequest {
    bucket: String,
    #[serde(default)]
    object_key: Option<String>,
    #[serde(default)]
    original_name: Option<String>,
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    extension: Option<String>,
    expected_size: u64,
    content_type: String,
    #[serde(default)]
    visibility: Option<Visibility>,
    #[serde(default)]
    ttl_seconds: Option<u64>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompleteUploadSessionHttpRequest {
    sha256: String,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct UploadContentQuery {
    token: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAccessKeyRequest {
    name: String,
    permissions: Vec<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateAccessKeyRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    permissions: Option<Vec<String>>,
    #[serde(default, deserialize_with = "deserialize_nullable_option")]
    expires_at: Option<Option<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateWebhookRequest {
    url: String,
    events: Vec<String>,
    #[serde(default = "default_true")]
    enabled: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ListWebhookDeliveriesQuery {
    status: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WebhookDeliveryCursorToken {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    updated_at: Option<i64>,
    row_id: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateWebhookRequest {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    events: Option<Vec<String>>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    rotate_secret: bool,
}
impl UpdateWebhookRequest {
    const fn has_changes(&self) -> bool {
        self.url.is_some() || self.events.is_some() || self.enabled.is_some() || self.rotate_secret
    }
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ListMediaQuery {
    bucket: Option<String>,
    status: Option<String>,
    mime: Option<String>,
    created_from: Option<String>,
    created_before: Option<String>,
    prefix: Option<String>,
    delimiter: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PathObjectListQuery {
    prefix: Option<String>,
    delimiter: Option<String>,
    limit: Option<usize>,
    cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaCursorToken {
    created_at: i64,
    id: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MediaDirectoryCursorToken {
    entry_key: String,
    is_prefix: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct ReadMediaQuery {
    token: Option<String>,
    w: Option<u32>,
    h: Option<u32>,
    fit: Option<VariantFit>,
    quality: Option<u8>,
    format: Option<VariantFormat>,
    blur: Option<u8>,
    crop: Option<CropPosition>,
    background: Option<String>,
}

impl ReadMediaQuery {
    fn transform(&self) -> Result<Option<VariantTransform>, ApiError> {
        let has_transform = self.w.is_some()
            || self.h.is_some()
            || self.fit.is_some()
            || self.quality.is_some()
            || self.format.is_some()
            || self.blur.is_some()
            || self.crop.is_some()
            || self.background.is_some();
        if !has_transform {
            return Ok(None);
        }
        VariantTransform::new(
            self.w,
            self.h,
            self.fit.unwrap_or_default(),
            self.quality.unwrap_or(80),
            self.format.unwrap_or(VariantFormat::Webp),
            self.blur.unwrap_or(0),
            self.crop.unwrap_or_default(),
            self.background.as_deref().unwrap_or("ffffff"),
        )
        .map(Some)
        .map_err(|error| ApiError::bad_request(error.to_string()))
    }
}

fn parse_read_media_query(
    query: Result<Query<ReadMediaQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<ReadMediaQuery, ApiError> {
    query.map(|Query(query)| query).map_err(|_| {
        ApiError::invalid_query("media query is invalid; format must be jpeg, png, or webp")
    })
}

fn deserialize_nullable_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

const fn default_true() -> bool {
    true
}

