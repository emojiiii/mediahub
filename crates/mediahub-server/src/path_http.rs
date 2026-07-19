// Path bucket and object HTTP handlers.

// Application path and object-content handlers.

async fn read_object_content(
    State(state): State<Arc<AppState>>,
    Path((app_id, bucket_name, object_key)): Path<(String, String, String)>,
    method: Method,
    query: Result<Query<ReadMediaQuery>, axum::extract::rejection::QueryRejection>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Response, ApiError> {
    let query = parse_read_media_query(query)?;
    let application = state
        .repository
        .find_application_by_app_id(&app_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("media not found"))?;
    let bucket = state
        .repository
        .find_bucket_by_name(application.id, &bucket_name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("media not found"))?;
    let media = state
        .repository
        .find_by_object_key(application.id, bucket.id(), &object_key)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("media not found"))?;
    media
        .ensure_readable()
        .map_err(|_| ApiError::not_found("media not found"))?;
    let visibility = media.effective_visibility(bucket.policy().visibility());
    let hmac_authorized = hmac_identity.as_ref().is_some_and(|identity| {
        identity.application_id == application.id
            && identity
                .permissions
                .iter()
                .any(|permission| permission == "media:read")
    });
    if visibility == Visibility::Private && !hmac_authorized {
        query
            .token
            .as_deref()
            .ok_or_else(|| ApiError::not_found("media not found"))
            .and_then(|token| {
                state
                    .media_url_signer
                    .verify(token, media.id(), OffsetDateTime::now_utc())
                    .map_err(|_| ApiError::not_found("media not found"))
            })?;
    }
    read_media_bytes(&state, &media, visibility, method, query, headers).await
}

async fn list_path_buckets(
    State(state): State<Arc<AppState>>,
    Path(app_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<Vec<BucketResponse>>, ApiError> {
    let auth = authenticated_path_application(
        &state,
        &headers,
        hmac_identity.map(|identity| identity.0),
        &app_id,
    )
    .await?;
    auth.authorize("bucket:list")?;
    let buckets = state
        .repository
        .list_buckets(auth.application.id)
        .await
        .map_err(ApiError::from_repository)?;
    Ok(Json(
        buckets.into_iter().map(BucketResponse::from).collect(),
    ))
}

async fn list_path_objects(
    State(state): State<Arc<AppState>>,
    Path((app_id, bucket_name)): Path<(String, String)>,
    Query(query): Query<PathObjectListQuery>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<MediaListResponse>, ApiError> {
    let auth = authenticated_path_application(
        &state,
        &headers,
        hmac_identity.map(|identity| identity.0),
        &app_id,
    )
    .await?;
    auth.authorize("media:list")?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("bucket not found"))?;
    let limit = query.limit.unwrap_or(50);
    if limit == 0 || limit > 100 {
        return Err(ApiError::bad_request("limit must be between 1 and 100"));
    }
    let directory_mode = parse_list_delimiter(query.delimiter.as_deref())?;
    if query.prefix.as_ref().is_some_and(|prefix| {
        prefix.len() > 1024 || prefix.bytes().any(|byte| byte.is_ascii_control())
    }) {
        return Err(ApiError::bad_request("prefix filter is invalid"));
    }
    let object_key_prefix = query.prefix.filter(|prefix| !prefix.is_empty());
    if directory_mode {
        let cursor = query
            .cursor
            .as_deref()
            .map(decode_media_directory_cursor)
            .transpose()?;
        let page = state
            .repository
            .list_media_directory_page(
                auth.application.id,
                &MediaDirectoryListQuery {
                    bucket_id: bucket.id(),
                    state: Some(MediaState::Active),
                    mime: None,
                    created_from: None,
                    created_before: None,
                    object_key_prefix: object_key_prefix.unwrap_or_default(),
                    cursor,
                    limit,
                },
            )
            .await
            .map_err(ApiError::from_repository)?;
        return Ok(Json(MediaListResponse {
            items: page.items.into_iter().map(MediaResponse::from).collect(),
            common_prefixes: page.common_prefixes,
            next_cursor: page.next_cursor.map(encode_media_directory_cursor),
        }));
    }
    let cursor = query
        .cursor
        .as_deref()
        .map(decode_media_cursor)
        .transpose()?;
    let page = state
        .repository
        .list_media_page(
            auth.application.id,
            &MediaListQuery {
                bucket_id: Some(bucket.id()),
                state: Some(MediaState::Active),
                object_key_prefix,
                cursor,
                limit,
                ..MediaListQuery::default()
            },
        )
        .await
        .map_err(ApiError::from_repository)?;
    let next_cursor = page.has_more.then(|| {
        page.items.last().map(|media| {
            encode_media_cursor(MediaListCursor {
                created_at: media.created_at(),
                id: media.id(),
            })
        })
    });
    Ok(Json(MediaListResponse {
        items: page.items.into_iter().map(MediaResponse::from).collect(),
        common_prefixes: Vec::new(),
        next_cursor: next_cursor.flatten(),
    }))
}

async fn head_path_bucket(
    State(state): State<Arc<AppState>>,
    Path((app_id, bucket_name)): Path<(String, String)>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticated_path_application(
        &state,
        &headers,
        hmac_identity.map(|identity| identity.0),
        &app_id,
    )
    .await?;
    auth.authorize("bucket:list")?;
    state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("bucket not found"))?;
    Ok(StatusCode::OK)
}

async fn create_path_bucket(
    State(state): State<Arc<AppState>>,
    Path((app_id, bucket_name)): Path<(String, String)>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
) -> Result<Response, ApiError> {
    let auth = authenticated_path_application(
        &state,
        &headers,
        hmac_identity.map(|identity| identity.0),
        &app_id,
    )
    .await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("bucket:manage")?;
    if let Some(bucket) = state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(ApiError::from_repository)?
    {
        return Ok((StatusCode::OK, Json(BucketResponse::from(bucket))).into_response());
    }
    let bucket = Bucket::new(
        BucketId::new(),
        auth.application.id,
        bucket_name,
        BucketPolicy::new(Visibility::Private, None, None, [])
            .map_err(|error| ApiError::bad_request(error.to_string()))?,
        OffsetDateTime::now_utc(),
    )
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    state
        .repository
        .create_bucket(&bucket)
        .await
        .map_err(ApiError::from_repository)?;
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "bucket.created",
        "bucket",
        bucket.id().to_string(),
        serde_json::json!({ "name": bucket.name(), "visibility": Visibility::Private }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(BucketResponse::from(bucket))).into_response())
}

