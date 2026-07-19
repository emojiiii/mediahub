// Media responses and application authentication helpers.

async fn read_media_bytes(
    state: &AppState,
    media: &Media,
    visibility: Visibility,
    method: Method,
    query: ReadMediaQuery,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if let Some(transform) = query.transform()? {
        let _slot = state
            .variant_slots
            .acquire()
            .await
            .map_err(|_| ApiError::unavailable("image processing is unavailable"))?;
        let source_content = state
            .object_store
            .read(media.storage_key())
            .await
            .map_err(|_| ApiError::not_found("media object is unavailable"))?;
        let receipt = VariantService::new(
            state.repository.clone(),
            state.object_store.clone(),
            RuntimeImageProcessor,
            SystemClock,
        )
        .generate(mediahub_app::GenerateVariantRequest {
            media_id: media.id(),
            media_sha256: media.sha256().to_owned(),
            source_content,
            transform,
        })
        .await
        .map_err(ApiError::from_variant)?;
        if receipt.cache_hit {
            state
                .http_metrics
                .variant_cache_hits
                .fetch_add(1, Ordering::Relaxed);
        } else {
            state
                .http_metrics
                .variant_cache_misses
                .fetch_add(1, Ordering::Relaxed);
        }
        if if_none_match_matches(&headers, &receipt.variant.transform_key) {
            return Ok(variant_not_modified_response(
                media,
                &receipt.variant.transform_key,
                visibility,
            ));
        }
        let head_only = method == Method::HEAD;
        let download_bytes_per_second = if head_only {
            None
        } else {
            configured_download_rate(state).await?
        };
        return Ok(variant_response_bytes(
            media,
            receipt,
            visibility,
            head_only,
            download_bytes_per_second,
        ));
    }
    if if_none_match_matches(&headers, media.etag()) {
        return Ok(media_not_modified_response(media, visibility));
    }
    let total = usize::try_from(media.size()).map_err(|_| ApiError::range_not_satisfiable())?;
    let range = headers
        .get(RANGE)
        .map(|value| {
            value
                .to_str()
                .map_err(|_| ApiError::range_not_satisfiable())
        })
        .transpose()?
        .map(|value| parse_single_range(value, total))
        .transpose()?;
    let body = if method == Method::HEAD {
        state
            .object_store
            .head(media.storage_key())
            .await
            .map(|_| Body::empty())
    } else {
        match range {
            Some((start, end)) => {
                let start = u64::try_from(start).map_err(|_| ApiError::range_not_satisfiable())?;
                let end = u64::try_from(end)
                    .map_err(|_| ApiError::range_not_satisfiable())?
                    .checked_add(1)
                    .ok_or_else(ApiError::range_not_satisfiable)?;
                state
                    .object_store
                    .read_range(media.storage_key(), start..end)
                    .await
                    .map(Body::from)
            }
            None => {
                if let Some(local_store) = state.object_store.local_store() {
                    local_store
                        .open_file(media.storage_key())
                        .await
                        .map(local_file_body)
                } else {
                    state
                        .object_store
                        .read(media.storage_key())
                        .await
                        .map(Body::from)
                }
            }
        }
    }
    .map_err(|_| ApiError::not_found("media object is unavailable"))?;
    let head_only = method == Method::HEAD;
    let download_bytes_per_second = if head_only {
        None
    } else {
        configured_download_rate(state).await?
    };
    Ok(media_response_body(
        media,
        body,
        total,
        range,
        visibility,
        head_only,
        download_bytes_per_second,
    ))
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<UserAccount, ApiError> {
    let token = session_token(headers).ok_or_else(ApiError::unauthorized)?;
    state
        .repository
        .find_user_by_session_hash(&token_hash(token), OffsetDateTime::now_utc())
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(ApiError::unauthorized)
}

async fn require_admin(state: &AppState, headers: &HeaderMap) -> Result<UserAccount, ApiError> {
    let user = authenticate(state, headers).await?;
    if user.system_role == "admin" {
        Ok(user)
    } else {
        Err(ApiError::forbidden(
            "system administrator privileges are required",
        ))
    }
}

async fn authenticated_application(
    state: &AppState,
    headers: &HeaderMap,
    hmac_identity: Option<HmacIdentity>,
) -> Result<ApplicationAuth, ApiError> {
    let requested_app_id = headers
        .get("x-mediahub-app-id")
        .map(|value| {
            value
                .to_str()
                .ok()
                .map(str::trim)
                .filter(|value| !value.is_empty() && value.len() <= 128)
                .ok_or_else(|| ApiError::bad_request("X-MediaHub-App-Id is invalid"))
        })
        .transpose()?;
    if let Some(hmac_identity) = hmac_identity {
        let application = state
            .repository
            .find_application_by_id(hmac_identity.application_id)
            .await
            .map_err(ApiError::from_repository)?
            .ok_or_else(ApiError::unauthorized)?;
        if requested_app_id.is_some_and(|app_id| app_id != application.app_id) {
            return Err(ApiError::forbidden(
                "HMAC credentials cannot switch application context",
            ));
        }
        return Ok(ApplicationAuth {
            application,
            actor_type: "access_key",
            actor_id: hmac_identity.access_key_id.clone(),
            hmac_identity: Some(hmac_identity),
        });
    }
    let user = authenticate(state, headers).await?;
    let application = if let Some(app_id) = requested_app_id {
        state
            .repository
            .application_for_user_by_app_id(user.id, app_id)
            .await
            .map_err(ApiError::from_repository)?
            .ok_or_else(|| ApiError::not_found("application not found"))?
    } else {
        default_application(state, user.id).await?
    };
    Ok(ApplicationAuth {
        application,
        hmac_identity: None,
        actor_type: "user",
        actor_id: user.id.to_string(),
    })
}

