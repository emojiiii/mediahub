// Router construction, request middleware, and HMAC authentication.

fn router(state: AppState, web_root: Option<PathBuf>) -> Router {
    let state = Arc::new(state);
    let cors = cors_layer(&state.cors_allowed_origins);
    let mut app = Router::new()
        .route("/health/live", get(liveness))
        .route("/health/ready", get(readiness))
        .route("/metrics", get(metrics))
        .route("/api/v1/capabilities", get(capabilities))
        .route("/api/v1/admin/users", get(admin_list_users))
        .route(
            "/api/v1/admin/users/{user_id}/status",
            patch(admin_update_user_status),
        )
        .route("/api/v1/admin/applications", get(admin_list_applications))
        .route(
            "/api/v1/admin/applications/{application_id}/quota",
            patch(admin_update_application_quota),
        )
        .route("/api/v1/admin/jobs", get(admin_list_jobs))
        .route("/api/v1/admin/storage", get(admin_storage))
        .route(
            "/api/v1/admin/settings",
            get(admin_settings).patch(admin_update_settings),
        )
        .route("/api/v1/admin/audit", get(admin_list_audit))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/verify-email", post(verify_email))
        .route(
            "/api/v1/auth/resend-verification",
            post(resend_verification),
        )
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/forgot-password", post(forgot_password))
        .route("/api/v1/auth/reset-password", post(reset_password))
        .route("/api/v1/auth/me", get(me))
        .route(
            "/api/v1/auth/sessions",
            get(list_sessions).delete(revoke_all_sessions),
        )
        .route(
            "/api/v1/auth/sessions/{session_id}",
            axum::routing::delete(revoke_session),
        )
        .route("/api/v1/me", get(me))
        .route("/api/v1/audit-logs", get(list_audit_logs))
        .route("/api/v1/webhooks", get(list_webhooks).post(create_webhook))
        .route(
            "/api/v1/webhooks/{webhook_id}",
            patch(update_webhook).delete(delete_webhook),
        )
        .route(
            "/api/v1/webhooks/{webhook_id}/deliveries",
            get(list_webhook_deliveries),
        )
        .route(
            "/api/v1/webhooks/{webhook_id}/deliveries/{event_id}/replay",
            post(replay_webhook_delivery),
        )
        .route(
            "/api/v1/applications",
            get(list_applications).post(create_application),
        )
        .route(
            "/api/v1/applications/{app_id}",
            get(get_application)
                .patch(update_application)
                .delete(delete_application),
        )
        .route(
            "/api/v1/applications/{app_id}/access-keys",
            get(list_access_keys).post(create_access_key),
        )
        .route(
            "/api/v1/access-keys/{access_key_id}",
            patch(update_access_key).delete(revoke_access_key),
        )
        .route("/api/v1/buckets", get(list_buckets).post(create_bucket))
        .route(
            "/api/v1/buckets/{name}",
            get(get_bucket).patch(update_bucket).delete(delete_bucket),
        )
        .route("/api/v1/media", get(list_media).post(upload_media))
        .route("/api/v1/media/batch", post(batch_media))
        .route(
            "/api/v1/jobs/{job_id}",
            get(get_async_job).delete(cancel_async_job),
        )
        .route("/api/v1/uploads", post(create_upload_session))
        .route(
            "/api/v1/uploads/{upload_session_id}",
            get(get_upload_session).delete(cancel_upload_session),
        )
        .route(
            "/api/v1/uploads/{upload_session_id}/content",
            put(put_upload_content),
        )
        .route(
            "/api/v1/uploads/{upload_session_id}/complete",
            post(complete_upload_session),
        )
        .route(
            "/s3/{bucket}/{*object_key}",
            get(s3_http::s3_get_object)
                .head(s3_http::s3_get_object)
                .put(s3_http::s3_put_object)
                .post(s3_http::s3_post_object)
                .delete(s3_http::s3_delete_object)
                .layer(DefaultBodyLimit::max(MAX_S3_GATEWAY_REQUEST_BYTES)),
        )
        .route(
            "/s3/{bucket}",
            get(s3_http::s3_list_objects)
                .post(s3_http::s3_bucket_post)
                .layer(DefaultBodyLimit::max(MAX_S3_GATEWAY_REQUEST_BYTES)),
        )
        .route("/{app_id}", get(list_path_buckets))
        .route(
            "/{app_id}/{bucket}",
            get(list_path_objects)
                .head(head_path_bucket)
                .put(create_path_bucket)
                .delete(delete_path_bucket),
        )
        .route(
            "/{app_id}/{bucket}/{*object_key}",
            get(read_object_content)
                .head(read_object_content)
                .put(put_path_object)
                .patch(update_path_object)
                .post(create_path_signed_url)
                .delete(delete_path_object),
        )
        .route_layer(cors)
        .route("/dav", any(webdav::handle_webdav))
        .route("/dav/", any(webdav::handle_webdav))
        .route("/dav/{*path}", any(webdav::handle_webdav));
    if let Some(web_root) = web_root {
        let index = ServeFile::new(web_root.join("index.html"));
        app = app
            .route_service("/", index.clone())
            .route_service("/login", index.clone())
            .route_service("/register", index.clone())
            .route_service("/verify-email", index.clone())
            .route_service("/forgot-password", index.clone())
            .route_service("/reset-password", index.clone())
            .route_service("/app", index.clone())
            .route_service("/app/{*path}", index.clone())
            .route_service("/admin", index.clone())
            .route_service("/admin/{*path}", index)
            .nest_service("/assets", ServeDir::new(web_root.join("assets")))
            .nest_service("/pdfjs", ServeDir::new(web_root.join("pdfjs")));
    }
    app
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            authenticate_hmac_request,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            metrics_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(middleware::from_fn(request_id_middleware))
        .with_state(state)
}

