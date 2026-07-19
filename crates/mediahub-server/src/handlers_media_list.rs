// Media listing and signed path operations.

fn idempotency_response(response: CompletedIdempotencyResponse) -> Result<Response, ApiError> {
    let status = StatusCode::from_u16(response.status)
        .map_err(|_| ApiError::unavailable("stored idempotency response is invalid"))?;
    serde_json::from_str::<serde_json::Value>(&response.payload)
        .map_err(|_| ApiError::unavailable("stored idempotency response is invalid"))?;
    let mut replay = (status, response.payload).into_response();
    replay.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    Ok(replay)
}

async fn list_media(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListMediaQuery>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<MediaListResponse>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.authorize("media:list")?;
    let limit = query.limit.unwrap_or(50);
    if limit == 0 || limit > 100 {
        return Err(ApiError::bad_request("limit must be between 1 and 100"));
    }
    let directory_mode = parse_list_delimiter(query.delimiter.as_deref())?;
    let bucket_id = match query.bucket {
        Some(name) => Some(
            state
                .repository
                .find_bucket_by_name(auth.application.id, name.trim())
                .await
                .map_err(ApiError::from_repository)?
                .ok_or_else(|| ApiError::not_found("bucket not found"))?
                .id(),
        ),
        None => None,
    };
    if directory_mode && bucket_id.is_none() {
        return Err(ApiError::bad_request("delimiter requires a bucket filter"));
    }
    let state_filter = query.status.as_deref().map(parse_media_state).transpose()?;
    let mime = query
        .mime
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    if mime.as_ref().is_some_and(|value| value.len() > 255) {
        return Err(ApiError::bad_request("mime filter is too long"));
    }
    let object_key_prefix = query.prefix.filter(|value| !value.is_empty());
    if object_key_prefix.as_ref().is_some_and(|value| {
        value.len() > 1024 || value.bytes().any(|byte| byte.is_ascii_control())
    }) {
        return Err(ApiError::bad_request("prefix filter is invalid"));
    }
    let created_from = query
        .created_from
        .as_deref()
        .map(parse_query_time)
        .transpose()?;
    let created_before = query
        .created_before
        .as_deref()
        .map(parse_query_time)
        .transpose()?;
    if created_from
        .zip(created_before)
        .is_some_and(|(from, before)| from >= before)
    {
        return Err(ApiError::bad_request(
            "created_from must precede created_before",
        ));
    }
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
                    bucket_id: bucket_id.expect("directory mode requires a bucket"),
                    state: state_filter,
                    mime,
                    created_from,
                    created_before,
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
                bucket_id,
                state: state_filter,
                mime,
                created_from,
                created_before,
                object_key_prefix,
                cursor,
                limit,
            },
        )
        .await
        .map_err(ApiError::from_repository)?;
    let next_cursor = if page.has_more {
        page.items.last().map(|media| {
            encode_media_cursor(MediaListCursor {
                created_at: media.created_at(),
                id: media.id(),
            })
        })
    } else {
        None
    };
    Ok(Json(MediaListResponse {
        items: page.items.into_iter().map(MediaResponse::from).collect(),
        common_prefixes: Vec::new(),
        next_cursor,
    }))
}

