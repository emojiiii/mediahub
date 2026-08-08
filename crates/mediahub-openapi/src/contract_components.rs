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

pub fn s3_identity_policy_schemas() -> [(&'static str, Value); 2] {
    let string_or_strings = || {
        json!({
            "oneOf": [
                { "type": "string", "minLength": 1, "maxLength": 2048 },
                {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 64,
                    "items": { "type": "string", "minLength": 1, "maxLength": 2048 }
                }
            ]
        })
    };
    let statement = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["Effect"],
        "description": "AWS-style identity statement. The server additionally requires exactly one of Action/NotAction and one of Resource/NotResource, and rejects Principal fields.",
        "properties": {
            "Sid": { "type": "string", "maxLength": 128 },
            "Effect": { "type": "string", "enum": ["Allow", "Deny"] },
            "Action": string_or_strings(),
            "NotAction": string_or_strings(),
            "Resource": string_or_strings(),
            "NotResource": string_or_strings(),
            "Condition": {
                "type": "object",
                "description": "AWS condition operator/key/value map. Supported operators and keys are validated strictly by the server.",
                "additionalProperties": { "type": "object", "additionalProperties": true }
            }
        }
    });
    let policy = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["Version", "Statement"],
        "description": "S3 Identity Policy attached to one Access Key. Missing policy means implicit deny; legacy permissions are never converted or used as fallback. Maximum encoded request size is 20 KiB.",
        "properties": {
            "Version": { "type": "string", "enum": ["2012-10-17", "2008-10-17"] },
            "Id": { "type": "string", "maxLength": 2048 },
            "Statement": {
                "oneOf": [
                    { "$ref": "#/components/schemas/S3IdentityPolicyStatement" },
                    {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 100,
                        "items": { "$ref": "#/components/schemas/S3IdentityPolicyStatement" }
                    }
                ]
            }
        }
    });
    [
        ("S3IdentityPolicy", policy),
        ("S3IdentityPolicyStatement", statement),
    ]
}

#[cfg(test)]
pub const OPERATION_COUNT: usize = OPERATIONS.len();
