// Authentication and session handlers.

async fn register(
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    Json(request): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegistrationResponse>), ApiError> {
    if !state.registration_enabled {
        return Err(ApiError::registration_disabled());
    }
    enforce_auth_rate_limit(
        &state,
        "register",
        &connect_info,
        Some(&request.email),
        5,
        3,
        StdDuration::from_secs(60 * 60),
    )?;
    let email = normalize_email(&request.email).map_err(ApiError::from_identity)?;
    let password_hash = hash_password(&request.password).map_err(ApiError::from_identity)?;
    let now = OffsetDateTime::now_utc();
    let user_id = UserId::new();
    let application_id = ApplicationId::new();
    let default_bucket = Bucket::new(
        BucketId::new(),
        application_id,
        "media",
        BucketPolicy::unrestricted(Visibility::Private),
        now,
    )
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let verification_token = generate_auth_token();
    let verification_expires_at = now + time::Duration::seconds(VERIFY_EMAIL_TOKEN_SECONDS);
    state
        .repository
        .register_user(
            user_id,
            &email,
            &password_hash,
            application_id,
            "Default application",
            &format!("app_{}", application_id.as_uuid().simple()),
            DEFAULT_APPLICATION_QUOTA_BYTES,
            &default_bucket,
            &token_hash(&verification_token),
            verification_expires_at,
            now,
        )
        .await
        .map_err(ApiError::from_repository)?;
    if let Some(provider) = &state.email_provider
        && let Err(error) = provider
            .send_token(
                &email,
                AuthEmailKind::VerifyEmail,
                &verification_token,
                verification_expires_at,
            )
            .await
    {
        warn!(error = %error.message, email, "registration verification email delivery failed");
    }
    Ok((
        StatusCode::CREATED,
        Json(RegistrationResponse {
            email,
            status: "pending_verification",
            verification_token: state.expose_auth_tokens.then_some(verification_token),
        }),
    ))
}

async fn verify_email(
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    Json(request): Json<OneTimeTokenRequest>,
) -> Result<Json<AuthStatusResponse>, ApiError> {
    validate_one_time_token(&request.token)?;
    enforce_auth_rate_limit(
        &state,
        "consume",
        &connect_info,
        Some(&token_hash(&request.token)),
        20,
        10,
        StdDuration::from_secs(60 * 60),
    )?;
    let consumed = state
        .repository
        .consume_email_verification_token(&token_hash(&request.token), OffsetDateTime::now_utc())
        .await
        .map_err(ApiError::from_repository)?;
    if !consumed {
        return Err(ApiError::invalid_one_time_token());
    }
    Ok(Json(AuthStatusResponse { status: "active" }))
}

async fn resend_verification(
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    Json(request): Json<ForgotPasswordRequest>,
) -> Result<(StatusCode, Json<ResendVerificationResponse>), ApiError> {
    enforce_auth_rate_limit(
        &state,
        "resend_verification",
        &connect_info,
        Some(&request.email),
        10,
        3,
        StdDuration::from_secs(60 * 60),
    )?;
    let now = OffsetDateTime::now_utc();
    let token = generate_auth_token();
    let expires_at = now + time::Duration::seconds(VERIFY_EMAIL_TOKEN_SECONDS);
    if let Ok(email) = normalize_email(&request.email)
        && let Some(user) = state
            .repository
            .find_user_by_email(&email)
            .await
            .map_err(ApiError::from_repository)?
        && user.status == "pending_verification"
    {
        state
            .repository
            .create_one_time_token(
                user.id,
                OneTimeTokenPurpose::VerifyEmail,
                &token_hash(&token),
                expires_at,
                now,
            )
            .await
            .map_err(ApiError::from_repository)?;
        if let Some(provider) = &state.email_provider
            && let Err(error) = provider
                .send_token(&email, AuthEmailKind::VerifyEmail, &token, expires_at)
                .await
        {
            warn!(error = %error.message, "verification email delivery failed");
        }
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(ResendVerificationResponse {
            message: "if the account is pending verification, instructions have been issued",
            verification_token: state.expose_auth_tokens.then_some(token),
        }),
    ))
}

async fn login(
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    enforce_auth_rate_limit(
        &state,
        "login",
        &connect_info,
        Some(&request.email),
        30,
        10,
        StdDuration::from_secs(60 * 15),
    )?;
    let email = normalize_email(&request.email).map_err(|_| ApiError::invalid_credentials())?;
    let Some(user) = state
        .repository
        .find_user_by_email(&email)
        .await
        .map_err(ApiError::from_repository)?
    else {
        return Err(ApiError::invalid_credentials());
    };
    if user.status != "active" || !verify_password(&user.password_hash, &request.password) {
        return Err(ApiError::invalid_credentials());
    }
    let application = default_application(&state, user.id).await?;
    let now = OffsetDateTime::now_utc();
    state
        .repository
        .record_user_login(user.id, now)
        .await
        .map_err(ApiError::from_repository)?;
    session_response(
        &state,
        user.id,
        MeResponse::from_user_and_application(&user, &application),
        now,
        StatusCode::OK,
        Some(connect_info.0.ip()),
        summarized_user_agent(&headers).as_deref(),
    )
    .await
}

