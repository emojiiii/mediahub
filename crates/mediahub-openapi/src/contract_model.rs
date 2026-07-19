// OpenAPI contract model types and response/request helpers.

#[derive(Clone, Copy)]
enum Method {
    Get,
    Head,
    Post,
    Put,
    Patch,
    Delete,
}

impl Method {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "get",
            Self::Head => "head",
            Self::Post => "post",
            Self::Put => "put",
            Self::Patch => "patch",
            Self::Delete => "delete",
        }
    }
}

#[derive(Clone, Copy)]
enum AuthPolicy {
    Public,
    Session,
    SessionCsrf,
    SessionOrHmac,
    SessionCsrfOrHmac,
    Metrics,
    UploadCapability,
    ObjectContent,
}

impl AuthPolicy {
    fn security(self) -> Value {
        match self {
            Self::Public => json!([]),
            Self::Session => json!([{ "SessionCookie": [] }]),
            Self::SessionCsrf => json!([{ "SessionCookie": [], "CsrfToken": [] }]),
            Self::SessionOrHmac => {
                json!([{ "SessionCookie": [] }, { "HmacAccessKey": [] }])
            }
            Self::SessionCsrfOrHmac => json!([
                { "SessionCookie": [], "CsrfToken": [] },
                { "HmacAccessKey": [] }
            ]),
            Self::Metrics => {
                json!([{ "SessionCookie": [] }, { "MetricsBearer": [] }])
            }
            Self::UploadCapability => json!([{ "UploadCapability": [] }]),
            Self::ObjectContent => {
                json!([{}, { "SignedMediaToken": [] }, { "HmacAccessKey": [] }])
            }
        }
    }
}

