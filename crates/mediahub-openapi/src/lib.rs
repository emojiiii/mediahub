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
        ensure!(
            schemas.len() == 61,
            "expected 61 public DTO schemas, got {}",
            schemas.len()
        );
        for required in [
            "AdminSettings",
            "AdminUpdateSettings",
            "AdminUser",
            "AdminUpdateApplicationQuota",
            "AsyncJobDetails",
            "Bucket",
            "CreateUploadSession",
            "Error",
            "LifecycleRule",
            "Media",
            "UploadSession",
            "WebhookDeliveryPage",
        ] {
            ensure!(
                schemas.contains_key(required),
                "missing DTO schema {required}"
            );
        }
        Ok(())
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