async fn forgot_password(
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    Json(request): Json<ForgotPasswordRequest>,
) -> Result<(StatusCode, Json<ForgotPasswordResponse>), ApiError> {
    enforce_auth_rate_limit(
        &state,
        "forgot",
        &connect_info,
        Some(&request.email),
        10,
        3,
        StdDuration::from_secs(60 * 60),
    )?;
    let now = OffsetDateTime::now_utc();
    let reset_token = generate_auth_token();
    let reset_expires_at = now + time::Duration::seconds(RESET_PASSWORD_TOKEN_SECONDS);
    if let Ok(email) = normalize_email(&request.email)
        && let Some(user) = state
            .repository
            .find_user_by_email(&email)
            .await
            .map_err(ApiError::from_repository)?
        && user.status != "deleted"
    {
        state
            .repository
            .create_one_time_token(
                user.id,
                OneTimeTokenPurpose::ResetPassword,
                &token_hash(&reset_token),
                reset_expires_at,
                now,
            )
            .await
            .map_err(ApiError::from_repository)?;
        if let Some(provider) = &state.email_provider
            && let Err(error) = provider
                .send_token(
                    &email,
                    AuthEmailKind::ResetPassword,
                    &reset_token,
                    reset_expires_at,
                )
                .await
        {
            warn!(error = %error.message, "password reset email delivery failed");
        }
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(ForgotPasswordResponse {
            message: "if the account exists, password reset instructions have been issued",
            reset_token: state.expose_auth_tokens.then_some(reset_token),
        }),
    ))
}

async fn reset_password(
    State(state): State<Arc<AppState>>,
    connect_info: ConnectInfo<SocketAddr>,
    Json(request): Json<ResetPasswordRequest>,
) -> Result<Response, ApiError> {
    validate_one_time_token(&request.token)?;
    enforce_auth_rate_limit(
        &state,
        "consume",
        &connect_info,
        Some(&token_hash(&request.token)),
        20,
        10,
        StdDuration::from_secs(60 * 60),
    )?;
    let password_hash = hash_password(&request.password).map_err(ApiError::from_identity)?;
    let consumed = state
        .repository
        .consume_password_reset_token(
            &token_hash(&request.token),
            &password_hash,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(ApiError::from_repository)?;
    if !consumed {
        return Err(ApiError::invalid_one_time_token());
    }
    clear_auth_cookies_response(&state, StatusCode::NO_CONTENT)
}

async fn logout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    verify_csrf(&state, &headers).await?;
    if let Some(token) = session_token(&headers) {
        state
            .repository
            .delete_session_by_hash(&token_hash(token))
            .await
            .map_err(ApiError::from_repository)?;
    }
    clear_auth_cookies_response(&state, StatusCode::NO_CONTENT)
}

async fn me(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<MeResponse>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let application = default_application(&state, user.id).await?;
    Ok(Json(MeResponse::from_user_and_application(
        &user,
        &application,
    )))
}

async fn list_sessions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<Vec<SessionResponse>>, ApiError> {
    let user = authenticate(&state, &headers).await?;
    let current_token_hash =
        token_hash(session_token(&headers).expect("authenticated request has a session cookie"));
    let sessions = state
        .repository
        .list_active_sessions(user.id, &current_token_hash, OffsetDateTime::now_utc())
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(
        sessions.into_iter().map(SessionResponse::from).collect(),
    ))
}

async fn revoke_session(
    State(state): State<Arc<AppState>>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    verify_csrf(&state, &headers).await?;
    let user = authenticate(&state, &headers).await?;
    let current_token_hash =
        token_hash(session_token(&headers).expect("authenticated request has a session cookie"));
    let revoked_current = state
        .repository
        .revoke_session(
            user.id,
            &session_id,
            &current_token_hash,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    if revoked_current {
        clear_auth_cookies_response(&state, StatusCode::NO_CONTENT)
    } else {
        Ok(StatusCode::NO_CONTENT.into_response())
    }
}

async fn revoke_all_sessions(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    verify_csrf(&state, &headers).await?;
    let user = authenticate(&state, &headers).await?;
    state
        .repository
        .revoke_all_sessions(user.id, OffsetDateTime::now_utc())
        .await
        .map_err(ApiError::from_repository)?;
    clear_auth_cookies_response(&state, StatusCode::NO_CONTENT)
}

