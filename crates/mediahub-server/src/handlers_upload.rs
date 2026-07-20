// Upload and upload-session handlers.

async fn upload_media(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
    multipart: Multipart,
) -> Result<(StatusCode, Json<MediaResponse>), ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("media:upload")?;
    let form = parse_upload(multipart).await?;
    let bucket_id = state
        .repository
        .find_bucket_by_name(auth.application.id, &form.bucket)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("bucket not found"))?
        .id();
    let expire_at = form
        .ttl_seconds
        .map(|seconds| expiration_from_ttl(seconds, OffsetDateTime::now_utc()))
        .transpose()?;
    let service = UploadMediaService::new(
        state.object_store.clone(),
        state.repository.clone(),
        state.repository.clone(),
        SystemClock,
    );
    let receipt = service
        .upload(&UploadMediaRequest {
            application_id: auth.application.id,
            bucket_id,
            object_key: form.object_key,
            original_name: form.original_name,
            display_name: form.display_name,
            extension: form.extension,
            mime: form.mime,
            content: form.content,
            visibility_override: form.visibility_override,
            expire_at,
            metadata: form.metadata,
        })
        .await
        .map_err(ApiError::from_application)?;
    state
        .http_metrics
        .uploaded_bytes
        .fetch_add(receipt.media.size(), Ordering::Relaxed);
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "media.uploaded",
        "media",
        receipt.media.id().to_string(),
        serde_json::json!({
            "bucket_id": receipt.media.bucket_id().to_string(),
            "object_key": receipt.media.object_key(),
            "size_bytes": receipt.media.size(),
            "mime": receipt.media.mime(),
        }),
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(MediaResponse::from(receipt.media)),
    ))
}

