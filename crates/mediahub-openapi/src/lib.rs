mod contract;
pub mod dto;

use anyhow::{Context, Result};
use serde_json::Value;
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "MediaHub API",
        version = "0.1.0",
        description = "Docker Profile V1 API contract. Authenticated resource ownership is always derived from the session or API key, never from client-supplied owner IDs. Every response includes X-Request-Id; JSON errors additionally expose error.request_id."
    ),
    components(schemas(
        dto::AccessKey,
        dto::AdminApplication,
        dto::AdminUpdateApplicationQuota,
        dto::AdminAudit,
        dto::AdminJob,
        dto::AdminSettings,
        dto::AdminStorage,
        dto::AdminUpdateUserStatus,
        dto::AdminUpdateSettings,
        dto::AdminUser,
        dto::Application,
        dto::AsyncJob,
        dto::AsyncJobAction,
        dto::AsyncJobDetails,
        dto::AsyncJobItemState,
        dto::AsyncJobItemResult,
        dto::AsyncJobReceipt,
        dto::AuditEvent,
        dto::AuthStatus,
        dto::BatchItemResult,
        dto::BatchMediaRequest,
        dto::BatchMediaResponse,
        dto::Bucket,
        dto::Capabilities,
        dto::CompleteUploadSession,
        dto::CompleteUploadSessionResponse,
        dto::CreateAccessKey,
        dto::CreateAccessKeyResponse,
        dto::CreateApplication,
        dto::CreateBucket,
        dto::CreateUploadSession,
        dto::CreateUploadSessionResponse,
        dto::CreateWebhook,
        dto::CreateWebhookResponse,
        dto::Credentials,
        dto::Error,
        dto::ForgotPassword,
        dto::ForgotPasswordResponse,
        dto::LifecycleRule,
        dto::Me,
        dto::Media,
        dto::MediaPage,
        dto::OneTimeToken,
        dto::Permission,
        dto::RegistrationResponse,
        dto::ResendVerificationResponse,
        dto::ResetPassword,
        dto::Session,
        dto::SignedMediaUrl,
        dto::UpdateAccessKey,
        dto::UpdateApplication,
        dto::UpdateBucket,
        dto::UpdateMedia,
        dto::UpdateWebhook,
        dto::UpdateWebhookResponse,
        dto::UploadMedia,
        dto::UploadSession,
        dto::UploadTarget,
        dto::Visibility,
        dto::Webhook,
        dto::WebhookDelivery,
        dto::WebhookDeliveryPage
    ))
)]
struct MediaHubApi;

pub fn document() -> Result<Value> {
    let mut document = serde_json::to_value(MediaHubApi::openapi())
        .context("failed to serialize utoipa OpenAPI document")?;
    document["paths"] = contract::paths();
    document["components"]["parameters"] = contract::parameters();
    document["components"]["responses"] = contract::error_responses();
    document["components"]["securitySchemes"] = contract::security_schemes();
    Ok(document)
}