#[derive(Clone, Copy)]
enum RequestBody {
    Json(&'static str),
    Multipart(&'static str),
    Binary,
}

#[derive(Clone, Copy)]
enum ResponseBody {
    Empty,
    Json(&'static str),
    JsonArray(&'static str),
    Text,
    Binary,
    Standard(&'static str),
}

#[derive(Clone, Copy)]
struct ResponseContract {
    status: &'static str,
    body: ResponseBody,
}

impl ResponseContract {
    const fn empty(status: &'static str) -> Self {
        Self {
            status,
            body: ResponseBody::Empty,
        }
    }

    const fn json(status: &'static str, schema: &'static str) -> Self {
        Self {
            status,
            body: ResponseBody::Json(schema),
        }
    }

    const fn array(status: &'static str, schema: &'static str) -> Self {
        Self {
            status,
            body: ResponseBody::JsonArray(schema),
        }
    }

    const fn standard(status: &'static str, response: &'static str) -> Self {
        Self {
            status,
            body: ResponseBody::Standard(response),
        }
    }
}

#[derive(Clone, Copy)]
struct OperationContract {
    method: Method,
    path: &'static str,
    summary: &'static str,
    auth: AuthPolicy,
    parameters: &'static [&'static str],
    request: Option<RequestBody>,
    responses: &'static [ResponseContract],
}

const INVALID: ResponseContract = ResponseContract::standard("400", "InvalidRequest");
const UNAUTHORIZED: ResponseContract = ResponseContract::standard("401", "Unauthorized");
const FORBIDDEN: ResponseContract = ResponseContract::standard("403", "Forbidden");
const NOT_FOUND: ResponseContract = ResponseContract::standard("404", "NotFound");
const CONFLICT: ResponseContract = ResponseContract::standard("409", "Conflict");
const TOO_LARGE: ResponseContract = ResponseContract::standard("413", "PayloadTooLarge");
const UNSUPPORTED: ResponseContract = ResponseContract::standard("415", "UnsupportedMediaType");
const UNPROCESSABLE: ResponseContract = ResponseContract::standard("422", "UnprocessableContent");
const RATE_LIMITED: ResponseContract = ResponseContract::standard("429", "RateLimited");
const UNAVAILABLE: ResponseContract = ResponseContract::standard("503", "Unavailable");

macro_rules! op {
    ($method:ident $path:literal, $summary:literal, $auth:ident, [$($parameter:literal),*], $request:expr, [$($response:expr),* $(,)?]) => {
        OperationContract {
            method: Method::$method,
            path: $path,
            summary: $summary,
            auth: AuthPolicy::$auth,
            parameters: &[$($parameter),*],
            request: $request,
            responses: &[$($response),*],
        }
    };
}

const OPERATIONS: &[OperationContract] = &[
    op!(Get "/health/live", "Process liveness", Public, [], None, [ResponseContract::empty("200")]),
    op!(Get "/health/ready", "Dependency readiness", Public, [], None, [ResponseContract::empty("200"), UNAVAILABLE]),
    op!(Get "/metrics", "Read Prometheus deployment metrics", Metrics, [], None, [ResponseContract { status: "200", body: ResponseBody::Text }, UNAUTHORIZED, FORBIDDEN, UNAVAILABLE]),
    op!(Get "/api/v1/capabilities", "Read deployment capabilities", Public, [], None, [ResponseContract::json("200", "Capabilities")]),
    op!(Get "/api/v1/admin/users", "List users for system administration", Session, ["AdminLimit"], None, [ResponseContract::array("200", "AdminUser"), INVALID, UNAUTHORIZED, FORBIDDEN, UNAVAILABLE]),
    op!(Patch "/api/v1/admin/users/{user_id}/status", "Suspend or reactivate a user", SessionCsrf, ["UserId"], Some(RequestBody::Json("AdminUpdateUserStatus")), [ResponseContract::json("200", "AdminUser"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT, UNAVAILABLE]),
    op!(Get "/api/v1/admin/applications", "List applications for system administration", Session, ["AdminLimit"], None, [ResponseContract::array("200", "AdminApplication"), INVALID, UNAUTHORIZED, FORBIDDEN, UNAVAILABLE]),
    op!(Patch "/api/v1/admin/applications/{application_id}/quota", "Change an application storage quota", SessionCsrf, ["ApplicationId"], Some(RequestBody::Json("AdminUpdateApplicationQuota")), [ResponseContract::json("200", "AdminApplication"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT, UNAVAILABLE]),
    op!(Get "/api/v1/admin/jobs", "List jobs for system administration", Session, ["AdminLimit"], None, [ResponseContract::array("200", "AdminJob"), INVALID, UNAUTHORIZED, FORBIDDEN, UNAVAILABLE]),
    op!(Get "/api/v1/admin/storage", "Read global storage totals", Session, [], None, [ResponseContract::json("200", "AdminStorage"), UNAUTHORIZED, FORBIDDEN, UNAVAILABLE]),
    op!(Get "/api/v1/admin/settings", "Read deployment download settings", Session, [], None, [ResponseContract::json("200", "AdminSettings"), UNAUTHORIZED, FORBIDDEN, UNAVAILABLE]),
    op!(Patch "/api/v1/admin/settings", "Update deployment download settings", SessionCsrf, [], Some(RequestBody::Json("AdminUpdateSettings")), [ResponseContract::json("200", "AdminSettings"), INVALID, UNAUTHORIZED, FORBIDDEN, UNAVAILABLE]),
    op!(Get "/api/v1/admin/audit", "List deployment audit events", Session, ["AdminLimit"], None, [ResponseContract::array("200", "AdminAudit"), INVALID, UNAUTHORIZED, FORBIDDEN, UNAVAILABLE]),
    op!(Post "/api/v1/auth/register", "Register a user", Public, [], Some(RequestBody::Json("Credentials")), [ResponseContract::json("201", "RegistrationResponse"), INVALID, ResponseContract::json("403", "Error"), CONFLICT, RATE_LIMITED]),
    op!(Post "/api/v1/auth/verify-email", "Verify an email address", Public, [], Some(RequestBody::Json("OneTimeToken")), [ResponseContract::json("200", "AuthStatus"), INVALID, RATE_LIMITED]),
    op!(Post "/api/v1/auth/resend-verification", "Resend email verification", Public, [], Some(RequestBody::Json("ForgotPassword")), [ResponseContract::json("202", "ResendVerificationResponse"), RATE_LIMITED]),
    op!(Post "/api/v1/auth/login", "Create a session", Public, [], Some(RequestBody::Json("Credentials")), [ResponseContract::json("200", "Me"), UNAUTHORIZED, RATE_LIMITED]),
    op!(Post "/api/v1/auth/logout", "Revoke the current session", SessionCsrf, [], None, [ResponseContract::empty("204"), UNAUTHORIZED, FORBIDDEN]),
    op!(Post "/api/v1/auth/forgot-password", "Request a password reset", Public, [], Some(RequestBody::Json("ForgotPassword")), [ResponseContract::json("202", "ForgotPasswordResponse"), RATE_LIMITED]),
    op!(Post "/api/v1/auth/reset-password", "Reset a password", Public, [], Some(RequestBody::Json("ResetPassword")), [ResponseContract::empty("204"), INVALID, RATE_LIMITED]),
    op!(Get "/api/v1/auth/me", "Read the current identity", Session, [], None, [ResponseContract::json("200", "Me"), UNAUTHORIZED]),
    op!(Get "/api/v1/auth/sessions", "List active sessions", Session, [], None, [ResponseContract::array("200", "Session"), UNAUTHORIZED]),
    op!(Delete "/api/v1/auth/sessions", "Revoke all sessions", SessionCsrf, [], None, [ResponseContract::empty("204"), UNAUTHORIZED, FORBIDDEN]),
    op!(Delete "/api/v1/auth/sessions/{session_id}", "Revoke one session", SessionCsrf, ["SessionId"], None, [ResponseContract::empty("204"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Get "/api/v1/me", "Read the current identity", Session, [], None, [ResponseContract::json("200", "Me"), UNAUTHORIZED]),
    op!(Get "/api/v1/audit-logs", "List application audit events", SessionOrHmac, ["ApplicationContext"], None, [ResponseContract::array("200", "AuditEvent"), UNAUTHORIZED, FORBIDDEN]),
    op!(Get "/api/v1/webhooks", "List webhook endpoints", SessionOrHmac, ["ApplicationContext"], None, [ResponseContract::array("200", "Webhook"), UNAUTHORIZED, FORBIDDEN]),
    op!(Post "/api/v1/webhooks", "Create a webhook endpoint", SessionCsrfOrHmac, ["ApplicationContext"], Some(RequestBody::Json("CreateWebhook")), [ResponseContract::json("201", "CreateWebhookResponse"), INVALID, UNAUTHORIZED, FORBIDDEN]),
    op!(Patch "/api/v1/webhooks/{webhook_id}", "Update a webhook endpoint", SessionCsrfOrHmac, ["ApplicationContext", "WebhookId"], Some(RequestBody::Json("UpdateWebhook")), [ResponseContract::json("200", "UpdateWebhookResponse"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Delete "/api/v1/webhooks/{webhook_id}", "Delete a webhook endpoint", SessionCsrfOrHmac, ["ApplicationContext", "WebhookId"], None, [ResponseContract::empty("204"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Get "/api/v1/webhooks/{webhook_id}/deliveries", "List webhook delivery history", SessionOrHmac, ["ApplicationContext", "WebhookId", "DeliveryStatus", "Limit", "Cursor"], None, [ResponseContract::json("200", "WebhookDeliveryPage"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Post "/api/v1/webhooks/{webhook_id}/deliveries/{event_id}/replay", "Replay a terminal webhook delivery", SessionCsrfOrHmac, ["ApplicationContext", "WebhookId", "EventId"], None, [ResponseContract::empty("202"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT]),
    op!(Get "/api/v1/applications", "List applications", SessionOrHmac, [], None, [ResponseContract::array("200", "Application"), UNAUTHORIZED, FORBIDDEN]),
    op!(Post "/api/v1/applications", "Create an application", SessionCsrf, [], Some(RequestBody::Json("CreateApplication")), [ResponseContract::json("201", "Application"), INVALID, UNAUTHORIZED, FORBIDDEN]),
    op!(Get "/api/v1/applications/{app_id}", "Read an application", SessionOrHmac, ["AppId"], None, [ResponseContract::json("200", "Application"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Patch "/api/v1/applications/{app_id}", "Update an application", SessionCsrf, ["AppId"], Some(RequestBody::Json("UpdateApplication")), [ResponseContract::json("200", "Application"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Delete "/api/v1/applications/{app_id}", "Delete an application", SessionCsrf, ["AppId"], None, [ResponseContract::empty("204"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT]),
    op!(Get "/api/v1/buckets", "List buckets", SessionOrHmac, ["ApplicationContext"], None, [ResponseContract::array("200", "Bucket"), UNAUTHORIZED]),
    op!(Post "/api/v1/buckets", "Create a bucket", SessionCsrfOrHmac, ["ApplicationContext", "IdempotencyKey"], Some(RequestBody::Json("CreateBucket")), [ResponseContract::json("201", "Bucket"), ResponseContract::empty("202"), CONFLICT]),
    op!(Get "/api/v1/buckets/{name}", "Read a bucket", SessionOrHmac, ["ApplicationContext", "BucketName"], None, [ResponseContract::json("200", "Bucket"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Patch "/api/v1/buckets/{name}", "Update a bucket", SessionCsrfOrHmac, ["ApplicationContext", "BucketName"], Some(RequestBody::Json("UpdateBucket")), [ResponseContract::json("200", "Bucket"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Delete "/api/v1/buckets/{name}", "Delete a bucket", SessionCsrfOrHmac, ["ApplicationContext", "BucketName"], None, [ResponseContract::empty("204"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT]),
    op!(Get "/api/v1/media", "List media or Bucket-scoped virtual directories with a stable cursor", SessionOrHmac, ["ApplicationContext", "MediaBucket", "MediaStatus", "MediaMime", "CreatedFrom", "CreatedBefore", "ObjectPrefix", "Delimiter", "Limit", "Cursor"], None, [ResponseContract::json("200", "MediaPage"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Post "/api/v1/media", "Upload media with multipart form data", SessionCsrfOrHmac, ["ApplicationContext"], Some(RequestBody::Multipart("UploadMedia")), [ResponseContract::json("201", "Media"), CONFLICT, TOO_LARGE]),
    op!(Post "/api/v1/uploads", "Create an upload session", SessionCsrfOrHmac, ["ApplicationContext"], Some(RequestBody::Json("CreateUploadSession")), [ResponseContract::json("201", "CreateUploadSessionResponse"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT, TOO_LARGE, UNSUPPORTED, UNAVAILABLE]),
    op!(Get "/api/v1/uploads/{upload_session_id}", "Read an upload session", SessionOrHmac, ["ApplicationContext", "UploadSessionId"], None, [ResponseContract::json("200", "UploadSession"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND, UNAVAILABLE]),
    op!(Delete "/api/v1/uploads/{upload_session_id}", "Cancel an upload session", SessionCsrfOrHmac, ["ApplicationContext", "UploadSessionId"], None, [ResponseContract::empty("204"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT, UNAVAILABLE]),
    op!(Put "/api/v1/uploads/{upload_session_id}/content", "Upload content using a short-lived capability", UploadCapability, ["UploadSessionId", "ContentLength", "ContentType"], Some(RequestBody::Binary), [ResponseContract::empty("204"), INVALID, NOT_FOUND, CONFLICT, TOO_LARGE, UNSUPPORTED, UNPROCESSABLE, UNAVAILABLE]),
    op!(Post "/api/v1/uploads/{upload_session_id}/complete", "Complete an upload session", SessionCsrfOrHmac, ["ApplicationContext", "UploadSessionId"], Some(RequestBody::Json("CompleteUploadSession")), [ResponseContract::json("200", "CompleteUploadSessionResponse"), ResponseContract::json("201", "CompleteUploadSessionResponse"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT, UNPROCESSABLE, UNAVAILABLE]),
    op!(Post "/api/v1/media/batch", "Run or schedule a media batch operation", SessionCsrfOrHmac, ["ApplicationContext", "BatchIdempotencyKey"], Some(RequestBody::Json("BatchMediaRequest")), [ResponseContract::json("200", "BatchMediaResponse"), ResponseContract::json("202", "AsyncJobReceipt"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT, UNAVAILABLE]),
    op!(Get "/api/v1/jobs/{job_id}", "Read an asynchronous job", SessionOrHmac, ["ApplicationContext", "JobId"], None, [ResponseContract::json("200", "AsyncJobDetails"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Delete "/api/v1/jobs/{job_id}", "Cancel an asynchronous job", SessionCsrfOrHmac, ["ApplicationContext", "JobId"], None, [ResponseContract::json("200", "AsyncJob"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT]),
    op!(Get "/api/v1/applications/{app_id}/access-keys", "List access keys", Session, ["AppId"], None, [ResponseContract::array("200", "AccessKey")]),
    op!(Post "/api/v1/applications/{app_id}/access-keys", "Create an access key", SessionCsrf, ["AppId"], Some(RequestBody::Json("CreateAccessKey")), [ResponseContract::json("201", "CreateAccessKeyResponse")]),
    op!(Patch "/api/v1/access-keys/{access_key_id}", "Update an access key", SessionCsrf, ["AccessKeyId"], Some(RequestBody::Json("UpdateAccessKey")), [ResponseContract::json("200", "AccessKey")]),
    op!(Delete "/api/v1/access-keys/{access_key_id}", "Revoke an access key", SessionCsrf, ["AccessKeyId"], None, [ResponseContract::empty("204")]),
    op!(Get "/{app_id}", "List Buckets by Application path", SessionOrHmac, ["AppId"], None, [ResponseContract::array("200", "Bucket"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Get "/{app_id}/{bucket}", "List objects or virtual directories by Bucket path", SessionOrHmac, ["AppId", "PublicBucketName", "ObjectPrefix", "Delimiter", "Limit", "Cursor"], None, [ResponseContract::json("200", "MediaPage"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Head "/{app_id}/{bucket}", "Check a Bucket by path", SessionOrHmac, ["AppId", "PublicBucketName"], None, [ResponseContract::empty("200"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Put "/{app_id}/{bucket}", "Create a private Bucket by path", SessionCsrfOrHmac, ["AppId", "PublicBucketName"], None, [ResponseContract::json("200", "Bucket"), ResponseContract::json("201", "Bucket"), INVALID, UNAUTHORIZED, FORBIDDEN, CONFLICT]),
    op!(Delete "/{app_id}/{bucket}", "Delete an empty Bucket by path", SessionCsrfOrHmac, ["AppId", "PublicBucketName"], None, [ResponseContract::empty("204"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT]),
    op!(Get "/{app_id}/{bucket}/{object_key}", "Read object content by Application, Bucket, and Object Key", ObjectContent, ["AppId", "PublicBucketName", "ObjectKey", "Range", "IfNoneMatch", "VariantWidth", "VariantHeight", "VariantFit", "VariantQuality", "VariantFormat", "VariantBlur", "VariantCrop", "VariantBackground"], None, [ResponseContract { status: "200", body: ResponseBody::Binary }, ResponseContract { status: "206", body: ResponseBody::Binary }, ResponseContract::empty("304"), INVALID, NOT_FOUND, TOO_LARGE, UNSUPPORTED, ResponseContract::empty("416"), UNPROCESSABLE]),
    op!(Head "/{app_id}/{bucket}/{object_key}", "Read object headers by Application, Bucket, and Object Key", ObjectContent, ["AppId", "PublicBucketName", "ObjectKey", "Range", "IfNoneMatch", "VariantWidth", "VariantHeight", "VariantFit", "VariantQuality", "VariantFormat", "VariantBlur", "VariantCrop", "VariantBackground"], None, [ResponseContract::empty("200"), ResponseContract::empty("206"), ResponseContract::empty("304"), INVALID, NOT_FOUND, TOO_LARGE, UNSUPPORTED, ResponseContract::empty("416"), UNPROCESSABLE]),
    op!(Put "/{app_id}/{bucket}/{object_key}", "Create immutable object content by path", SessionCsrfOrHmac, ["AppId", "PublicBucketName", "ObjectKey", "ContentLength", "ContentType"], Some(RequestBody::Binary), [ResponseContract::empty("201"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT, TOO_LARGE, UNSUPPORTED, UNAVAILABLE]),
    op!(Patch "/{app_id}/{bucket}/{object_key}", "Update object metadata by path", SessionCsrfOrHmac, ["AppId", "PublicBucketName", "ObjectKey", "IfMatch"], Some(RequestBody::Json("UpdateMedia")), [ResponseContract::json("200", "Media"), INVALID, UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT]),
    op!(Post "/{app_id}/{bucket}/{object_key}", "Create a signed object URL by path", SessionCsrfOrHmac, ["AppId", "PublicBucketName", "ObjectKey"], None, [ResponseContract::json("200", "SignedMediaUrl"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND]),
    op!(Delete "/{app_id}/{bucket}/{object_key}", "Schedule object deletion by path", SessionCsrfOrHmac, ["AppId", "PublicBucketName", "ObjectKey"], None, [ResponseContract::empty("202"), UNAUTHORIZED, FORBIDDEN, NOT_FOUND, CONFLICT]),
];

fn response_description(status: &str) -> &'static str {
    match status {
        "200" => "Successful response",
        "201" => "Resource created",
        "202" => "Request accepted",
        "204" => "Successful response with no body",
        "206" => "Partial content",
        "304" => "Not modified",
        "400" => "Invalid request",
        "401" => "Authentication required",
        "403" => "Permission denied",
        "404" => "Resource not found",
        "409" => "State or idempotency conflict",
        "413" => "Payload too large",
        "415" => "Unsupported media type",
        "416" => "Range not satisfiable",
        "422" => "Content or policy validation failed",
        "429" => "Rate limit exceeded",
        "503" => "Dependency unavailable",
        _ => "Response",
    }
}

fn schema_ref(name: &str) -> Value {
    json!({ "$ref": format!("#/components/schemas/{name}") })
}

fn response_value(response: ResponseContract) -> Value {
    match response.body {
        ResponseBody::Standard(name) => {
            json!({ "$ref": format!("#/components/responses/{name}") })
        }
        ResponseBody::Empty => json!({ "description": response_description(response.status) }),
        ResponseBody::Json(schema) => json!({
            "description": response_description(response.status),
            "content": { "application/json": { "schema": schema_ref(schema) } }
        }),
        ResponseBody::JsonArray(schema) => json!({
            "description": response_description(response.status),
            "content": { "application/json": { "schema": {
                "type": "array", "items": schema_ref(schema)
            } } }
        }),
        ResponseBody::Text => json!({
            "description": response_description(response.status),
            "content": { "text/plain": { "schema": { "type": "string" } } }
        }),
        ResponseBody::Binary => json!({
            "description": response_description(response.status),
            "content": { "application/octet-stream": { "schema": {
                "type": "string", "format": "binary"
            } } }
        }),
    }
}

fn request_value(body: RequestBody) -> Value {
    let (content_type, schema) = match body {
        RequestBody::Json(schema) => ("application/json", schema_ref(schema)),
        RequestBody::Multipart(schema) => ("multipart/form-data", schema_ref(schema)),
        RequestBody::Binary => (
            "application/octet-stream",
            json!({ "type": "string", "format": "binary" }),
        ),
    };
    json!({
        "required": true,
        "content": { content_type: { "schema": schema } }
    })
}

