// Access-key and session helpers.

async fn list_access_keys(
    State(state): State<Arc<AppState>>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Vec<AccessKeyResponse>>, ApiError> {
    let application = owned_application_by_app_id(&state, &headers, &app_id).await?;
    let keys = state
        .repository
        .list_access_keys(application.id)
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(
        keys.into_iter().map(AccessKeyResponse::from).collect(),
    ))
}

async fn create_access_key(
    State(state): State<Arc<AppState>>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    Json(request): Json<CreateAccessKeyRequest>,
) -> Result<(StatusCode, Json<CreateAccessKeyResponse>), ApiError> {
    verify_csrf(&state, &headers).await?;
    let user = authenticate(&state, &headers).await?;
    let application = state
        .repository
        .application_for_user_by_app_id(user.id, &app_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("application not found"))?;
    let name = validate_access_key_name(request.name)?;
    let permissions = validate_permissions(request.permissions)?;
    let audit_summary = serde_json::json!({
        "name": name,
        "permissions": permissions,
    });
    let expires_at = expiration_from_request(request.expires_at, OffsetDateTime::now_utc())?;
    let secret_access_key = generate_secret();
    let encrypted_secret = state
        .access_key_cipher
        .encrypt(secret_access_key.as_bytes())
        .map_err(ApiError::from_access_key_cipher)?;
    let access_key_id = format!("mh_ak_{}", uuid::Uuid::now_v7().simple());
    let secret_last_four = secret_access_key
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    let now = OffsetDateTime::now_utc();
    state
        .repository
        .create_access_key(&NewAccessKey {
            id: uuid::Uuid::now_v7().to_string(),
            application_id: application.id,
            access_key_id: access_key_id.clone(),
            secret_ciphertext: encrypted_secret,
            secret_key_version: state.access_key_cipher.version(),
            secret_last_four,
            name: name.clone(),
            permissions: permissions.clone(),
            expires_at,
            created_at: now,
        })
        .await
        .map_err(ApiError::from_repository)?;
    record_session_audit(
        &state,
        user.id,
        &request_id.0.0,
        SessionAudit {
            application_id: application.id,
            action: "access_key.created",
            target_type: "access_key",
            target_id: access_key_id.clone(),
            summary: audit_summary,
        },
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(CreateAccessKeyResponse {
            app_id: application.app_id,
            access_key_id,
            secret_access_key,
            expires_at,
        }),
    ))
}

async fn update_access_key(
    State(state): State<Arc<AppState>>,
    Path(access_key_id): Path<String>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    Json(request): Json<UpdateAccessKeyRequest>,
) -> Result<Json<AccessKeyResponse>, ApiError> {
    verify_csrf(&state, &headers).await?;
    let user = authenticate(&state, &headers).await?;
    let access_key = state
        .repository
        .find_access_key(&access_key_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("access key not found"))?;
    let application = state
        .repository
        .application_for_user_by_id(user.id, access_key.application_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("access key not found"))?;
    if access_key.revoked_at.is_some() {
        return Err(ApiError::conflict("access key is revoked"));
    }
    let name = request
        .name
        .map(validate_access_key_name)
        .transpose()?
        .unwrap_or(access_key.name);
    let permissions = request
        .permissions
        .map(validate_permissions)
        .transpose()?
        .unwrap_or(access_key.permissions);
    let expires_at = match request.expires_at {
        Some(value) => expiration_from_request(value, OffsetDateTime::now_utc())?,
        None => access_key.expires_at,
    };
    let updated = state
        .repository
        .update_access_key(
            &access_key_id,
            application.id,
            &name,
            &permissions,
            expires_at,
        )
        .await
        .map_err(ApiError::from_repository)?;
    if !updated {
        return Err(ApiError::not_found("access key not found"));
    }
    let access_key = state
        .repository
        .find_access_key(&access_key_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("access key not found"))?;
    record_session_audit(
        &state,
        user.id,
        &request_id.0.0,
        SessionAudit {
            application_id: application.id,
            action: "access_key.updated",
            target_type: "access_key",
            target_id: access_key.access_key_id.clone(),
            summary: serde_json::json!({ "name": access_key.name, "permissions": access_key.permissions, "expires_at": access_key.expires_at }),
        },
    )
    .await;
    Ok(Json(AccessKeyResponse::from(access_key)))
}

