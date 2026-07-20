// Webhook and audit handlers.

// Webhook, audit, access-key, and application-auth helpers.

async fn list_audit_logs(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<Vec<AuditResponse>>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.authorize("application:read")?;
    let events = state
        .repository
        .list_audit(auth.application.id, 100)
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(events.into_iter().map(AuditResponse::from).collect()))
}

async fn list_webhooks(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<Vec<WebhookResponse>>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.authorize("webhook:manage")?;
    let endpoints = state
        .repository
        .list_webhook_endpoints(auth.application.id)
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(
        endpoints.into_iter().map(WebhookResponse::from).collect(),
    ))
}

async fn create_webhook(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
    Json(request): Json<CreateWebhookRequest>,
) -> Result<(StatusCode, Json<CreateWebhookResponse>), ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("webhook:manage")?;
    let url = validate_webhook_url(request.url)?;
    let subscribed_events = validate_webhook_events(request.events)?;
    let secret = generate_secret();
    let endpoint = NewWebhookEndpoint {
        id: format!("wh_{}", uuid::Uuid::now_v7().simple()),
        application_id: auth.application.id,
        url: url.clone(),
        secret_ciphertext: state
            .access_key_cipher
            .encrypt(secret.as_bytes())
            .map_err(ApiError::from_access_key_cipher)?,
        secret_key_version: state.access_key_cipher.version(),
        subscribed_events: subscribed_events.clone(),
        enabled: request.enabled,
        created_at: OffsetDateTime::now_utc(),
    };
    state
        .repository
        .create_webhook_endpoint(&endpoint)
        .await
        .map_err(ApiError::from_repository)?;
    let response = WebhookResponse {
        id: endpoint.id.clone(),
        url: endpoint.url.clone(),
        events: endpoint.subscribed_events.clone(),
        enabled: endpoint.enabled,
        created_at: endpoint.created_at,
        updated_at: endpoint.created_at,
    };
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "webhook.created",
        "webhook",
        endpoint.id.clone(),
        serde_json::json!({ "url": response.url.clone(), "events": response.events.clone(), "enabled": response.enabled }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(CreateWebhookResponse {
            endpoint: response,
            secret,
        }),
    ))
}

async fn update_webhook(
    State(state): State<Arc<AppState>>,
    Path(webhook_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
    Json(request): Json<UpdateWebhookRequest>,
) -> Result<Json<UpdateWebhookResponse>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("webhook:manage")?;
    if !request.has_changes() {
        return Err(ApiError::bad_request(
            "at least one webhook field is required",
        ));
    }
    let endpoint = state
        .repository
        .find_webhook_endpoint(auth.application.id, &webhook_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("webhook endpoint not found"))?;
    let url = request
        .url
        .map(validate_webhook_url)
        .transpose()?
        .unwrap_or_else(|| endpoint.url.clone());
    let subscribed_events = request
        .events
        .map(validate_webhook_events)
        .transpose()?
        .unwrap_or_else(|| endpoint.subscribed_events.clone());
    let enabled = request.enabled.unwrap_or(endpoint.enabled);
    let (secret, secret_ciphertext, secret_key_version) = if request.rotate_secret {
        let secret = generate_secret();
        let ciphertext = state
            .access_key_cipher
            .encrypt(secret.as_bytes())
            .map_err(ApiError::from_access_key_cipher)?;
        (Some(secret), ciphertext, state.access_key_cipher.version())
    } else {
        (
            None,
            endpoint.secret_ciphertext.clone(),
            endpoint.secret_key_version,
        )
    };
    let update = WebhookEndpointUpdate {
        url: url.clone(),
        secret_ciphertext,
        secret_key_version,
        subscribed_events: subscribed_events.clone(),
        enabled,
        updated_at: OffsetDateTime::now_utc(),
    };
    let updated = state
        .repository
        .update_webhook_endpoint(auth.application.id, &webhook_id, &update)
        .await
        .map_err(ApiError::from_repository)?;
    if !updated {
        return Err(ApiError::not_found("webhook endpoint not found"));
    }
    let response = WebhookResponse {
        id: endpoint.id.clone(),
        url,
        events: subscribed_events,
        enabled,
        created_at: endpoint.created_at,
        updated_at: update.updated_at,
    };
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "webhook.updated",
        "webhook",
        webhook_id,
        serde_json::json!({ "url": response.url.clone(), "events": response.events.clone(), "enabled": response.enabled, "secret_rotated": secret.is_some() }),
    )
    .await;
    Ok(Json(UpdateWebhookResponse {
        endpoint: response,
        secret,
    }))
}