async fn metrics_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let started = Instant::now();
    let response = next.run(request).await;
    state.http_metrics.requests.fetch_add(1, Ordering::Relaxed);
    if response.status().is_client_error() || response.status().is_server_error() {
        state.http_metrics.errors.fetch_add(1, Ordering::Relaxed);
    }
    let elapsed = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX);
    state
        .http_metrics
        .duration_micros
        .fetch_add(elapsed, Ordering::Relaxed);
    response
}

async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| is_valid_request_id(value))
        .map(str::to_owned)
        .unwrap_or_else(|| format!("req_{}", uuid::Uuid::now_v7().simple()));
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));
    let response = next.run(request).await;
    attach_request_id(response, &request_id).await
}

fn is_valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

async fn attach_request_id(response: Response, request_id: &str) -> Response {
    let status = response.status();
    let (mut parts, body) = response.into_parts();
    parts.headers.insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_str(request_id).expect("validated request ID is a header value"),
    );
    if !status.is_client_error() && !status.is_server_error() {
        return Response::from_parts(parts, body);
    }
    let Ok(body) = to_bytes(body, MAX_ERROR_RESPONSE_BYTES).await else {
        return error_response_with_request_id(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "error response could not be encoded",
            request_id,
        );
    };
    let Ok(mut payload) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return Response::from_parts(parts, Body::from(body));
    };
    let Some(error) = payload
        .get_mut("error")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return Response::from_parts(parts, Body::from(body));
    };
    error.insert(
        "request_id".into(),
        serde_json::Value::String(request_id.to_owned()),
    );
    let payload = serde_json::to_vec(&payload).expect("JSON value serializes");
    parts.headers.remove(CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(payload))
}

fn error_response_with_request_id(
    status: StatusCode,
    code: &str,
    message: &str,
    request_id: &str,
) -> Response {
    let mut response = (
        status,
        Json(serde_json::json!({
            "error": { "code": code, "message": message, "request_id": request_id }
        })),
    )
        .into_response();
    response.headers_mut().insert(
        HeaderName::from_static("x-request-id"),
        HeaderValue::from_str(request_id).expect("validated request ID is a header value"),
    );
    response
}

fn cors_layer(origins: &[HeaderValue]) -> CorsLayer {
    let layer = CorsLayer::new()
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
        ])
        .allow_headers(cors_allowed_headers());
    if origins.is_empty() {
        layer
    } else {
        layer.allow_origin(origins.to_vec())
    }
}

fn cors_allowed_headers() -> [HeaderName; 10] {
    [
        axum::http::header::AUTHORIZATION,
        axum::http::header::CONTENT_TYPE,
        axum::http::header::IF_MATCH,
        HeaderName::from_static("idempotency-key"),
        HeaderName::from_static("x-csrf-token"),
        HeaderName::from_static("x-mediahub-access-key"),
        HeaderName::from_static("x-mediahub-app-id"),
        HeaderName::from_static("x-mediahub-content-sha256"),
        HeaderName::from_static("x-mediahub-date"),
        HeaderName::from_static("x-mediahub-nonce"),
    ]
}

#[derive(Clone, Debug)]
struct HmacIdentity {
    application_id: ApplicationId,
    access_key_id: String,
    permissions: Vec<String>,
}

#[derive(Clone, Debug)]
struct RequestId(String);

#[derive(Clone, Debug)]
struct HmacRequestContext {
    idempotency_key: Option<String>,
    request_hash: String,
    operation_scope: String,
}

