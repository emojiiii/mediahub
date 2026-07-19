// Administration handlers.

async fn admin_list_users(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AdminListQuery>,
) -> Result<Json<Vec<AdminUserResponse>>, ApiError> {
    require_admin(&state, &headers).await?;
    let users = state
        .repository
        .list_admin_users(admin_limit(query.limit)?)
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(
        users.into_iter().map(AdminUserResponse::from).collect(),
    ))
}

async fn admin_update_user_status(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    Path(user_id): Path<String>,
    Json(request): Json<AdminUpdateUserStatusRequest>,
) -> Result<Json<AdminUserResponse>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    verify_csrf(&state, &headers).await?;
    if !matches!(request.status.as_str(), "active" | "suspended") {
        return Err(ApiError::bad_request("status must be active or suspended"));
    }
    let target =
        UserId::from_str(&user_id).map_err(|_| ApiError::bad_request("user id is invalid"))?;
    let user = state
        .repository
        .transition_user_status(
            admin.id,
            target,
            &request.status,
            &request_id.0.0,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(AdminUserResponse::from(user)))
}

async fn admin_list_applications(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AdminListQuery>,
) -> Result<Json<Vec<AdminApplicationResponse>>, ApiError> {
    require_admin(&state, &headers).await?;
    let applications = state
        .repository
        .list_admin_applications(admin_limit(query.limit)?)
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(
        applications
            .into_iter()
            .map(AdminApplicationResponse::from)
            .collect(),
    ))
}

async fn admin_update_application_quota(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    Path(application_id): Path<String>,
    Json(request): Json<AdminUpdateApplicationQuotaRequest>,
) -> Result<Json<AdminApplicationResponse>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    verify_csrf(&state, &headers).await?;
    if request.quota_bytes > i64::MAX as u64 {
        return Err(ApiError::bad_request(
            "quota_bytes exceeds the supported range",
        ));
    }
    let application_id = ApplicationId::from_str(&application_id)
        .map_err(|_| ApiError::bad_request("application id is invalid"))?;
    let application = state
        .repository
        .update_application_quota(
            admin.id,
            application_id,
            request.quota_bytes,
            &request_id.0.0,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|error| match error {
            mediahub_app::RepositoryError::Conflict => ApiError {
                status: StatusCode::CONFLICT,
                code: "quota_below_usage",
                message: "quota cannot be less than used plus reserved bytes".into(),
            },
            other => ApiError::from_repository(other),
        })?;
    Ok(Json(AdminApplicationResponse::from(application)))
}

async fn admin_list_jobs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AdminListQuery>,
) -> Result<Json<Vec<AdminJobResponse>>, ApiError> {
    require_admin(&state, &headers).await?;
    let jobs = state
        .repository
        .list_admin_jobs(admin_limit(query.limit)?)
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(jobs.into_iter().map(AdminJobResponse::from).collect()))
}

async fn admin_storage(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminStorageResponse>, ApiError> {
    require_admin(&state, &headers).await?;
    let summary = state
        .repository
        .admin_storage_summary()
        .await
        .map_err(ApiError::from_repository)?;
    let mut response = AdminStorageResponse::from(summary);
    (response.disk_total_bytes, response.disk_available_bytes) =
        storage_capacity(&state.object_store)?;
    Ok(Json(response))
}

async fn admin_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Json<AdminSettingsResponse>, ApiError> {
    require_admin(&state, &headers).await?;
    let settings = state
        .repository
        .admin_system_settings()
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(AdminSettingsResponse::from(settings)))
}

async fn admin_update_settings(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    Json(request): Json<AdminUpdateSettingsRequest>,
) -> Result<Json<AdminSettingsResponse>, ApiError> {
    let admin = require_admin(&state, &headers).await?;
    verify_csrf(&state, &headers).await?;
    let download_bytes_per_second = request
        .download_bytes_per_second
        .ok_or_else(|| ApiError::bad_request("download_bytes_per_second must be provided"))?;
    if download_bytes_per_second.is_some_and(|value| {
        !(MIN_DOWNLOAD_BYTES_PER_SECOND..=MAX_DOWNLOAD_BYTES_PER_SECOND).contains(&value)
    }) {
        return Err(ApiError::bad_request(format!(
            "download_bytes_per_second must be null or between {MIN_DOWNLOAD_BYTES_PER_SECOND} and {MAX_DOWNLOAD_BYTES_PER_SECOND}"
        )));
    }
    let settings = state
        .repository
        .update_admin_system_settings(
            admin.id,
            download_bytes_per_second,
            &request_id.0.0,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(ApiError::from_repository)?;
    record_admin_settings_audit(&state, admin.id, &request_id.0.0, settings).await;
    Ok(Json(AdminSettingsResponse::from(settings)))
}

async fn record_admin_settings_audit(
    state: &AppState,
    admin_id: UserId,
    request_id: &str,
    settings: AdminSystemSettings,
) {
    match state
        .repository
        .default_application_for_user(admin_id)
        .await
    {
        Ok(Some(application)) => {
            record_session_audit(
                state,
                admin_id,
                request_id,
                SessionAudit {
                    application_id: application.id,
                    action: "system.settings_updated",
                    target_type: "system_settings",
                    target_id: "global".into(),
                    summary: serde_json::json!({
                        "download_bytes_per_second": settings.download_bytes_per_second,
                    }),
                },
            )
            .await;
        }
        Ok(None) => {
            warn!(%admin_id, "settings update audit skipped because the Admin has no Application");
        }
        Err(error) => {
            warn!(%admin_id, %error, "settings update audit Application lookup failed");
        }
    }
}

async fn admin_list_audit(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<AdminListQuery>,
) -> Result<Json<Vec<AdminAuditResponse>>, ApiError> {
    require_admin(&state, &headers).await?;
    let events = state
        .repository
        .list_admin_audit(admin_limit(query.limit)?)
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(
        events.into_iter().map(AdminAuditResponse::from).collect(),
    ))
}

fn admin_limit(limit: Option<usize>) -> Result<usize, ApiError> {
    let limit = limit.unwrap_or(100);
    if (1..=200).contains(&limit) {
        Ok(limit)
    } else {
        Err(ApiError::bad_request("limit must be between 1 and 200"))
    }
}

