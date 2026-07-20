// Batch and asynchronous job handlers.

async fn batch_media(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    hmac_context: Option<Extension<HmacRequestContext>>,
    request_id: Extension<RequestId>,
    Json(request): Json<BatchMediaRequest>,
) -> Result<Response, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    match &request.action {
        AsyncJobAction::Delete => auth.authorize("media:delete")?,
        _ => auth.authorize("media:update")?,
    }
    if request.media_ids.is_empty() || request.media_ids.len() > MAX_BATCH_ITEMS {
        return Err(ApiError::bad_request(format!(
            "media_ids must contain between 1 and {MAX_BATCH_ITEMS} items"
        )));
    }
    if matches!(
        request.action,
        AsyncJobAction::UpdateTtlSeconds {
            ttl_seconds: Some(0)
        }
    ) {
        return Err(ApiError::bad_request(
            "ttl_seconds must be greater than zero or null",
        ));
    }
    let media_ids = request
        .media_ids
        .iter()
        .map(|value| {
            MediaId::from_str(value).map_err(|_| ApiError::bad_request("media ID is invalid"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if media_ids.iter().copied().collect::<HashSet<_>>().len() != media_ids.len() {
        return Err(ApiError::bad_request("media_ids contains duplicates"));
    }
    for media_id in &media_ids {
        let owned = state
            .repository
            .find_media_by_id(*media_id)
            .await
            .map_err(ApiError::from_repository)?
            .is_some_and(|media| media.application_id() == auth.application.id);
        if !owned {
            return Err(ApiError::not_found("media not found"));
        }
    }

    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 255)
        .ok_or_else(|| ApiError::bad_request("a valid Idempotency-Key is required"))?
        .to_owned();
    let request_hash = hmac_context.map_or_else(
        || {
            let bytes = serde_json::to_vec(&request).expect("batch request serializes");
            hex::encode(Sha256::digest(bytes))
        },
        |context| context.0.request_hash,
    );

    if media_ids.len() > SYNC_BATCH_LIMIT {
        let receipt = AsyncJobService::new(state.repository.clone(), SystemClock)
            .create(&CreateAsyncJobRequest {
                application_id: auth.application.id,
                operation_scope: "media.batch".to_owned(),
                idempotency_key,
                request_hash,
                request_id: Some(request_id.0.0),
                action: request.action,
                media_ids,
                max_attempts: mediahub_app::DEFAULT_ASYNC_JOB_MAX_ATTEMPTS,
            })
            .await
            .map_err(ApiError::from_async_job)?;
        return Ok((
            StatusCode::ACCEPTED,
            Json(AsyncJobReceiptResponse::from(receipt)),
        )
            .into_response());
    }

    let now = OffsetDateTime::now_utc();
    let mut idempotency = IdempotencyContext {
        application_id: auth.application.id,
        operation_scope: "media.batch.sync".to_owned(),
        key: idempotency_key.clone(),
        request_hash: request_hash.clone(),
        claim_token: String::new(),
    };
    match state
        .repository
        .claim_idempotency_key(
            auth.application.id,
            "media.batch.sync",
            &idempotency_key,
            &request_hash,
            now + time::Duration::seconds(IDEMPOTENCY_SECONDS),
            now,
        )
        .await
        .map_err(ApiError::from_repository)?
    {
        IdempotencyClaim::Completed(response) => return idempotency_response(response),
        IdempotencyClaim::InProgress => return Err(ApiError::idempotency_in_progress()),
        IdempotencyClaim::Conflict => return Err(ApiError::idempotency_conflict()),
        IdempotencyClaim::Claimed(claim_token) => idempotency.claim_token = claim_token,
    }

    let mut results = Vec::with_capacity(media_ids.len());
    for media_id in media_ids {
        match execute_batch_action(
            &state.repository,
            auth.application.id,
            media_id,
            &request.action,
            now,
        )
        .await
        {
            Ok(result) => results.push(BatchItemResponse {
                media_id: media_id.to_string(),
                state: "succeeded",
                result: Some(result),
                error: None,
            }),
            Err(error) => results.push(BatchItemResponse {
                media_id: media_id.to_string(),
                state: "failed",
                result: None,
                error: Some(BatchItemErrorResponse {
                    code: error.code,
                    message: error.summary,
                }),
            }),
        }
    }
    let response = BatchMediaResponse { results };
    let payload = serde_json::to_string(&response)
        .map_err(|_| ApiError::unavailable("batch response could not be encoded"))?;
    state
        .repository
        .complete_idempotency_key(
            auth.application.id,
            "media.batch.sync",
            &idempotency_key,
            &request_hash,
            &idempotency.claim_token,
            &CompletedIdempotencyResponse {
                status: StatusCode::OK.as_u16(),
                payload: payload.clone(),
                resource_id: None,
            },
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(ApiError::from_repository)?;
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "media.batch_completed",
        "media_batch",
        idempotency_key,
        serde_json::json!({ "item_count": response.results.len() }),
    )
    .await;
    idempotency_response(CompletedIdempotencyResponse {
        status: StatusCode::OK.as_u16(),
        payload,
        resource_id: None,
    })
}

async fn get_async_job(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<AsyncJobDetailsResponse>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.authorize("media:list")?;
    let job_id = AsyncJobId::from_str(&job_id).map_err(|_| ApiError::not_found("job not found"))?;
    let details = AsyncJobService::new(state.repository.clone(), SystemClock)
        .get(auth.application.id, job_id)
        .await
        .map_err(ApiError::from_async_job)?;
    Ok(Json(details.into()))
}

async fn cancel_async_job(
    State(state): State<Arc<AppState>>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<AsyncJobResponse>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("media:update")?;
    let job_id = AsyncJobId::from_str(&job_id).map_err(|_| ApiError::not_found("job not found"))?;
    let job = AsyncJobService::new(state.repository.clone(), SystemClock)
        .cancel(&CancelAsyncJobRequest {
            application_id: auth.application.id,
            job_id,
        })
        .await
        .map_err(ApiError::from_async_job)?;
    Ok(Json(job.into()))
}