async fn delete_path_bucket(
    State(state): State<Arc<AppState>>,
    Path((app_id, bucket_name)): Path<(String, String)>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticated_path_application(
        &state,
        &headers,
        hmac_identity.map(|identity| identity.0),
        &app_id,
    )
    .await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("bucket:manage")?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("bucket not found"))?;
    if !state
        .repository
        .delete_empty_bucket(auth.application.id, &bucket_name)
        .await
        .map_err(ApiError::from_repository)?
    {
        return Err(ApiError::conflict("bucket is not empty"));
    }
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "bucket.deleted",
        "bucket",
        bucket.id().to_string(),
        serde_json::json!({ "name": bucket.name() }),
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

struct ObjectUpload<'a> {
    bucket_name: &'a str,
    object_key: &'a str,
    headers: &'a HeaderMap,
    content: Bytes,
    visibility_override: Option<Visibility>,
    request_id: &'a str,
    protocol: &'a str,
}

async fn upload_object_content(
    state: &AppState,
    auth: &ApplicationAuth,
    upload: ObjectUpload<'_>,
) -> Result<Media, ApiError> {
    let ObjectUpload {
        bucket_name,
        object_key,
        headers,
        content,
        visibility_override,
        request_id,
        protocol,
    } = upload;
    auth.authorize("media:upload")?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, bucket_name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("bucket not found"))?;
    let display_name = object_key
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| ApiError::bad_request("object key is invalid"))?
        .to_owned();
    let extension = display_name
        .rsplit_once('.')
        .and_then(|(_, extension)| (!extension.is_empty()).then(|| extension.to_owned()));
    let mime = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            mime_guess::from_path(object_key)
                .first_raw()
                .unwrap_or("application/octet-stream")
                .to_owned()
        });
    let receipt = UploadMediaService::new(
        state.object_store.clone(),
        state.repository.clone(),
        state.repository.clone(),
        SystemClock,
    )
    .upload(&UploadMediaRequest {
        application_id: auth.application.id,
        bucket_id: bucket.id(),
        object_key: object_key.to_owned(),
        original_name: Some(display_name.clone()),
        display_name,
        extension,
        mime,
        content: content.to_vec(),
        visibility_override,
        expire_at: None,
        metadata: ClientMetadata::default(),
    })
    .await
    .map_err(ApiError::from_application)?;
    state
        .http_metrics
        .uploaded_bytes
        .fetch_add(receipt.media.size(), Ordering::Relaxed);
    record_audit(
        state,
        auth,
        request_id,
        "media.uploaded",
        "media",
        receipt.media.id().to_string(),
        serde_json::json!({
            "bucket": bucket.name(),
            "object_key": receipt.media.object_key(),
            "size": receipt.media.size(),
            "protocol": protocol,
        }),
    )
    .await;
    Ok(receipt.media)
}

