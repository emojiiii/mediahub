// Path object mutation handlers.

async fn create_path_signed_url(
    State(state): State<Arc<AppState>>,
    Path((app_id, bucket_name, object_key)): Path<(String, String, String)>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<SignedMediaUrlResponse>, ApiError> {
    let auth = authenticated_path_application(
        &state,
        &headers,
        hmac_identity.map(|identity| identity.0),
        &app_id,
    )
    .await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("media:read")?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("object not found"))?;
    let media = state
        .repository
        .find_by_object_key(auth.application.id, bucket.id(), &object_key)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("object not found"))?;
    media
        .ensure_readable()
        .map_err(|_| ApiError::not_found("object not found"))?;
    let expires_at = OffsetDateTime::now_utc() + time::Duration::seconds(SIGNED_MEDIA_URL_SECONDS);
    let token = state.media_url_signer.sign(media.id(), expires_at);
    Ok(Json(SignedMediaUrlResponse {
        url: format!(
            "{}?token={token}",
            object_content_path(&app_id, &bucket_name, &object_key)
        ),
        expires_at,
    }))
}

async fn update_path_object(
    State(state): State<Arc<AppState>>,
    Path((app_id, bucket_name, object_key)): Path<(String, String, String)>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
    Json(request): Json<UpdateMediaRequest>,
) -> Result<Json<MediaResponse>, ApiError> {
    let auth = authenticated_path_application(
        &state,
        &headers,
        hmac_identity.map(|identity| identity.0),
        &app_id,
    )
    .await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("media:update")?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("object not found"))?;
    let mut media = state
        .repository
        .find_by_object_key(auth.application.id, bucket.id(), &object_key)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("object not found"))?;
    media
        .ensure_readable()
        .map_err(|_| ApiError::conflict("object cannot be updated in its current state"))?;

    let expected_revision = parse_if_match(&headers)?;
    let mut revision = expected_revision;
    if expected_revision != media.revision() {
        return Err(ApiError::conflict("media revision does not match"));
    }
    let now = OffsetDateTime::now_utc();
    let mut changed = false;
    if let Some(display_name) = request.display_name {
        media
            .set_display_name(display_name, revision, now)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        revision = media.revision();
        changed = true;
    }
    if let Some(visibility) = request.visibility {
        media
            .set_visibility_override(visibility, revision, now)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        revision = media.revision();
        changed = true;
    }
    if let Some(ttl_seconds) = request.ttl_seconds {
        let expires_at = ttl_seconds
            .map(|seconds| {
                let seconds = i64::try_from(seconds)
                    .map_err(|_| ApiError::bad_request("ttl_seconds is too large"))?;
                Ok(now + time::Duration::seconds(seconds))
            })
            .transpose()?;
        media
            .set_expire_at(expires_at, revision, now)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        revision = media.revision();
        changed = true;
    }
    if let Some(metadata) = request.metadata {
        let metadata = ClientMetadata::from_value(metadata)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        media
            .replace_client_metadata(metadata, revision, now)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
        changed = true;
    }
    if !changed {
        return Err(ApiError::bad_request(
            "at least one mutable field is required",
        ));
    }
    let event = OutboxEvent::media_metadata_updated(&media, now);
    state
        .repository
        .update_media(media.clone(), expected_revision, event)
        .await
        .map_err(ApiError::from_repository)?;
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "media.updated",
        "media",
        media.id().to_string(),
        serde_json::json!({ "revision": media.revision() }),
    )
    .await;
    Ok(Json(MediaResponse::from(media)))
}