pub fn to_pretty_json() -> Result<String> {
    let mut output = serde_json::to_string_pretty(&document()?)?;
    output.push('\n');
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use anyhow::{Result, anyhow, ensure};
    use serde_json::Value;

    use super::{contract, document};

    const METHODS: &[&str] = &["get", "head", "post", "put", "patch", "delete"];

    #[test]
    fn contract_has_expected_v1_surface() -> Result<()> {
        let document = document()?;
        let paths = document["paths"]
            .as_object()
            .ok_or_else(|| anyhow!("paths must be an object"))?;
        ensure!(paths.len() == 44, "expected 44 paths, got {}", paths.len());
        ensure!(
            contract::OPERATION_COUNT == 67,
            "expected 67 operations, got {}",
            contract::OPERATION_COUNT
        );
        ensure!(
            paths["/api/v1/admin/users"]["get"].is_object(),
            "Admin user list operation is missing"
        );
        ensure!(
            paths["/api/v1/admin/storage"]["get"].is_object(),
            "Admin storage operation is missing"
        );
        ensure!(
            paths["/api/v1/admin/settings"]["get"].is_object()
                && paths["/api/v1/admin/settings"]["patch"].is_object(),
            "Admin settings operations are missing"
        );
        ensure!(
            paths["/api/v1/admin/applications/{application_id}/quota"]["patch"].is_object(),
            "Admin application quota operation is missing"
        );
        ensure!(
            paths["/api/v1/uploads/{upload_session_id}"]["get"].is_object(),
            "UploadSession GET operation is missing"
        );
        ensure!(
            paths["/{app_id}/{bucket}/{object_key}"]["get"].is_object()
                && paths["/{app_id}/{bucket}/{object_key}"]["head"].is_object()
                && paths["/{app_id}/{bucket}/{object_key}"]["put"].is_object()
                && paths["/{app_id}/{bucket}/{object_key}"]["patch"].is_object()
                && paths["/{app_id}/{bucket}/{object_key}"]["post"].is_object()
                && paths["/{app_id}/{bucket}/{object_key}"]["delete"].is_object(),
            "canonical object operations are missing"
        );
        ensure!(
            !paths.contains_key("/api/v1/media/{media_id}")
                && !paths.contains_key("/api/v1/media/{media_id}/signed-url")
                && !paths.contains_key("/media/{media_id}")
                && !paths.contains_key("/public/{app_id}/{bucket}/{object_key}")
                && !paths.contains_key("/signed/{media_id}"),
            "legacy content routes must not be generated"
        );
        Ok(())
    }

    #[test]
    fn every_operation_declares_valid_security() -> Result<()> {
        let document = document()?;
        let schemes = document["components"]["securitySchemes"]
            .as_object()
            .ok_or_else(|| anyhow!("securitySchemes must be an object"))?;
        let mut operations = 0;
        for (path, item) in document["paths"]
            .as_object()
            .ok_or_else(|| anyhow!("paths must be an object"))?
        {
            for method in METHODS {
                let Some(operation) = item.get(method) else {
                    continue;
                };
                operations += 1;
                let security = operation["security"].as_array().ok_or_else(|| {
                    anyhow!("{} {} must explicitly declare security", method, path)
                })?;
                for requirement in security {
                    for name in requirement
                        .as_object()
                        .ok_or_else(|| anyhow!("security requirement must be an object"))?
                        .keys()
                    {
                        ensure!(schemes.contains_key(name), "unknown security scheme {name}");
                    }
                }
            }
        }
        ensure!(operations == 67, "expected 67 operations, got {operations}");
        Ok(())
    }

    #[test]
    fn all_local_references_resolve() -> Result<()> {
        let document = document()?;
        let mut references = BTreeSet::new();
        collect_references(&document, &mut references);
        ensure!(
            !references.is_empty(),
            "contract should contain local references"
        );
        for reference in references {
            ensure!(
                reference.starts_with("#/"),
                "only local references are allowed: {reference}"
            );
            let pointer = reference
                .strip_prefix('#')
                .ok_or_else(|| anyhow!("invalid local reference {reference}"))?;
            ensure!(
                document.pointer(pointer).is_some(),
                "unresolved reference: {reference}"
            );
        }
        Ok(())
    }

    #[test]
    fn dto_components_cover_the_documented_surface() -> Result<()> {
        let document = document()?;
        let schemas = document["components"]["schemas"]
            .as_object()
            .ok_or_else(|| anyhow!("schemas must be an object"))?;
        let surfaces: &[(&str, &[&str])] = &[
            (
                "administration",
                &[
                    "AdminApplication",
                    "AdminAudit",
                    "AdminJob",
                    "AdminSettings",
                    "AdminStorage",
                    "AdminUpdateApplicationQuota",
                    "AdminUpdateSettings",
                    "AdminUpdateUserStatus",
                    "AdminUser",
                ],
            ),
            (
                "authentication",
                &["Credentials", "Me", "RegistrationResponse", "Session"],
            ),
            (
                "control plane",
                &[
                    "AccessKey",
                    "Application",
                    "Bucket",
                    "CreateAccessKeyResponse",
                    "LifecycleRule",
                ],
            ),
            (
                "media",
                &["Media", "MediaPage", "SignedMediaUrl", "UploadMedia"],
            ),
            (
                "upload sessions",
                &[
                    "CompleteUploadSessionResponse",
                    "CreateUploadSession",
                    "CreateUploadSessionResponse",
                    "UploadSession",
                    "UploadTarget",
                ],
            ),
            (
                "asynchronous jobs",
                &[
                    "AsyncJob",
                    "AsyncJobAction",
                    "AsyncJobDetails",
                    "AsyncJobItemResult",
                    "AsyncJobItemState",
                    "AsyncJobReceipt",
                    "BatchMediaRequest",
                    "BatchMediaResponse",
                ],
            ),
            (
                "webhooks",
                &[
                    "CreateWebhookResponse",
                    "UpdateWebhookResponse",
                    "Webhook",
                    "WebhookDelivery",
                    "WebhookDeliveryPage",
                ],
            ),
        ];
        for (surface, required_schemas) in surfaces {
            for required in *required_schemas {
                ensure!(
                    schemas.contains_key(*required),
                    "{surface} surface is missing DTO schema {required}"
                );
            }
        }

        let assert_schema_ref = |schema: &Value, expected: &str, context: &str| -> Result<()> {
            let expected_ref = format!("#/components/schemas/{expected}");
            ensure!(
                schema["$ref"].as_str() == Some(expected_ref.as_str()),
                "{context} must reference {expected_ref}"
            );
            Ok(())
        };

        let batch = &document["paths"]["/api/v1/media/batch"]["post"];
        assert_schema_ref(
            &batch["requestBody"]["content"]["application/json"]["schema"],
            "BatchMediaRequest",
            "media batch request",
        )?;
        assert_schema_ref(
            &batch["responses"]["202"]["content"]["application/json"]["schema"],
            "AsyncJobReceipt",
            "media batch asynchronous response",
        )?;

        let jobs = &document["paths"]["/api/v1/jobs/{job_id}"];
        assert_schema_ref(
            &jobs["get"]["responses"]["200"]["content"]["application/json"]["schema"],
            "AsyncJobDetails",
            "async job read response",
        )?;
        assert_schema_ref(
            &jobs["delete"]["responses"]["200"]["content"]["application/json"]["schema"],
            "AsyncJob",
            "async job cancellation response",
        )?;

        let uploads = &document["paths"]["/api/v1/uploads"]["post"];
        assert_schema_ref(
            &uploads["requestBody"]["content"]["application/json"]["schema"],
            "CreateUploadSession",
            "upload session request",
        )?;
        assert_schema_ref(
            &uploads["responses"]["201"]["content"]["application/json"]["schema"],
            "CreateUploadSessionResponse",
            "upload session response",
        )?;

        let webhook_history = &document["paths"]["/api/v1/webhooks/{webhook_id}/deliveries"]["get"];
        assert_schema_ref(
            &webhook_history["responses"]["200"]["content"]["application/json"]["schema"],
            "WebhookDeliveryPage",
            "webhook delivery history response",
        )?;
        Ok(())
    }

    #[test]
    fn async_job_contract_matches_the_public_runtime_shape() -> Result<()> {
        let document = document()?;
        let job = &document["components"]["schemas"]["AsyncJob"];
        let job_properties = job["properties"]
            .as_object()
            .ok_or_else(|| anyhow!("AsyncJob properties must be an object"))?;
        for internal in [
            "lease_token",
            "leased_until",
            "idempotency_key",
            "request_hash",
        ] {
            ensure!(
                !job_properties.contains_key(internal),
                "AsyncJob must not expose internal field {internal}"
            );
        }

        let assert_required = |schema: &Value, fields: &[&str], name: &str| -> Result<()> {
            let required = schema["required"]
                .as_array()
                .ok_or_else(|| anyhow!("{name} required fields must be an array"))?;
            for field in fields {
                ensure!(
                    required.iter().any(|required| required == field),
                    "{name}.{field} is always serialized and must be required"
                );
            }
            Ok(())
        };
        let assert_nullable_datetime =
            |schema: &Value, fields: &[&str], name: &str| -> Result<()> {
                for field in fields {
                    let property = &schema["properties"][field];
                    ensure!(
                        property["type"] == serde_json::json!(["string", "null"])
                            && property["format"] == "date-time",
                        "{name}.{field} must be a nullable date-time string"
                    );
                }
                Ok(())
            };
        let assert_nullable_string = |schema: &Value, fields: &[&str], name: &str| -> Result<()> {
            for field in fields {
                ensure!(
                    schema["properties"][field]["type"] == serde_json::json!(["string", "null"]),
                    "{name}.{field} must be a nullable string"
                );
            }
            Ok(())
        };
        let assert_datetime = |schema: &Value, fields: &[&str], name: &str| -> Result<()> {
            for field in fields {
                ensure!(
                    schema["properties"][field]
                        == serde_json::json!({ "type": "string", "format": "date-time" }),
                    "{name}.{field} must be a date-time string"
                );
            }
            Ok(())
        };

        assert_required(
            job,
            &[
                "request_id",
                "next_attempt_at",
                "error_summary",
                "started_at",
                "completed_at",
                "failed_at",
                "cancelled_at",
            ],
            "AsyncJob",
        )?;
        assert_nullable_string(job, &["request_id", "error_summary"], "AsyncJob")?;
        assert_nullable_datetime(
            job,
            &[
                "next_attempt_at",
                "started_at",
                "completed_at",
                "failed_at",
                "cancelled_at",
            ],
            "AsyncJob",
        )?;
        assert_datetime(job, &["created_at", "updated_at"], "AsyncJob")?;

        let item = &document["components"]["schemas"]["AsyncJobItemResult"];
        assert_required(
            item,
            &[
                "result",
                "error_code",
                "error_summary",
                "started_at",
                "completed_at",
            ],
            "AsyncJobItemResult",
        )?;
        assert_nullable_string(item, &["error_code", "error_summary"], "AsyncJobItemResult")?;
        assert_nullable_datetime(item, &["started_at", "completed_at"], "AsyncJobItemResult")?;
        assert_datetime(item, &["updated_at"], "AsyncJobItemResult")
    }

    #[test]
    fn admin_settings_contract_is_nullable_bounded_and_csrf_protected() -> Result<()> {
        let document = document()?;
        for schema_name in ["AdminSettings", "AdminUpdateSettings"] {
            let schema = &document["components"]["schemas"][schema_name];
            let required = schema["required"]
                .as_array()
                .ok_or_else(|| anyhow!("{schema_name} required fields must be an array"))?;
            ensure!(
                required
                    .iter()
                    .any(|field| field == "download_bytes_per_second"),
                "{schema_name} must require download_bytes_per_second"
            );
            let rate = &schema["properties"]["download_bytes_per_second"];
            ensure!(
                rate["type"] == serde_json::json!(["integer", "null"]),
                "{schema_name} rate must be an integer or null"
            );
            ensure!(
                rate["minimum"] == serde_json::json!(1_048_576_u64)
                    && rate["maximum"] == serde_json::json!(1_073_741_824_u64),
                "{schema_name} rate bounds must be 1 MiB/s through 1 GiB/s"
            );
            ensure!(
                rate["description"]
                    .as_str()
                    .is_some_and(|description| description.contains("null means unlimited")),
                "{schema_name} must document null as unlimited"
            );
        }
        let response_schema = &document["components"]["schemas"]["AdminSettings"];
        ensure!(
            response_schema["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|field| field == "updated_at")),
            "AdminSettings must require updated_at"
        );
        ensure!(
            response_schema["properties"]["updated_at"]
                == serde_json::json!({ "type": "string", "format": "date-time" }),
            "AdminSettings updated_at must be a date-time string"
        );
        ensure!(
            document["components"]["schemas"]["AdminUpdateSettings"]["properties"]
                .as_object()
                .is_some_and(|properties| {
                    properties.len() == 1 && properties.contains_key("download_bytes_per_second")
                }),
            "AdminUpdateSettings must contain only download_bytes_per_second"
        );

        let settings = &document["paths"]["/api/v1/admin/settings"];
        ensure!(
            settings["get"]["security"] == serde_json::json!([{ "SessionCookie": [] }]),
            "Admin settings GET must require an Admin session cookie"
        );
        ensure!(
            settings["patch"]["security"]
                == serde_json::json!([{ "SessionCookie": [], "CsrfToken": [] }]),
            "Admin settings PATCH must require both session and CSRF"
        );
        ensure!(
            settings["patch"]["requestBody"]["content"]["application/json"]["schema"]["$ref"]
                == "#/components/schemas/AdminUpdateSettings",
            "Admin settings PATCH must use AdminUpdateSettings"
        );
        for (method, statuses) in [
            ("get", &["401", "403", "503"][..]),
            ("patch", &["400", "401", "403", "503"][..]),
        ] {
            for status in statuses {
                ensure!(
                    settings[method]["responses"][status]["$ref"]
                        .as_str()
                        .is_some_and(|reference| reference.starts_with("#/components/responses/")),
                    "Admin settings {method} {status} must use a structured error response"
                );
            }
        }
        Ok(())
    }

    #[test]
    fn upload_requests_expose_only_canonical_bucket_fields() -> Result<()> {
        let document = document()?;
        let create = &document["components"]["schemas"]["CreateUploadSession"];
        let create_properties = create["properties"]
            .as_object()
            .ok_or_else(|| anyhow!("CreateUploadSession properties must be an object"))?;
        let create_required = create["required"]
            .as_array()
            .ok_or_else(|| anyhow!("CreateUploadSession required must be an array"))?;
        for field in ["bucket", "expected_size", "content_type"] {
            ensure!(
                create_required.iter().any(|value| value == field),
                "CreateUploadSession must require {field}"
            );
        }
        ensure!(
            !create_properties.contains_key("bucket_id")
                && !create_properties.contains_key("expected_mime"),
            "CreateUploadSession must not expose legacy upload fields"
        );

        let multipart = &document["components"]["schemas"]["UploadMedia"];
        let multipart_properties = multipart["properties"]
            .as_object()
            .ok_or_else(|| anyhow!("UploadMedia properties must be an object"))?;
        ensure!(
            multipart_properties.contains_key("bucket")
                && !multipart_properties.contains_key("bucket_id"),
            "UploadMedia must expose only the canonical Bucket name"
        );
        Ok(())
    }

    #[test]
    fn variant_output_formats_are_bounded() -> Result<()> {
        let document = document()?;
        ensure!(
            document["components"]["parameters"]["VariantFormat"]["schema"]["enum"]
                == serde_json::json!(["jpeg", "png", "webp"]),
            "Variant format contract must expose only JPEG, PNG, and WebP"
        );
        Ok(())
    }

    #[test]
    fn request_parameters_match_server_names_and_bounds() -> Result<()> {
        let document = document()?;
        let application = &document["components"]["parameters"]["ApplicationContext"];
        ensure!(
            application["name"] == serde_json::json!("X-MediaHub-App-Id")
                && application["schema"] == serde_json::json!({ "type": "string" }),
            "application selector must use the server's app-id header"
        );
        ensure!(
            document["components"]["parameters"]["AdminLimit"]["schema"]
                == serde_json::json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 200,
                    "default": 100
                }),
            "admin pagination bounds must match the handler"
        );
        for name in ["VariantWidth", "VariantHeight"] {
            ensure!(
                document["components"]["parameters"][name]["schema"]["minimum"]
                    == serde_json::json!(1)
                    && document["components"]["parameters"][name]["schema"]["maximum"]
                        == serde_json::json!(4_096),
                "{name} bounds are missing"
            );
        }
        ensure!(
            document["components"]["parameters"]["ContentLength"]["schema"]
                == serde_json::json!({
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 2_147_483_648_u64
                }),
            "binary content length bounds are missing"
        );
        let binary = &document["paths"]["/api/v1/uploads/{upload_session_id}/content"]["put"]["requestBody"]
            ["content"]["application/octet-stream"]["schema"];
        ensure!(
            binary == &serde_json::json!({ "type": "string", "format": "binary" }),
            "binary upload body must use a binary string schema"
        );
        Ok(())
    }

    #[test]
    fn request_nullability_matches_server_defaults() -> Result<()> {
        let document = document()?;
        for field in ["allowed_mime_types", "lifecycle_rules"] {
            let schema = &document["components"]["schemas"]["CreateBucket"];
            ensure!(
                schema["required"]
                    .as_array()
                    .is_none_or(|required| !required.iter().any(|value| value == field)),
                "CreateBucket {field} must be optional"
            );
            ensure!(
                schema["properties"][field]["type"] == serde_json::json!("array"),
                "CreateBucket {field} must reject null and use an array"
            );
        }
        for (schema_name, field, default) in [
            ("CreateWebhook", "enabled", true),
            ("UpdateWebhook", "rotate_secret", false),
        ] {
            let schema = &document["components"]["schemas"][schema_name];
            ensure!(
                schema["required"]
                    .as_array()
                    .is_none_or(|required| !required.iter().any(|value| value == field)),
                "{schema_name}.{field} must be optional"
            );
            ensure!(
                schema["properties"][field]
                    == serde_json::json!({ "type": "boolean", "default": default }),
                "{schema_name}.{field} must be a non-null boolean default"
            );
        }
        Ok(())
    }

    fn collect_references(value: &Value, references: &mut BTreeSet<String>) {
        match value {
            Value::Array(values) => {
                for value in values {
                    collect_references(value, references);
                }
            }
            Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(Value::as_str) {
                    references.insert(reference.to_owned());
                }
                for value in object.values() {
                    collect_references(value, references);
                }
            }
            _ => {}
        }
    }
}