async fn delete_webhook(
    State(state): State<Arc<AppState>>,
    Path(webhook_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
) -> Result<StatusCode, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("webhook:manage")?;
    let deleted = state
        .repository
        .delete_webhook_endpoint(auth.application.id, &webhook_id)
        .await
        .map_err(ApiError::from_repository)?;
    if !deleted {
        return Err(ApiError::not_found("webhook endpoint not found"));
    }
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "webhook.deleted",
        "webhook",
        webhook_id,
        serde_json::json!({}),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_webhook_deliveries(
    State(state): State<Arc<AppState>>,
    Path(webhook_id): Path<String>,
    Query(query): Query<ListWebhookDeliveriesQuery>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<WebhookDeliveryListResponse>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.authorize("webhook:manage")?;
    state
        .repository
        .find_webhook_endpoint(auth.application.id, &webhook_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("webhook endpoint not found"))?;
    let limit = query.limit.unwrap_or(50);
    if limit == 0 || limit > 100 {
        return Err(ApiError::bad_request("limit must be between 1 and 100"));
    }
    let status = query
        .status
        .as_deref()
        .map(parse_webhook_delivery_status)
        .transpose()?;
    let cursor = query
        .cursor
        .as_deref()
        .map(decode_webhook_delivery_cursor)
        .transpose()?;
    let page = state
        .repository
        .list_webhook_delivery_history(
            auth.application.id,
            &webhook_id,
            &WebhookDeliveryHistoryQuery {
                status,
                cursor,
                limit,
            },
        )
        .await
        .map_err(ApiError::from_repository)?;
    let next_cursor = if page.has_more {
        page.items.last().map(|item| {
            encode_webhook_delivery_cursor(WebhookDeliveryHistoryCursor {
                row_id: item.row_id,
            })
        })
    } else {
        None
    };
    Ok(Json(WebhookDeliveryListResponse {
        items: page
            .items
            .into_iter()
            .map(WebhookDeliveryHistoryResponse::from)
            .collect(),
        next_cursor,
    }))
}

async fn replay_webhook_delivery(
    State(state): State<Arc<AppState>>,
    Path((webhook_id, event_id)): Path<(String, String)>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
) -> Result<StatusCode, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("webhook:manage")?;
    state
        .repository
        .find_webhook_endpoint(auth.application.id, &webhook_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("webhook endpoint not found"))?;
    let replayed = state
        .repository
        .replay_webhook_delivery(
            auth.application.id,
            &webhook_id,
            &event_id,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(ApiError::from_repository)?;
    if !replayed {
        return Err(ApiError::conflict(
            "only delivered or dead-lettered webhook deliveries can be replayed",
        ));
    }
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "webhook.delivery_replayed",
        "webhook_delivery",
        event_id,
        serde_json::json!({ "endpoint_id": webhook_id }),
    )
    .await;
    Ok(StatusCode::ACCEPTED)
}

async fn record_audit(
    state: &AppState,
    auth: &ApplicationAuth,
    request_id: &str,
    action: &str,
    target_type: &str,
    target_id: String,
    summary: serde_json::Value,
) {
    let event = AuditEvent {
        id: uuid::Uuid::now_v7().to_string(),
        application_id: auth.application.id,
        actor_type: auth.actor_type.into(),
        actor_id: auth.actor_id.clone(),
        action: action.into(),
        target_type: target_type.into(),
        target_id,
        request_id: request_id.into(),
        summary,
        created_at: OffsetDateTime::now_utc(),
    };
    if let Err(error) = state.repository.record_audit(&event).await {
        warn!(error = %error, action, target_type, "audit write failed after successful operation");
    }
}

async fn record_session_audit(
    state: &AppState,
    user_id: UserId,
    request_id: &str,
    audit: SessionAudit<'_>,
) {
    let event = AuditEvent {
        id: uuid::Uuid::now_v7().to_string(),
        application_id: audit.application_id,
        actor_type: "user".into(),
        actor_id: user_id.to_string(),
        action: audit.action.into(),
        target_type: audit.target_type.into(),
        target_id: audit.target_id,
        request_id: request_id.into(),
        summary: audit.summary,
        created_at: OffsetDateTime::now_utc(),
    };
    if let Err(error) = state.repository.record_audit(&event).await {
        warn!(error = %error, action = audit.action, target_type = audit.target_type, "audit write failed after successful operation");
    }
}

struct SessionAudit<'a> {
    application_id: ApplicationId,
    action: &'a str,
    target_type: &'a str,
    target_id: String,
    summary: serde_json::Value,
}