#[derive(Clone, Debug)]
struct ApplicationAuth {
    application: ApplicationSummary,
    hmac_identity: Option<HmacIdentity>,
    actor_type: &'static str,
    actor_id: String,
}

impl ApplicationAuth {
    fn authorize(&self, permission: &str) -> Result<(), ApiError> {
        if self
            .hmac_identity
            .as_ref()
            .is_none_or(|identity| identity.permissions.iter().any(|value| value == permission))
        {
            Ok(())
        } else {
            Err(ApiError::forbidden(
                "access key lacks the required permission",
            ))
        }
    }

    async fn verify_mutation_csrf(
        &self,
        state: &AppState,
        headers: &HeaderMap,
    ) -> Result<(), ApiError> {
        if self.hmac_identity.is_none() {
            verify_csrf(state, headers).await?;
        }
        Ok(())
    }
}

async fn authenticate_hmac_request(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if request.headers().get("x-mediahub-access-key").is_none() {
        return next.run(request).await;
    }
    match validate_hmac_request(&state, request).await {
        Ok((request, identity, context)) => {
            let mut request = request;
            request.extensions_mut().insert(identity);
            request.extensions_mut().insert(context);
            next.run(request).await
        }
        Err(error) => error.into_response(),
    }
}

async fn validate_hmac_request(
    state: &AppState,
    request: Request,
) -> Result<(Request, HmacIdentity, HmacRequestContext), ApiError> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, MAX_REQUEST_BYTES)
        .await
        .map_err(|_| ApiError::bad_request("request body exceeds the supported size"))?;
    let body_sha256 = hex::encode(Sha256::digest(&body));
    let supplied_body_sha256 = header_value(&parts.headers, "x-mediahub-content-sha256")?;
    if !constant_time_eq(&body_sha256, supplied_body_sha256) {
        return Err(ApiError::unauthorized_with_message(
            "request body hash does not match",
        ));
    }
    let access_key_id = header_value(&parts.headers, "x-mediahub-access-key")?;
    let date = parse_hmac_date(header_value(&parts.headers, "x-mediahub-date")?)?;
    let nonce = parts
        .headers
        .get("x-mediahub-nonce")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let is_mutation = !matches!(parts.method.as_str(), "GET" | "HEAD" | "OPTIONS");
    if is_mutation && nonce.as_deref().is_none_or(str::is_empty) {
        return Err(ApiError::bad_request(
            "X-MediaHub-Nonce is required for mutation requests",
        ));
    }
    if nonce.as_deref().is_some_and(|value| value.len() > 256) {
        return Err(ApiError::bad_request("X-MediaHub-Nonce is too long"));
    }
    let authorization = header_value(&parts.headers, "authorization")?;
    let (signed_headers, signature) = parse_authorization(authorization)?;
    let canonical_headers = canonical_headers(&parts.headers, &signed_headers)?;
    for required in [
        "x-mediahub-access-key",
        "x-mediahub-date",
        "x-mediahub-content-sha256",
    ] {
        if !signed_headers.iter().any(|header| header == required) {
            return Err(ApiError::bad_request(
                "required HMAC headers are not signed",
            ));
        }
    }
    if is_mutation
        && !signed_headers
            .iter()
            .any(|header| header == "x-mediahub-nonce")
    {
        return Err(ApiError::bad_request("X-MediaHub-Nonce must be signed"));
    }
    let idempotency_key = parts
        .headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if idempotency_key
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.len() > 256)
    {
        return Err(ApiError::bad_request("Idempotency-Key is invalid"));
    }
    if idempotency_key.is_some()
        && !signed_headers
            .iter()
            .any(|header| header == "idempotency-key")
    {
        return Err(ApiError::bad_request("Idempotency-Key must be signed"));
    }
    let query = canonical_query(parts.uri.query());
    let idempotency_request_hash = request_hash(
        parts.method.as_str(),
        parts.uri.path(),
        &query,
        &body_sha256,
    );
    let canonical = CanonicalRequest {
        method: parts.method.to_string(),
        path: parts.uri.path().to_owned(),
        query,
        headers: canonical_headers,
        body_sha256,
        timestamp: date,
        nonce: nonce.clone().unwrap_or_default(),
        idempotency_key: idempotency_key.clone(),
    };
    let now = chrono::Utc::now();
    let access_key = state
        .repository
        .find_active_access_key(access_key_id, OffsetDateTime::now_utc())
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(ApiError::unauthorized)?;
    let secret = state
        .access_key_cipher
        .decrypt(&access_key.secret_ciphertext, access_key.secret_key_version)
        .map_err(ApiError::from_access_key_cipher)?;
    verify_hmac(&secret, signature, &canonical, now).map_err(ApiError::from_hmac)?;
    if is_mutation {
        let nonce = nonce.expect("mutation requests require a nonce");
        let expires_at =
            OffsetDateTime::from_unix_timestamp(date.timestamp() + MAX_SIGNATURE_AGE.num_seconds())
                .map_err(|_| ApiError::bad_request("X-MediaHub-Date is invalid"))?;
        state
            .repository
            .record_replay_nonce(access_key_id, &nonce, expires_at, OffsetDateTime::now_utc())
            .await
            .map_err(|error| match error {
                mediahub_app::RepositoryError::Conflict => ApiError::replay_detected(),
                error => ApiError::from_repository(error),
            })?;
    }
    Ok((
        Request::from_parts(parts, Body::from(body)),
        HmacIdentity {
            application_id: access_key.application_id,
            access_key_id: access_key.access_key_id,
            permissions: access_key.permissions,
        },
        HmacRequestContext {
            idempotency_key,
            request_hash: idempotency_request_hash,
            operation_scope: format!("{} {}", canonical.method, canonical.path),
        },
    ))
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, ApiError> {
    headers
        .get(name)
        .ok_or_else(|| ApiError::bad_request(format!("{name} is required")))?
        .to_str()
        .map_err(|_| ApiError::bad_request(format!("{name} is invalid")))
}

