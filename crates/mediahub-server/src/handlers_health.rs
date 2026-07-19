// Health and service capability handlers.

// Health, administration, authentication, and application handlers.

async fn liveness() -> Json<StatusResponse> {
    Json(StatusResponse { status: "ok" })
}

async fn readiness(State(state): State<Arc<AppState>>) -> Result<Json<StatusResponse>, ApiError> {
    state
        .repository
        .health_check()
        .await
        .map_err(|_| ApiError::unavailable("database is unavailable"))?;
    state
        .object_store
        .health_check()
        .await
        .map_err(|_| ApiError::unavailable("object storage is unavailable"))?;
    validate_storage_database_consistency(&state.repository, &state.object_store)
        .await
        .map_err(|error| {
            warn!(error = %error, "storage/database readiness check failed");
            ApiError::unavailable("object storage does not match database")
        })?;
    Ok(Json(StatusResponse { status: "ok" }))
}

fn storage_capacity(object_store: &RuntimeObjectStore) -> Result<(u64, u64), ApiError> {
    let Some(root) = object_store.local_root() else {
        return Ok((0, 0));
    };
    let total = fs2::total_space(root)
        .map_err(|_| ApiError::unavailable("storage capacity is unavailable"))?;
    let available = fs2::available_space(root)
        .map_err(|_| ApiError::unavailable("storage capacity is unavailable"))?;
    Ok((total, available))
}

async fn metrics(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let bearer_is_valid = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .zip(state.metrics_bearer_token.as_deref())
        .is_some_and(|(supplied, expected)| constant_time_eq(supplied, expected));
    if !bearer_is_valid {
        require_admin(&state, &headers).await?;
    }
    let snapshot = state
        .repository
        .admin_metrics_snapshot()
        .await
        .map_err(ApiError::from_repository)?;
    let requests = state.http_metrics.requests.load(Ordering::Relaxed);
    let errors = state.http_metrics.errors.load(Ordering::Relaxed);
    let duration_seconds =
        state.http_metrics.duration_micros.load(Ordering::Relaxed) as f64 / 1_000_000.0;
    let (disk_total_bytes, disk_available_bytes) = storage_capacity(&state.object_store)?;
    let body = format!(
        concat!(
            "# HELP mediahub_http_requests_total Total HTTP responses.\n",
            "# TYPE mediahub_http_requests_total counter\n",
            "mediahub_http_requests_total {requests}\n",
            "# HELP mediahub_http_errors_total Total HTTP 4xx and 5xx responses.\n",
            "# TYPE mediahub_http_errors_total counter\n",
            "mediahub_http_errors_total {errors}\n",
            "# HELP mediahub_http_request_duration_seconds Aggregate HTTP response duration.\n",
            "# TYPE mediahub_http_request_duration_seconds summary\n",
            "mediahub_http_request_duration_seconds_count {requests}\n",
            "mediahub_http_request_duration_seconds_sum {duration_seconds}\n",
            "# TYPE mediahub_async_jobs_pending gauge\n",
            "mediahub_async_jobs_pending {pending_jobs}\n",
            "# TYPE mediahub_async_jobs_running gauge\n",
            "mediahub_async_jobs_running {running_jobs}\n",
            "# TYPE mediahub_outbox_pending gauge\n",
            "mediahub_outbox_pending {pending_outbox}\n",
            "# TYPE mediahub_webhook_deliveries_pending gauge\n",
            "mediahub_webhook_deliveries_pending {pending_webhook_deliveries}\n",
            "# TYPE mediahub_deletions_pending gauge\n",
            "mediahub_deletions_pending {pending_deletions}\n",
            "# TYPE mediahub_upload_bytes_total counter\n",
            "mediahub_upload_bytes_total {uploaded_bytes}\n",
            "# TYPE mediahub_variant_cache_hits_total counter\n",
            "mediahub_variant_cache_hits_total {variant_cache_hits}\n",
            "# TYPE mediahub_variant_cache_misses_total counter\n",
            "mediahub_variant_cache_misses_total {variant_cache_misses}\n",
            "# TYPE mediahub_storage_quota_bytes gauge\n",
            "mediahub_storage_quota_bytes {quota_bytes}\n",
            "# TYPE mediahub_storage_used_bytes gauge\n",
            "mediahub_storage_used_bytes {used_bytes}\n",
            "# TYPE mediahub_storage_reserved_bytes gauge\n",
            "mediahub_storage_reserved_bytes {reserved_bytes}\n",
            "# TYPE mediahub_storage_media_objects gauge\n",
            "mediahub_storage_media_objects {media_objects}\n",
            "# TYPE mediahub_storage_variant_bytes gauge\n",
            "mediahub_storage_variant_bytes {variant_bytes}\n",
            "# TYPE mediahub_storage_disk_total_bytes gauge\n",
            "mediahub_storage_disk_total_bytes {disk_total_bytes}\n",
            "# TYPE mediahub_storage_disk_available_bytes gauge\n",
            "mediahub_storage_disk_available_bytes {disk_available_bytes}\n"
        ),
        requests = requests,
        errors = errors,
        duration_seconds = duration_seconds,
        pending_jobs = snapshot.pending_jobs,
        running_jobs = snapshot.running_jobs,
        pending_outbox = snapshot.pending_outbox,
        pending_webhook_deliveries = snapshot.pending_webhook_deliveries,
        pending_deletions = snapshot.pending_deletions,
        uploaded_bytes = state.http_metrics.uploaded_bytes.load(Ordering::Relaxed),
        variant_cache_hits = state
            .http_metrics
            .variant_cache_hits
            .load(Ordering::Relaxed),
        variant_cache_misses = state
            .http_metrics
            .variant_cache_misses
            .load(Ordering::Relaxed),
        quota_bytes = snapshot.storage.quota_bytes,
        used_bytes = snapshot.storage.used_bytes,
        reserved_bytes = snapshot.storage.reserved_bytes,
        media_objects = snapshot.storage.media_objects,
        variant_bytes = snapshot.storage.variant_bytes,
        disk_total_bytes = disk_total_bytes,
        disk_available_bytes = disk_available_bytes,
    );
    Ok((
        StatusCode::OK,
        [(CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
        .into_response())
}

async fn capabilities() -> Json<CapabilitiesResponse> {
    Json(CapabilitiesResponse {
        deployment_profile: "docker",
        storage: ["local", "s3"],
        s3_gateway: true,
        image_processing: true,
        video_processing: false,
        resumable_upload: false,
        archive_restore: false,
    })
}