async fn put_path_object(
    State(state): State<Arc<AppState>>,
    Path((app_id, bucket_name, object_key)): Path<(String, String, String)>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
    content: Bytes,
) -> Result<Response, ApiError> {
    let auth = authenticated_path_application(
        &state,
        &headers,
        hmac_identity.map(|identity| identity.0),
        &app_id,
    )
    .await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    let media = upload_object_content(
        &state,
        &auth,
        ObjectUpload {
            bucket_name: &bucket_name,
            object_key: &object_key,
            headers: &headers,
            content,
            visibility_override: None,
            request_id: &request_id.0.0,
            protocol: "path_api",
        },
    )
    .await?;
    let mut response = StatusCode::CREATED.into_response();
    response.headers_mut().insert(
        ETAG,
        HeaderValue::from_str(&format!("\"{}\"", media.etag()))
            .map_err(|_| ApiError::unavailable("object ETag is invalid"))?,
    );
    response.headers_mut().insert(
        axum::http::header::LOCATION,
        HeaderValue::from_str(&object_content_path(&app_id, &bucket_name, &object_key))
            .map_err(|_| ApiError::unavailable("object location is invalid"))?,
    );
    response.headers_mut().insert(
        HeaderName::from_static("x-mediahub-media-id"),
        HeaderValue::from_str(&media.id().to_string())
            .map_err(|_| ApiError::unavailable("media identifier is invalid"))?,
    );
    Ok(response)
}

async fn delete_path_object(
    State(state): State<Arc<AppState>>,
    Path((app_id, bucket_name, object_key)): Path<(String, String, String)>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
) -> Result<StatusCode, ApiError> {
    let auth = authenticated_path_application(
        &state,
        &headers,
        hmac_identity.map(|identity| identity.0),
        &app_id,
    )
    .await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("media:delete")?;
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
        .filter(|media| media.state().is_readable())
        .ok_or_else(|| ApiError::not_found("object not found"))?;
    let now = OffsetDateTime::now_utc();
    let event = OutboxEvent::media_delete_scheduled(&media, now, "path_api");
    let media = state
        .repository
        .schedule_delete(media.id(), now, event)
        .await
        .map_err(ApiError::from_repository)?;
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "media.delete_scheduled",
        "media",
        media.id().to_string(),
        serde_json::json!({ "reason": "path_api", "object_key": object_key }),
    )
    .await;
    Ok(StatusCode::ACCEPTED)
}

async fn authenticated_path_application(
    state: &AppState,
    headers: &HeaderMap,
    hmac_identity: Option<HmacIdentity>,
    app_id: &str,
) -> Result<ApplicationAuth, ApiError> {
    let path_application = state
        .repository
        .find_application_by_app_id(app_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("application not found"))?;
    let auth = authenticated_application(state, headers, hmac_identity).await?;
    if auth.application.id != path_application.id {
        return Err(ApiError::not_found("application not found"));
    }
    Ok(auth)
}

fn object_content_path(app_id: &str, bucket: &str, object_key: &str) -> String {
    let mut url = Url::parse("http://mediahub.invalid").expect("static content URL base is valid");
    {
        let mut segments = url
            .path_segments_mut()
            .expect("HTTP URLs support path segments");
        segments.clear().push(app_id).push(bucket);
        for segment in object_key.split('/') {
            segments.push(segment);
        }
    }
    url.path().to_owned()
}

