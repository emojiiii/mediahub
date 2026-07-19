// OpenAPI parameter definitions.

#[derive(Clone, Copy)]
enum ParameterLocation {
    Path,
    Query,
    Header,
}

impl ParameterLocation {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Path => "path",
            Self::Query => "query",
            Self::Header => "header",
        }
    }
}

#[derive(Clone, Copy)]
enum ParameterSchema {
    String,
    Uuid,
    Integer,
}

#[derive(Clone, Copy)]
struct ParameterContract {
    component: &'static str,
    name: &'static str,
    location: ParameterLocation,
    required: bool,
    schema: ParameterSchema,
}

macro_rules! parameter {
    ($component:literal, $name:literal, $location:ident, $required:literal, $schema:ident) => {
        ParameterContract {
            component: $component,
            name: $name,
            location: ParameterLocation::$location,
            required: $required,
            schema: ParameterSchema::$schema,
        }
    };
}

const PARAMETERS: &[ParameterContract] = &[
    parameter!("IdempotencyKey", "Idempotency-Key", Header, false, String),
    parameter!(
        "BatchIdempotencyKey",
        "Idempotency-Key",
        Header,
        true,
        String
    ),
    parameter!("IfNoneMatch", "If-None-Match", Header, false, String),
    parameter!("IfMatch", "If-Match", Header, false, String),
    parameter!("Range", "Range", Header, false, String),
    parameter!("MediaId", "media_id", Path, true, Uuid),
    parameter!("JobId", "job_id", Path, true, Uuid),
    parameter!("AppId", "app_id", Path, true, String),
    parameter!("BucketName", "name", Path, true, String),
    parameter!("PublicBucketName", "bucket", Path, true, String),
    parameter!("ObjectKey", "object_key", Path, true, String),
    parameter!("AccessKeyId", "access_key_id", Path, true, String),
    parameter!("SessionId", "session_id", Path, true, String),
    parameter!("WebhookId", "webhook_id", Path, true, String),
    parameter!("UploadSessionId", "upload_session_id", Path, true, Uuid),
    parameter!("UserId", "user_id", Path, true, Uuid),
    parameter!("ApplicationId", "application_id", Path, true, Uuid),
    parameter!("EventId", "event_id", Path, true, String),
    parameter!("AdminLimit", "limit", Query, false, Integer),
    parameter!(
        "ApplicationContext",
        "X-Application-Id",
        Header,
        false,
        Uuid
    ),
    parameter!("DeliveryStatus", "status", Query, false, String),
    parameter!("MediaBucket", "bucket", Query, false, String),
    parameter!("MediaStatus", "status", Query, false, String),
    parameter!("MediaMime", "mime", Query, false, String),
    parameter!("CreatedFrom", "created_from", Query, false, String),
    parameter!("CreatedBefore", "created_before", Query, false, String),
    parameter!("ObjectPrefix", "prefix", Query, false, String),
    parameter!("Delimiter", "delimiter", Query, false, String),
    parameter!("Limit", "limit", Query, false, Integer),
    parameter!("Cursor", "cursor", Query, false, String),
    parameter!("ContentLength", "Content-Length", Header, true, Integer),
    parameter!("ContentType", "Content-Type", Header, true, String),
    parameter!("VariantWidth", "w", Query, false, Integer),
    parameter!("VariantHeight", "h", Query, false, Integer),
    parameter!("VariantFit", "fit", Query, false, String),
    parameter!("VariantQuality", "quality", Query, false, Integer),
    parameter!("VariantFormat", "format", Query, false, String),
    parameter!("VariantBlur", "blur", Query, false, Integer),
    parameter!("VariantCrop", "crop", Query, false, String),
    parameter!("VariantBackground", "background", Query, false, String),
];

pub fn parameters() -> Value {
    Value::Object(
        PARAMETERS
            .iter()
            .map(|parameter| {
                let mut schema = match parameter.schema {
                    ParameterSchema::String => json!({ "type": "string" }),
                    ParameterSchema::Uuid => json!({ "type": "string", "format": "uuid" }),
                    ParameterSchema::Integer => json!({ "type": "integer" }),
                };
                if parameter.component == "Limit" || parameter.component == "AdminLimit" {
                    schema["minimum"] = json!(1);
                    schema["maximum"] = json!(100);
                    schema["default"] = json!(50);
                }
                if parameter.component == "VariantFormat" {
                    schema["enum"] = json!(["jpeg", "png", "webp"]);
                    schema["default"] = json!("webp");
                }
                (
                    parameter.component.into(),
                    json!({
                        "name": parameter.name,
                        "in": parameter.location.as_str(),
                        "required": parameter.required,
                        "schema": schema
                    }),
                )
            })
            .collect(),
    )
}