fn parse_hmac_date(value: &str) -> Result<chrono::DateTime<chrono::Utc>, ApiError> {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&chrono::Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ")
                .map(|value| value.and_utc())
        })
        .map_err(|_| ApiError::bad_request("X-MediaHub-Date is invalid"))
}

fn parse_authorization(value: &str) -> Result<(Vec<String>, &str), ApiError> {
    let fields = value
        .strip_prefix("MH-HMAC-SHA256 ")
        .ok_or_else(|| ApiError::unauthorized_with_message("Authorization is invalid"))?
        .split(';')
        .map(str::trim)
        .filter_map(|field| field.split_once('='))
        .collect::<BTreeMap<_, _>>();
    let signed_headers = fields
        .get("SignedHeaders")
        .ok_or_else(|| ApiError::bad_request("Authorization SignedHeaders is required"))?
        .split(',')
        .map(str::trim)
        .filter(|header| !header.is_empty())
        .map(|header| header.to_ascii_lowercase())
        .collect::<Vec<_>>();
    if signed_headers.is_empty()
        || signed_headers.iter().any(|header| {
            !header
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
    {
        return Err(ApiError::bad_request(
            "Authorization SignedHeaders is invalid",
        ));
    }
    let signature = *fields
        .get("Signature")
        .filter(|signature| !signature.is_empty())
        .ok_or_else(|| ApiError::bad_request("Authorization Signature is required"))?;
    Ok((signed_headers, signature))
}

fn canonical_headers(
    headers: &HeaderMap,
    signed_headers: &[String],
) -> Result<BTreeMap<String, String>, ApiError> {
    let mut result = BTreeMap::new();
    for header in signed_headers {
        let value = headers
            .get(header)
            .ok_or_else(|| ApiError::bad_request("a signed header is missing"))?
            .to_str()
            .map_err(|_| ApiError::bad_request("a signed header is invalid"))?;
        if result
            .insert(header.clone(), value.trim().to_owned())
            .is_some()
        {
            return Err(ApiError::bad_request(
                "Authorization SignedHeaders contains duplicates",
            ));
        }
    }
    Ok(result)
}

fn canonical_query(query: Option<&str>) -> BTreeMap<String, Vec<String>> {
    let mut result = BTreeMap::<String, Vec<String>>::new();
    for (key, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        result
            .entry(key.into_owned())
            .or_default()
            .push(value.into_owned());
    }
    result
}

fn request_hash(
    method: &str,
    path: &str,
    query: &BTreeMap<String, Vec<String>>,
    body_sha256: &str,
) -> String {
    let mut encoder = url::form_urlencoded::Serializer::new(String::new());
    for (key, values) in query {
        let mut values = values.iter().collect::<Vec<_>>();
        values.sort();
        for value in values {
            encoder.append_pair(key, value);
        }
    }
    let input = format!(
        "{}\n{path}\n{}\n{body_sha256}",
        method.to_ascii_uppercase(),
        encoder.finish()
    );
    hex::encode(Sha256::digest(input.as_bytes()))
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    use subtle::ConstantTimeEq;
    left.as_bytes().ct_eq(right.as_bytes()).into()
}

