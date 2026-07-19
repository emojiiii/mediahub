// Application management handlers.

async fn list_applications(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<Vec<ApplicationResponse>>, ApiError> {
    if let Some(hmac_identity) = hmac_identity {
        let auth = authenticated_application(&state, &headers, Some(hmac_identity.0)).await?;
        auth.authorize("application:read")?;
        return Ok(Json(vec![ApplicationResponse::from(auth.application)]));
    }
    let user = authenticate(&state, &headers).await?;
    let applications = state
        .repository
        .list_applications_for_user(user.id)
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(
        applications
            .into_iter()
            .map(ApplicationResponse::from)
            .collect(),
    ))
}

async fn create_application(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    Json(request): Json<CreateApplicationRequest>,
) -> Result<(StatusCode, Json<ApplicationResponse>), ApiError> {
    verify_csrf(&state, &headers).await?;
    let user = authenticate(&state, &headers).await?;
    let name = validate_application_name(request.name)?;
    let application_id = ApplicationId::new();
    let app_id = format!("app_{}", application_id.as_uuid().simple());
    let now = OffsetDateTime::now_utc();
    state
        .repository
        .create_application(
            application_id,
            user.id,
            &name,
            &app_id,
            DEFAULT_APPLICATION_QUOTA_BYTES,
            now,
        )
        .await
        .map_err(ApiError::from_repository)?;
    let application = state
        .repository
        .application_for_user_by_app_id(user.id, &app_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::unavailable("created application is unavailable"))?;
    record_session_audit(
        &state,
        user.id,
        &request_id.0.0,
        SessionAudit {
            application_id,
            action: "application.created",
            target_type: "application",
            target_id: app_id,
            summary: serde_json::json!({ "name": name }),
        },
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(ApplicationResponse::from(application)),
    ))
}

async fn get_application(
    State(state): State<Arc<AppState>>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<ApplicationResponse>, ApiError> {
    if let Some(hmac_identity) = hmac_identity {
        let auth = authenticated_application(&state, &headers, Some(hmac_identity.0)).await?;
        auth.authorize("application:read")?;
        if auth.application.app_id != app_id {
            return Err(ApiError::not_found("application not found"));
        }
        return Ok(Json(ApplicationResponse::from(auth.application)));
    }
    let application = owned_application_by_app_id(&state, &headers, &app_id).await?;
    Ok(Json(ApplicationResponse::from(application)))
}

async fn update_application(
    State(state): State<Arc<AppState>>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    Json(request): Json<UpdateApplicationRequest>,
) -> Result<Json<ApplicationResponse>, ApiError> {
    verify_csrf(&state, &headers).await?;
    let user = authenticate(&state, &headers).await?;
    let name = validate_application_name(request.name)?;
    let updated = state
        .repository
        .update_application_name_for_user(user.id, &app_id, &name, OffsetDateTime::now_utc())
        .await
        .map_err(ApiError::from_repository)?;
    if !updated {
        return Err(ApiError::not_found("application not found"));
    }
    let application = state
        .repository
        .application_for_user_by_app_id(user.id, &app_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("application not found"))?;
    record_session_audit(
        &state,
        user.id,
        &request_id.0.0,
        SessionAudit {
            application_id: application.id,
            action: "application.updated",
            target_type: "application",
            target_id: application.app_id.clone(),
            summary: serde_json::json!({ "name": application.name.clone() }),
        },
    )
    .await;
    Ok(Json(ApplicationResponse::from(application)))
}

async fn delete_application(
    State(state): State<Arc<AppState>>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    verify_csrf(&state, &headers).await?;
    let user = authenticate(&state, &headers).await?;
    let deleted = state
        .repository
        .delete_application_for_user(user.id, &app_id)
        .await
        .map_err(ApiError::from_repository)?;
    if !deleted {
        return Err(ApiError::not_found("application not found"));
    }
    Ok(StatusCode::NO_CONTENT)
}

