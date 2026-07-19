// OpenAPI error and security components.

pub fn error_responses() -> Value {
    let responses = [
        ("InvalidRequest", "Invalid request"),
        ("Unauthorized", "Authentication required"),
        ("Forbidden", "Permission denied"),
        ("NotFound", "Resource not found"),
        ("Conflict", "State or idempotency conflict"),
        ("PayloadTooLarge", "Payload too large"),
        ("UnsupportedMediaType", "Unsupported media type"),
        (
            "UnprocessableContent",
            "Content or policy validation failed",
        ),
        ("RateLimited", "Rate limit exceeded"),
        ("Unavailable", "Dependency unavailable"),
    ];
    Value::Object(
        responses
            .into_iter()
            .map(|(name, description)| {
                (
                    name.into(),
                    json!({
                        "description": description,
                        "content": { "application/json": { "schema": schema_ref("Error") } }
                    }),
                )
            })
            .collect(),
    )
}

pub fn security_schemes() -> Value {
    json!({
        "SessionCookie": {
            "type": "apiKey", "in": "cookie", "name": "mediahub_session"
        },
        "CsrfToken": {
            "type": "apiKey", "in": "header", "name": "X-CSRF-Token"
        },
        "HmacAccessKey": {
            "type": "apiKey", "in": "header", "name": "Authorization",
            "description": "MediaHub-HMAC-SHA256 signed request with timestamp, nonce, body hash, and signed headers."
        },
        "MetricsBearer": {
            "type": "http", "scheme": "bearer",
            "description": "Dedicated deployment metrics bearer token."
        },
        "UploadCapability": {
            "type": "apiKey", "in": "query", "name": "token",
            "description": "Short-lived capability bound to PUT and one upload session."
        },
        "SignedMediaToken": {
            "type": "apiKey", "in": "query", "name": "token",
            "description": "Short-lived token bound to one media revision and response policy."
        }
    })
}

#[cfg(test)]
pub const OPERATION_COUNT: usize = OPERATIONS.len();