async fn create_upload_session(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    hmac_context: Option<Extension<HmacRequestContext>>,
    request_id: Extension<RequestId>,
    Json(request): Json<CreateUploadSessionHttpRequest>,
) -> Result<Response, ApiError> {
    let is_hmac = hmac_identity.is_some();
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("media:upload")?;
    let idempotency = hmac_context
        .map(|value| value.0)
        .filter(|context| context.idempotency_key.is_some())
        .map(|context| IdempotencyContext {
            application_id: auth.application.id,
            operation_scope: context.operation_scope,
            key: context.idempotency_key.expect("filtered idempotency key"),
            request_hash: context.request_hash,
            claim_token: String::new(),
        });
    if is_hmac && idempotency.is_none() {
        return Err(ApiError::bad_request(
            "Idempotency-Key is required for HMAC upload session creation",
        ));
    }
    validate_upload_expected_size(request.expected_size)?;
    let bucket_id = state
        .repository
        .find_bucket_by_name(auth.application.id, request.bucket.trim())
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("bucket not found"))?
        .id();
    let expected_mime = normalized_mime(&request.content_type)?;
    let original_name = request.original_name;
    let object_key = request
        .object_key
        .unwrap_or_else(|| generated_object_key(original_name.as_deref()));
    let display_name = request
        .display_name
        .or_else(|| original_name.clone())
        .unwrap_or_else(|| object_key.clone());
    let extension = request.extension.or_else(|| {
        original_name
            .as_deref()
            .and_then(|name| name.rsplit_once('.').map(|(_, value)| value.to_owned()))
    });
    let media_expires_at = request
        .ttl_seconds
        .map(|seconds| {
            if seconds == 0 {
                Err(ApiError::bad_request(
                    "ttl_seconds must be greater than zero",
                ))
            } else {
                expiration_from_ttl(seconds, OffsetDateTime::now_utc())
            }
        })
        .transpose()?;
    let metadata = request
        .metadata
        .map(ClientMetadata::from_value)
        .transpose()
        .map_err(|error| ApiError::bad_request(error.to_string()))?
        .unwrap_or_default();
    let create_request = CreateUploadSessionRequest {
        application_id: auth.application.id,
        bucket_id,
        object_key,
        original_name,
        display_name,
        extension,
        expected_size: request.expected_size,
        expected_mime,
        visibility_override: request.visibility,
        media_expires_at,
        metadata,
    };
    let mut idempotency = idempotency;
    if let Some(idempotency_context) = &mut idempotency {
        let now = OffsetDateTime::now_utc();
        match state
            .repository
            .claim_idempotency_key(
                idempotency_context.application_id,
                &idempotency_context.operation_scope,
                &idempotency_context.key,
                &idempotency_context.request_hash,
                now + time::Duration::seconds(IDEMPOTENCY_SECONDS),
                now,
            )
            .await
            .map_err(ApiError::from_repository)?
        {
            IdempotencyClaim::Completed(response) => return idempotency_response(response),
            IdempotencyClaim::InProgress => return Err(ApiError::idempotency_in_progress()),
            IdempotencyClaim::Conflict => return Err(ApiError::idempotency_conflict()),
            IdempotencyClaim::Claimed(claim_token) => idempotency_context.claim_token = claim_token,
        }
    }
    let service = upload_session_service(&state);
    let receipt_result = if idempotency.is_some() {
        service.prepare(&create_request).await
    } else {
        service.create(&create_request).await
    };
    let receipt = match receipt_result {
        Ok(receipt) => receipt,
        Err(error) => {
            if let Some(idempotency) = &idempotency {
                let _ = state.repository.release_idempotency_key(idempotency).await;
            }
            return Err(ApiError::from_application(error));
        }
    };
    let url = client_upload_target_url(
        &state,
        &receipt.session,
        receipt.target.url,
        receipt.target.expires_at,
    );
    let response = CreateUploadSessionResponse {
        upload_id: receipt.session.id().to_string(),
        media_id: receipt.session.media_id().to_string(),
        bucket_id: receipt.session.bucket_id().to_string(),
        object_key: receipt.session.object_key().to_owned(),
        expected_size: receipt.session.expected_size(),
        expected_mime: receipt.session.expected_mime().to_owned(),
        method: receipt.target.method,
        url,
        headers: receipt.target.headers,
        expires_at: receipt.target.expires_at,
    };
    if let Some(idempotency) = &idempotency {
        let response_payload = serde_json::to_string(&response)
            .map_err(|_| ApiError::unavailable("failed to encode idempotency response"))?;
        let completed_response = CompletedIdempotencyResponse {
            status: StatusCode::CREATED.as_u16(),
            payload: response_payload,
            resource_id: Some(receipt.session.id().to_string()),
        };
        if let Err(error) = state
            .repository
            .create_upload_session_and_complete_idempotency(
                &receipt.session,
                idempotency,
                &completed_response,
                OffsetDateTime::now_utc(),
            )
            .await
        {
            let _ = state.repository.release_idempotency_key(idempotency).await;
            return Err(ApiError::from_repository(error));
        }
    }
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "upload_session.created",
        "upload_session",
        response.upload_id.clone(),
        serde_json::json!({
            "bucket_id": response.bucket_id.clone(),
            "object_key": response.object_key.clone(),
            "expected_size": response.expected_size,
            "expected_mime": response.expected_mime.clone(),
        }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn get_upload_session(
    State(state): State<Arc<AppState>>,
    Path(upload_session_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<UploadSessionResponse>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.authorize("media:upload")?;
    let upload_session_id = UploadSessionId::from_str(&upload_session_id)
        .map_err(|_| ApiError::not_found("upload session not found"))?;
    let session = state
        .repository
        .find_upload_session(upload_session_id)
        .await
        .map_err(ApiError::from_repository)?
        .filter(|session| session.application_id() == auth.application.id)
        .ok_or_else(|| ApiError::not_found("upload session not found"))?;
    let now = OffsetDateTime::now_utc();
    let upload_target =
        if session.state() == UploadSessionState::Pending && !session.is_expired_at(now) {
            let prepared = state
                .object_store
                .prepare_upload(
                    session.id(),
                    session.media_id(),
                    session.expected_size(),
                    session.expected_mime(),
                    session.session_expires_at(),
                )
                .await
                .map_err(|_| ApiError::unavailable("upload target is unavailable"))?;
            if prepared.storage_backend != session.storage_backend()
                || prepared.storage_key != session.storage_key()
            {
                return Err(ApiError::unavailable(
                    "upload target does not match the durable session",
                ));
            }
            Some(UploadTargetResponse::from_target(
                &state,
                &session,
                prepared.target,
            ))
        } else {
            None
        };
    Ok(Json(UploadSessionResponse::from_session(
        &session,
        upload_target,
    )))
}

async fn put_upload_content(
    State(state): State<Arc<AppState>>,
    Path(upload_session_id): Path<String>,
    Query(query): Query<UploadContentQuery>,
    headers: HeaderMap,
    body: Body,
) -> Result<StatusCode, ApiError> {
    let upload_session_id = UploadSessionId::from_str(&upload_session_id)
        .map_err(|_| ApiError::not_found("upload session not found"))?;
    let token = query
        .token
        .as_deref()
        .ok_or_else(|| ApiError::not_found("upload session not found"))?;
    state
        .media_url_signer
        .verify_upload_content(token, upload_session_id, OffsetDateTime::now_utc())
        .map_err(|_| ApiError::not_found("upload session not found"))?;
    let session = state
        .repository
        .find_upload_session(upload_session_id)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("upload session not found"))?;
    if session.storage_backend() != "local" || state.object_store.backend_name() != "local" {
        return Err(ApiError::not_found("upload session not found"));
    }
    if session.state() != UploadSessionState::Pending
        || session.is_expired_at(OffsetDateTime::now_utc())
    {
        return Err(ApiError::conflict("upload session is not writable"));
    }
    let content_length = header_value(&headers, "content-length")?
        .parse::<u64>()
        .map_err(|_| ApiError::bad_request("Content-Length is invalid"))?;
    if content_length != session.expected_size() {
        return Err(ApiError::unprocessable(
            "Content-Length does not match expected_size",
        ));
    }
    let content_type = normalized_mime(header_value(&headers, "content-type")?)?;
    if content_type != session.expected_mime() {
        return Err(ApiError::unsupported_media_type(
            "Content-Type does not match the upload session",
        ));
    }
    let local_store = state
        .object_store
        .local_store()
        .ok_or_else(|| ApiError::not_found("upload session not found"))?;
    match local_store
        .put_temporary_stream(
            session.storage_key(),
            body.into_data_stream(),
            session.expected_size(),
            session.expected_mime(),
        )
        .await
    {
        Ok(()) => {
            let current = state
                .repository
                .find_upload_session(session.id())
                .await
                .map_err(ApiError::from_repository)?
                .ok_or_else(|| ApiError::not_found("upload session not found"))?;
            if current.state() != UploadSessionState::Pending
                || current.is_expired_at(OffsetDateTime::now_utc())
            {
                let _ = state.object_store.abort_upload(&current).await;
                return Err(ApiError::conflict("upload session is not writable"));
            }
            state
                .http_metrics
                .uploaded_bytes
                .fetch_add(session.expected_size(), Ordering::Relaxed);
            Ok(StatusCode::NO_CONTENT)
        }
        Err(LocalUploadError::Storage(ObjectStoreError::AlreadyExists)) => {
            let stored = state
                .object_store
                .inspect_upload(&session)
                .await
                .map_err(|_| ApiError::conflict("upload content already exists"))?;
            if stored.size == session.expected_size() && stored.mime == session.expected_mime() {
                // Completion still compares the client checksum with the stored
                // object, so an idempotent retry cannot activate different bytes.
                Ok(StatusCode::NO_CONTENT)
            } else {
                Err(ApiError::conflict(
                    "different upload content already exists",
                ))
            }
        }
        Err(LocalUploadError::SizeMismatch { .. }) => Err(ApiError::unprocessable(
            "uploaded content does not match expected_size",
        )),
        Err(LocalUploadError::Stream(_)) => Err(ApiError::bad_request("upload body stream failed")),
        Err(LocalUploadError::Storage(error)) => Err(ApiError::from_application(
            ApplicationError::ObjectStore(error),
        )),
    }
}

async fn complete_upload_session(
    State(state): State<Arc<AppState>>,
    Path(upload_session_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
    Json(request): Json<CompleteUploadSessionHttpRequest>,
) -> Result<(StatusCode, Json<CompleteUploadSessionResponse>), ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("media:upload")?;
    let upload_session_id = UploadSessionId::from_str(&upload_session_id)
        .map_err(|_| ApiError::not_found("upload session not found"))?;
    if request.sha256.len() != 64 || !request.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request("sha256 is invalid"));
    }
    let receipt = upload_session_service(&state)
        .complete(&CompleteUploadSessionRequest {
            application_id: auth.application.id,
            upload_session_id,
            sha256: request.sha256,
        })
        .await
        .map_err(ApiError::from_application)?;
    if !receipt.already_completed {
        record_audit(
            &state,
            &auth,
            &request_id.0.0,
            "upload_session.completed",
            "upload_session",
            receipt.session.id().to_string(),
            serde_json::json!({ "media_id": receipt.media.id().to_string() }),
        )
        .await;
    }
    let status = if receipt.already_completed {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(CompleteUploadSessionResponse {
            upload_id: receipt.session.id().to_string(),
            event_id: receipt.event_id,
            already_completed: receipt.already_completed,
            media: MediaResponse::from(receipt.media),
        }),
    ))
}

async fn cancel_upload_session(
    State(state): State<Arc<AppState>>,
    Path(upload_session_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
) -> Result<StatusCode, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("media:upload")?;
    let upload_session_id = UploadSessionId::from_str(&upload_session_id)
        .map_err(|_| ApiError::not_found("upload session not found"))?;
    let receipt = upload_session_service(&state)
        .cancel(&CancelUploadSessionRequest {
            application_id: auth.application.id,
            upload_session_id,
        })
        .await
        .map_err(ApiError::from_application)?;
    if !receipt.already_cancelled {
        record_audit(
            &state,
            &auth,
            &request_id.0.0,
            "upload_session.cancelled",
            "upload_session",
            receipt.session.id().to_string(),
            serde_json::json!({}),
        )
        .await;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn upload_session_service(
    state: &AppState,
) -> UploadSessionService<RuntimeObjectStore, PostgresRepository, PostgresRepository, SystemClock> {
    UploadSessionService::new(
        state.object_store.clone(),
        state.repository.clone(),
        state.repository.clone(),
        SystemClock,
    )
}

