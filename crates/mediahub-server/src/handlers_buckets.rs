// Bucket handlers.

// Bucket, media, and upload-session handlers.

async fn list_buckets(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<Vec<BucketResponse>>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
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

async fn get_bucket(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<BucketResponse>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.authorize("bucket:list")?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("bucket not found"))?;
    Ok(Json(BucketResponse::from(bucket)))
}

async fn update_bucket(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
    Json(request): Json<UpdateBucketRequest>,
) -> Result<Json<BucketResponse>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("bucket:manage")?;
    if !request.has_changes() {
        return Err(ApiError::bad_request(
            "at least one bucket policy field is required",
        ));
    }
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("bucket not found"))?;
    let current_policy = bucket.policy();
    let policy = BucketPolicy::new(
        request.visibility.unwrap_or(current_policy.visibility()),
        request
            .default_ttl_seconds
            .unwrap_or(current_policy.default_ttl_seconds()),
        request
            .max_object_size
            .unwrap_or(current_policy.max_object_size()),
        request.allowed_mime_types.unwrap_or_else(|| {
            current_policy
                .allowed_mime_types()
                .map(str::to_owned)
                .collect()
        }),
    )
    .and_then(|policy| {
        policy.with_lifecycle_rules(
            request
                .lifecycle_rules
                .unwrap_or_else(|| current_policy.lifecycle_rules().to_vec()),
        )
    })
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let now = OffsetDateTime::now_utc();
    let updated = state
        .repository
        .update_bucket_policy(auth.application.id, &name, &policy, now)
        .await
        .map_err(ApiError::from_repository)?;
    if !updated {
        return Err(ApiError::not_found("bucket not found"));
    }
    let mut bucket = bucket;
    bucket.update_policy(policy, now);
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "bucket.updated",
        "bucket",
        bucket.id().to_string(),
        serde_json::json!({
            "name": bucket.name(),
            "visibility": bucket.policy().visibility(),
            "default_ttl_seconds": bucket.policy().default_ttl_seconds(),
            "max_object_size": bucket.policy().max_object_size(),
            "allowed_mime_types": bucket.policy().allowed_mime_types().collect::<Vec<_>>(),
            "lifecycle_rules": bucket.policy().lifecycle_rules(),
        }),
    )
    .await;
    Ok(Json(BucketResponse::from(bucket)))
}

async fn delete_bucket(
    State(state): State<Arc<AppState>>,
    Path(name): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    request_id: Extension<RequestId>,
) -> Result<StatusCode, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("bucket:manage")?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &name)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("bucket not found"))?;
    let deleted = state
        .repository
        .delete_empty_bucket(auth.application.id, &name)
        .await
        .map_err(ApiError::from_repository)?;
    if !deleted {
        return Err(ApiError::not_found("bucket not found"));
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

async fn create_bucket(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    hmac_context: Option<Extension<HmacRequestContext>>,
    request_id: Extension<RequestId>,
    Json(request): Json<CreateBucketRequest>,
) -> Result<Response, ApiError> {
    let is_hmac = hmac_identity.is_some();
    let auth = authenticated_application(
        &state,
        &headers,
        hmac_identity.as_ref().map(|value| value.0.clone()),
    )
    .await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("bucket:manage")?;
    let visibility = request.visibility.unwrap_or(Visibility::Private);
    let policy = BucketPolicy::new(
        visibility,
        request.default_ttl_seconds,
        request.max_object_size,
        request.allowed_mime_types,
    )
    .and_then(|policy| policy.with_lifecycle_rules(request.lifecycle_rules))
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let bucket = Bucket::new(
        BucketId::new(),
        auth.application.id,
        request.name,
        policy,
        OffsetDateTime::now_utc(),
    )
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let bucket_response = BucketResponse::from(bucket.clone());
    let response_payload = serde_json::to_string(&bucket_response)
        .map_err(|_| ApiError::unavailable("failed to encode idempotency response"))?;
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
            "Idempotency-Key is required for HMAC bucket creation",
        ));
    }
    if let Some(mut idempotency) = idempotency {
        let now = OffsetDateTime::now_utc();
        match state
            .repository
            .claim_idempotency_key(
                idempotency.application_id,
                &idempotency.operation_scope,
                &idempotency.key,
                &idempotency.request_hash,
                now + time::Duration::seconds(IDEMPOTENCY_SECONDS),
                now,
            )
            .await
            .map_err(ApiError::from_repository)?
        {
            IdempotencyClaim::Completed(response) => return idempotency_response(response),
            IdempotencyClaim::InProgress => {
                return Err(ApiError::idempotency_in_progress());
            }
            IdempotencyClaim::Conflict => return Err(ApiError::idempotency_conflict()),
            IdempotencyClaim::Claimed(claim_token) => idempotency.claim_token = claim_token,
        }
        let completed_response = CompletedIdempotencyResponse {
            status: StatusCode::CREATED.as_u16(),
            payload: response_payload,
            resource_id: Some(bucket.id().to_string()),
        };
        if let Err(error) = state
            .repository
            .create_bucket_and_complete_idempotency(&bucket, &idempotency, &completed_response, now)
            .await
        {
            let _ = state.repository.release_idempotency_key(&idempotency).await;
            return Err(ApiError::from_repository(error));
        }
    } else {
        state
            .repository
            .create_bucket(&bucket)
            .await
            .map_err(ApiError::from_repository)?;
    }
    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "bucket.created",
        "bucket",
        bucket.id().to_string(),
        serde_json::json!({ "name": bucket.name(), "visibility": bucket.policy().visibility() }),
    )
    .await;
    Ok((StatusCode::CREATED, Json(bucket_response)).into_response())
}