async fn revoke_access_key(
    State(state): State<Arc<AppState>>,
    Path(access_key_id): Path<String>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<StatusCode, ApiError> {
    verify_csrf(&state, &headers).await?;
    let user = authenticate(&state, &headers).await?;
    let access_key = state
        .repository
        .find_access_key(&access_key_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("access key not found"))?;
    let owned = state
        .repository
        .application_for_user_by_id(user.id, access_key.application_id)
        .await
        .map_err(ApiError::from_repository)?
        .is_some();
    if !owned {
        return Err(ApiError::not_found("access key not found"));
    }
    state
        .repository
        .revoke_access_key(
            &access_key_id,
            access_key.application_id,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(ApiError::from_repository)?;
    record_session_audit(
        &state,
        user.id,
        &request_id.0.0,
        SessionAudit {
            application_id: access_key.application_id,
            action: "access_key.revoked",
            target_type: "access_key",
            target_id: access_key_id,
            summary: serde_json::json!({}),
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn owned_application_by_app_id(
    state: &AppState,
    headers: &HeaderMap,
    app_id: &str,
) -> Result<ApplicationSummary, ApiError> {
    let user = authenticate(state, headers).await?;
    state
        .repository
        .application_for_user_by_app_id(user.id, app_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("application not found"))
}

async fn verify_csrf(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let session = session_token(headers).ok_or_else(ApiError::unauthorized)?;
    let csrf = headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .zip(cookie_value(headers, CSRF_COOKIE))
        .filter(|(header, cookie)| header == cookie)
        .map(|(token, _)| token)
        .ok_or_else(|| ApiError::forbidden("CSRF token is required"))?;
    let valid = state
        .repository
        .valid_session_csrf(
            &token_hash(session),
            &token_hash(csrf),
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(ApiError::from_repository)?;
    if valid {
        Ok(())
    } else {
        Err(ApiError::forbidden("CSRF token is invalid"))
    }
}

async fn default_application(
    state: &AppState,
    user_id: UserId,
) -> Result<ApplicationSummary, ApiError> {
    state
        .repository
        .default_application_for_user(user_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("application not found"))
}

async fn session_response<T: Serialize>(
    state: &AppState,
    user_id: UserId,
    body: T,
    now: OffsetDateTime,
    status: StatusCode,
    created_ip: Option<IpAddr>,
    user_agent_summary: Option<&str>,
) -> Result<Response, ApiError> {
    let token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let csrf_token = URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>());
    let created_ip = created_ip.map(|value| value.to_string());
    state
        .repository
        .create_session_with_context(
            user_id,
            &token_hash(&token),
            &token_hash(&csrf_token),
            now + time::Duration::seconds(SESSION_SECONDS),
            now,
            created_ip.as_deref(),
            user_agent_summary,
        )
        .await
        .map_err(ApiError::from_repository)?;
    let cookie_attributes = cookie_attributes(&state.cookie_config);
    let cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly{cookie_attributes}; Max-Age={SESSION_SECONDS}"
    );
    let csrf_cookie =
        format!("{CSRF_COOKIE}={csrf_token}; Path=/{cookie_attributes}; Max-Age={SESSION_SECONDS}");
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&cookie)
            .map_err(|_| ApiError::unavailable("failed to create session cookie"))?,
    );
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&csrf_cookie)
            .map_err(|_| ApiError::unavailable("failed to create CSRF cookie"))?,
    );
    Ok(response)
}

