// Background workers and asynchronous batch execution.

const WEBHOOK_DELIVERY_LEASE_SECONDS: i64 = 30;
const WEBHOOK_DNS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const WEBHOOK_ATTEMPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(25);

pub(super) async fn validate_referenced_key_versions(
    repository: &(impl SecretKeyVersionRepository + ?Sized),
    cipher: &AccessKeyCipher,
) -> anyhow::Result<()> {
    for version in repository
        .referenced_secret_key_versions()
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
    {
        if !cipher.supports_version(version) {
            return Err(anyhow::anyhow!(
                "database references unavailable master key version {version}"
            ));
        }
    }
    Ok(())
}

pub(super) async fn validate_storage_database_consistency(
    repository: &PostgresRepository,
    object_store: &RuntimeObjectStore,
) -> Result<(), String> {
    let (used_bytes, storage_keys) = repository
        .storage_consistency_sample(STORAGE_CONSISTENCY_SAMPLE_SIZE)
        .await
        .map_err(|error| format!("failed to inspect stored-object metadata: {error}"))?;
    if used_bytes == 0 {
        return Ok(());
    }
    if storage_keys.is_empty() {
        return Err(format!(
            "database reports {used_bytes} used bytes but has no non-deleted object keys for the configured {} storage backend",
            object_store.backend_name()
        ));
    }
    let sampled_count = storage_keys.len();
    for storage_key in &storage_keys {
        match object_store.exists(storage_key).await {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect a sampled object in the configured {} storage backend: {error}",
                    object_store.backend_name()
                ));
            }
        }
    }
    Err(format!(
        "database reports {used_bytes} used bytes but none of {sampled_count} sampled objects exists in the configured {} storage backend",
        object_store.backend_name()
    ))
}

pub(super) async fn run_lifecycle_worker(repository: PostgresRepository, object_store: RuntimeObjectStore) {
    let upload_sessions = UploadSessionService::new(
        object_store.clone(),
        repository.clone(),
        repository.clone(),
        SystemClock,
    );
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        interval.tick().await;
        let now = OffsetDateTime::now_utc();
        if let Err(error) = upload_sessions.expire_due(100).await {
            warn!(error = %error, "upload session expiry scan failed");
        }
        reconcile_stale_uploads(&repository, &object_store, now).await;
        match repository.expire_multipart_uploads(now, 100).await {
            Ok(expired) => {
                for expired_upload in expired {
                    if s3_multipart_storage::cleanup_multipart_storage(
                        &object_store,
                        &expired_upload.upload.upload_id,
                    )
                    .await
                        && let Err(error) = repository
                            .clear_multipart_parts(&expired_upload.upload.upload_id)
                            .await
                    {
                        warn!(
                            upload_id = %expired_upload.upload.upload_id,
                            error = %error,
                            "failed to clear expired S3 multipart metadata"
                        );
                    }
                }
            }
            Err(error) => warn!(error = %error, "S3 multipart expiry scan failed"),
        }
        match repository.list_expired_media(now, 100).await {
            Ok(media) => {
                for media in media {
                    let event = OutboxEvent::media_delete_scheduled(&media, now, "ttl");
                    if let Err(error) = repository.schedule_delete(media.id(), now, event).await {
                        warn!(media_id = %media.id(), error = %error, "failed to schedule expired media deletion");
                    }
                }
            }
            Err(error) => warn!(error = %error, "lifecycle expiry scan failed"),
        }

        match repository.list_lifecycle_buckets().await {
            Ok(buckets) => {
                for bucket in buckets {
                    let mut due = HashMap::<MediaId, (Media, &'static str)>::new();
                    for rule in bucket.policy().lifecycle_rules() {
                        if !rule.enabled() {
                            continue;
                        }
                        let result = match rule {
                            LifecycleRule::ExpireAfter {
                                prefix,
                                duration_seconds,
                                ..
                            } => {
                                let duration = i64::try_from(*duration_seconds)
                                    .expect("validated lifecycle duration fits i64");
                                let Some(cutoff) =
                                    now.checked_sub(time::Duration::seconds(duration))
                                else {
                                    continue;
                                };
                                repository
                                    .list_expire_after_due(
                                        bucket.application_id(),
                                        bucket.id(),
                                        prefix,
                                        cutoff,
                                        100,
                                    )
                                    .await
                                    .map(|media| (media, "ttl"))
                            }
                            LifecycleRule::KeepLatest { prefix, count, .. } => repository
                                .list_keep_latest_surplus(
                                    bucket.application_id(),
                                    bucket.id(),
                                    prefix,
                                    *count,
                                    100,
                                )
                                .await
                                .map(|media| (media, "keep_latest")),
                        };
                        match result {
                            Ok((media, reason)) => {
                                for media in media {
                                    due.entry(media.id()).or_insert((media, reason));
                                }
                            }
                            Err(error) => {
                                warn!(bucket_id = %bucket.id(), rule_id = rule.id(), error = %error, "bucket lifecycle rule scan failed");
                            }
                        }
                    }
                    for (_, (media, reason)) in due {
                        let event = OutboxEvent::media_delete_scheduled(&media, now, reason);
                        if let Err(error) = repository.schedule_delete(media.id(), now, event).await
                        {
                            warn!(media_id = %media.id(), reason, error = %error, "failed to schedule lifecycle deletion");
                        }
                    }
                }
            }
            Err(error) => warn!(error = %error, "bucket lifecycle scan failed"),
        }

        let pending = repository.list_pending_deletions(100).await;
        match pending {
            Ok(deletions) => {
                for deletion in deletions {
                    let media_id = deletion.media_id;
                    let storage_key = deletion.storage_key;
                    let variant_keys = match repository.list_variant_storage_keys(media_id).await {
                        Ok(keys) => keys,
                        Err(error) => {
                            warn!(media_id = %media_id, error = %error, "failed to list variants during deletion");
                            continue;
                        }
                    };
                    let mut variant_cleanup_failed = false;
                    for variant_key in variant_keys {
                        if let Err(error) = object_store.delete(&variant_key).await {
                            warn!(media_id = %media_id, storage_key = %variant_key, error = %error, "failed to remove variant during deletion");
                            variant_cleanup_failed = true;
                            break;
                        }
                    }
                    if variant_cleanup_failed {
                        continue;
                    }
                    if let Err(error) = object_store.delete(&storage_key).await {
                        warn!(media_id = %media_id, error = %error, "failed to remove object during deletion");
                        continue;
                    }
                    if let Err(error) = repository
                        .finalize_delete(media_id, OffsetDateTime::now_utc())
                        .await
                    {
                        warn!(media_id = %media_id, error = %error, "failed to finalize media deletion");
                    }
                }
            }
            Err(error) => warn!(error = %error, "pending deletion scan failed"),
        }
    }
}

async fn reconcile_stale_uploads(
    repository: &PostgresRepository,
    object_store: &RuntimeObjectStore,
    now: OffsetDateTime,
) {
    let leased_until = now + time::Duration::seconds(MEDIA_UPLOAD_LEASE_SECONDS);
    let uploads = match repository
        .claim_stale_uploading(now, leased_until, 100)
        .await
    {
        Ok(uploads) => uploads,
        Err(error) => {
            warn!(error = %error, "stale upload reconciliation scan failed");
            return;
        }
    };
    for leased in uploads {
        let media = &leased.media;
        let metadata = match run_reconciliation_storage_operation(
            repository,
            media.id(),
            &leased.lease_token,
            object_store.head(media.storage_key()),
        )
        .await
        {
            Ok(metadata) => metadata,
            Err(ReconciliationOperationError::Storage(ObjectStoreError::NotFound)) => {
                if run_reconciliation_storage_operation(
                    repository,
                    media.id(),
                    &leased.lease_token,
                    object_store.delete(&leased.temporary_key),
                )
                .await
                .is_ok()
                    && let Err(error) = repository
                        .abort_upload(media.id(), &leased.lease_token, OffsetDateTime::now_utc())
                        .await
                {
                    warn!(media_id = %media.id(), error = %error, "missing upload reconciliation rollback failed");
                }
                continue;
            }
            Err(error) => {
                warn!(media_id = %media.id(), error = %error, "stale upload storage inspection failed");
                continue;
            }
        };
        let checksum = match run_reconciliation_storage_operation(
            repository,
            media.id(),
            &leased.lease_token,
            object_store.checksum_sha256(media.storage_key()),
        )
        .await
        {
            Ok(checksum) => checksum,
            Err(error) => {
                warn!(media_id = %media.id(), error = %error, "stale upload checksum verification failed");
                continue;
            }
        };
        let matches = metadata.size == media.size()
            && metadata.content_type.as_deref() == Some(media.mime())
            && checksum.eq_ignore_ascii_case(media.sha256());
        if matches {
            if let Err(error) = run_reconciliation_storage_operation(
                repository,
                media.id(),
                &leased.lease_token,
                object_store.delete(&leased.temporary_key),
            )
            .await
            {
                warn!(media_id = %media.id(), error = %error, "promoted upload temporary cleanup failed");
                continue;
            }
            let committed_at = OffsetDateTime::now_utc();
            let event = OutboxEvent::media_uploaded(media, committed_at);
            if let Err(error) = repository
                .commit_upload(media.id(), &leased.lease_token, committed_at, event)
                .await
            {
                warn!(media_id = %media.id(), error = %error, "promoted upload reconciliation failed");
            }
            continue;
        }

        let final_deleted = run_reconciliation_storage_operation(
            repository,
            media.id(),
            &leased.lease_token,
            object_store.delete(media.storage_key()),
        )
        .await
        .is_ok();
        let temporary_deleted = run_reconciliation_storage_operation(
            repository,
            media.id(),
            &leased.lease_token,
            object_store.delete(&leased.temporary_key),
        )
        .await
        .is_ok();
        if final_deleted
            && temporary_deleted
            && let Err(error) = repository
                .abort_upload(media.id(), &leased.lease_token, OffsetDateTime::now_utc())
                .await
        {
            warn!(media_id = %media.id(), error = %error, "corrupt upload reconciliation rollback failed");
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum ReconciliationOperationError {
    #[error(transparent)]
    Storage(#[from] ObjectStoreError),
    #[error(transparent)]
    Repository(#[from] mediahub_app::RepositoryError),
    #[error("ordinary upload reconciliation lease was lost")]
    LeaseLost,
}

async fn run_reconciliation_storage_operation<T>(
    repository: &PostgresRepository,
    media_id: MediaId,
    lease_token: &str,
    operation: impl std::future::Future<Output = Result<T, ObjectStoreError>>,
) -> Result<T, ReconciliationOperationError> {
    tokio::pin!(operation);
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(
        MEDIA_UPLOAD_HEARTBEAT_SECONDS,
    ));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval.tick().await;
    loop {
        tokio::select! {
            result = &mut operation => {
                renew_reconciliation_lease(repository, media_id, lease_token).await?;
                return result.map_err(Into::into);
            }
            _ = interval.tick() => {
                renew_reconciliation_lease(repository, media_id, lease_token).await?;
            }
        }
    }
}

async fn renew_reconciliation_lease(
    repository: &PostgresRepository,
    media_id: MediaId,
    lease_token: &str,
) -> Result<(), ReconciliationOperationError> {
    let now = OffsetDateTime::now_utc();
    let leased_until = now + time::Duration::seconds(MEDIA_UPLOAD_LEASE_SECONDS);
    if repository
        .renew_upload_lease(media_id, lease_token, now, leased_until)
        .await?
    {
        Ok(())
    } else {
        Err(ReconciliationOperationError::LeaseLost)
    }
}

pub(super) async fn run_outbox_worker(
    repository: PostgresRepository,
    access_key_cipher: Arc<AccessKeyCipher>,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        interval.tick().await;
        let now = OffsetDateTime::now_utc();
        if let Err(error) = repository.finalize_unsubscribed_outbox_events(100).await {
            warn!(error = %error, "unsubscribed outbox cleanup failed");
        }
        let lease_until = now + time::Duration::seconds(WEBHOOK_DELIVERY_LEASE_SECONDS);
        let leased = match repository
            .claim_webhook_deliveries(now, lease_until, 1)
            .await
        {
            Ok(deliveries) => deliveries,
            Err(error) => {
                warn!(error = %error, "webhook delivery claim failed");
                continue;
            }
        };
        for leased_delivery in leased {
            let delivery = &leased_delivery.delivery;
            let event = &delivery.event;
            let endpoint = &delivery.endpoint;
            match deliver_webhook_delivery_with_timeout(&access_key_cipher, delivery).await {
                Ok(response_status) => {
                    match repository
                        .mark_webhook_delivery_delivered_with_status(
                            &event.id,
                            &endpoint.id,
                            &leased_delivery.lease_token,
                            OffsetDateTime::now_utc(),
                            Some(response_status),
                        )
                        .await
                    {
                        Ok(true) => {}
                        Ok(false) => {
                            warn!(event_id = %event.id, endpoint_id = %endpoint.id, "stale webhook delivery acknowledgement ignored")
                        }
                        Err(error) => {
                            warn!(event_id = %event.id, endpoint_id = %endpoint.id, error = %error, "webhook delivery acknowledgement failed")
                        }
                    }
                }
                Err(error) => {
                    let failed_at = OffsetDateTime::now_utc();
                    let retry_at = webhook_retry_at(delivery.attempt_count, failed_at);
                    match repository
                        .record_webhook_delivery_failure_with_status(
                            &event.id,
                            &endpoint.id,
                            &leased_delivery.lease_token,
                            failed_at,
                            retry_at,
                            WEBHOOK_MAX_ATTEMPTS,
                            error.response_status,
                            &error.summary,
                        )
                        .await
                    {
                        Ok(Some(WebhookDeliveryFailureDisposition::RetryScheduled {
                            attempt_count,
                            next_attempt_at,
                        })) => {
                            warn!(event_id = %event.id, endpoint_id = %endpoint.id, attempts = attempt_count, retry_at = %next_attempt_at, error = %error.summary, "webhook delivery will retry")
                        }
                        Ok(Some(WebhookDeliveryFailureDisposition::DeadLettered {
                            attempt_count,
                            dead_lettered_at,
                        })) => {
                            warn!(event_id = %event.id, endpoint_id = %endpoint.id, attempts = attempt_count, dead_lettered_at = %dead_lettered_at, error = %error.summary, "webhook delivery moved to dead letter")
                        }
                        Ok(None) => {
                            warn!(event_id = %event.id, endpoint_id = %endpoint.id, "stale webhook delivery failure ignored")
                        }
                        Err(repository_error) => {
                            warn!(event_id = %event.id, endpoint_id = %endpoint.id, error = %repository_error, "webhook delivery failure recording failed")
                        }
                    }
                }
            }
        }
    }
}

pub(super) async fn run_async_job_worker(repository: PostgresRepository) {
    let service = AsyncJobService::new(repository.clone(), SystemClock);
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(2));
    loop {
        interval.tick().await;
        let leased_jobs = match service.claim(1).await {
            Ok(jobs) => jobs,
            Err(error) => {
                warn!(error = %error, "async job claim failed");
                continue;
            }
        };
        for leased in leased_jobs {
            let job_id = leased.job.id();
            let application_id = leased.job.application_id();
            let attempt_count = leased.job.attempt_count();
            let action = leased.job.action().clone();
            let action_effective_at = leased.job.created_at();
            let mut item_results = Vec::with_capacity(leased.pending_media_ids.len());
            let mut result_error = None;
            let mut lease_lost = false;
            'items: for (ordinal, media_id) in
                leased.pending_media_ids.into_iter().enumerate()
            {
                match service.renew(job_id, &leased.lease_token).await {
                    Ok(true) => {}
                    Ok(false) => {
                        lease_lost = true;
                        warn!(job_id = %job_id, "async job lease was lost before item execution");
                        break;
                    }
                    Err(error) => {
                        lease_lost = true;
                        warn!(job_id = %job_id, error = %error, "async job lease renewal failed");
                        break;
                    }
                }
                let started_at = OffsetDateTime::now_utc();
                let execution = execute_batch_action(
                    &repository,
                    application_id,
                    media_id,
                    &action,
                    action_effective_at,
                );
                tokio::pin!(execution);
                let heartbeat_period = std::time::Duration::from_secs(10);
                let mut heartbeat = tokio::time::interval_at(
                    tokio::time::Instant::now() + heartbeat_period,
                    heartbeat_period,
                );
                let execution_result = loop {
                    tokio::select! {
                        result = &mut execution => break result,
                        _ = heartbeat.tick() => match service.renew(job_id, &leased.lease_token).await {
                            Ok(true) => {}
                            Ok(false) => {
                                lease_lost = true;
                                warn!(job_id = %job_id, "async job lease was lost during item execution");
                                break 'items;
                            }
                            Err(error) => {
                                lease_lost = true;
                                warn!(job_id = %job_id, error = %error, "async job lease heartbeat failed");
                                break 'items;
                            }
                        }
                    }
                };
                let completed_at = OffsetDateTime::now_utc();
                let result = match execution_result {
                    Ok(value) => AsyncJobItemResult::succeeded(
                        job_id,
                        application_id,
                        media_id,
                        u32::try_from(ordinal).unwrap_or(u32::MAX),
                        attempt_count,
                        Some(value),
                        started_at,
                        completed_at,
                    ),
                    Err(error) => AsyncJobItemResult::failed(
                        job_id,
                        application_id,
                        media_id,
                        u32::try_from(ordinal).unwrap_or(u32::MAX),
                        attempt_count,
                        error.code,
                        error.summary,
                        started_at,
                        completed_at,
                    ),
                };
                match result {
                    Ok(result) => item_results.push(result),
                    Err(error) => {
                        result_error = Some(error.to_string());
                        break;
                    }
                }
            }

            if lease_lost {
                continue;
            }

            if let Some(error_summary) = result_error {
                let retry_at = OffsetDateTime::now_utc() + time::Duration::seconds(5);
                if let Err(error) = service
                    .fail(&FailAsyncJobRequest {
                        job_id,
                        lease_token: leased.lease_token,
                        error_summary,
                        retry_at: Some(retry_at),
                    })
                    .await
                {
                    warn!(job_id = %job_id, error = %error, "async job failure could not be recorded");
                }
                continue;
            }

            if let Err(error) = service
                .complete(&CompleteAsyncJobRequest {
                    job_id,
                    lease_token: leased.lease_token,
                    item_results,
                })
                .await
            {
                warn!(job_id = %job_id, error = %error, "async job completion failed");
            }
        }
    }
}

#[derive(Debug)]
pub(super) struct BatchExecutionError {
    pub(super) code: &'static str,
    pub(super) summary: String,
}

pub(super) async fn execute_batch_action(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    media_id: MediaId,
    action: &AsyncJobAction,
    action_effective_at: OffsetDateTime,
) -> Result<serde_json::Value, BatchExecutionError> {
    let mut media = repository
        .find_media_by_id(media_id)
        .await
        .map_err(batch_repository_error)?
        .filter(|media| media.application_id() == application_id)
        .ok_or_else(|| BatchExecutionError {
            code: "not_found",
            summary: "media was not found".to_owned(),
        })?;
    let now = OffsetDateTime::now_utc();
    if matches!(action, AsyncJobAction::Delete)
        && matches!(
            media.state(),
            MediaState::DeletePending | MediaState::Deleted
        )
    {
        return Ok(batch_media_result(&media));
    }
    if media.state() != MediaState::Active {
        return Err(BatchExecutionError {
            code: "invalid_state",
            summary: "media cannot be changed in its current state".to_owned(),
        });
    }

    match action {
        AsyncJobAction::UpdateTtlSeconds { ttl_seconds } => {
            let expires_at = ttl_seconds
                .map(|seconds| {
                    i64::try_from(seconds)
                        .ok()
                        .and_then(|seconds| {
                            action_effective_at.checked_add(time::Duration::seconds(seconds))
                        })
                        .ok_or_else(|| BatchExecutionError {
                            code: "invalid_ttl",
                            summary: "ttl_seconds is too large".to_owned(),
                        })
                })
                .transpose()?;
            if media.expire_at() == expires_at {
                return Ok(batch_media_result(&media));
            }
            let expected_revision = media.revision();
            media
                .set_expire_at(expires_at, expected_revision, now)
                .map_err(batch_domain_error)?;
            let event = OutboxEvent::media_metadata_updated(&media, now);
            repository
                .update_media(media.clone(), expected_revision, event)
                .await
                .map_err(batch_repository_error)?;
        }
        AsyncJobAction::UpdateVisibility { visibility } => {
            if media.visibility_override() == Some(*visibility) {
                return Ok(batch_media_result(&media));
            }
            let expected_revision = media.revision();
            media
                .set_visibility_override(Some(*visibility), expected_revision, now)
                .map_err(batch_domain_error)?;
            let event = OutboxEvent::media_metadata_updated(&media, now);
            repository
                .update_media(media.clone(), expected_revision, event)
                .await
                .map_err(batch_repository_error)?;
        }
        AsyncJobAction::Delete => {
            let event = OutboxEvent::media_delete_scheduled(&media, now, "manual");
            media = repository
                .schedule_delete(media.id(), now, event)
                .await
                .map_err(batch_repository_error)?;
        }
    }
    Ok(batch_media_result(&media))
}

fn batch_media_result(media: &Media) -> serde_json::Value {
    serde_json::json!({
        "id": media.id().to_string(),
        "state": media.state(),
        "revision": media.revision(),
    })
}

fn batch_domain_error(error: DomainError) -> BatchExecutionError {
    BatchExecutionError {
        code: "invalid_request",
        summary: error.to_string(),
    }
}

fn batch_repository_error(error: mediahub_app::RepositoryError) -> BatchExecutionError {
    let (code, summary) = match error {
        mediahub_app::RepositoryError::NotFound => ("not_found", "resource was not found"),
        mediahub_app::RepositoryError::Conflict => ("conflict", "resource changed concurrently"),
        mediahub_app::RepositoryError::QuotaExceeded => ("quota_exceeded", "quota is exhausted"),
        mediahub_app::RepositoryError::Invariant(_)
        | mediahub_app::RepositoryError::Unavailable(_) => {
            ("unavailable", "metadata storage is unavailable")
        }
    };
    BatchExecutionError {
        code,
        summary: summary.to_owned(),
    }
}

fn webhook_retry_at(attempt_count: u32, now: OffsetDateTime) -> OffsetDateTime {
    let exponent = attempt_count.min(8);
    let seconds = 1_i64 << exponent;
    now + time::Duration::seconds(seconds.min(300))
}

async fn deliver_webhook_delivery(
    access_key_cipher: &AccessKeyCipher,
    delivery: &WebhookDelivery,
) -> Result<u16, WebhookAttemptError> {
    let event = &delivery.event;
    let endpoint = &delivery.endpoint;
    let (client, url) = webhook_client_for_url(&endpoint.url)
        .await
        .map_err(WebhookAttemptError::without_response)?;
    let body = serde_json::to_vec(&serde_json::json!({
        "id": event.id,
        "type": event.event_type,
        "application_id": event.application_id.to_string(),
        "aggregate_id": event.aggregate_id,
        "created_at": event.created_at,
        "data": event.payload,
    }))
    .map_err(|_| WebhookAttemptError::without_response("webhook payload encoding failed"))?;
    let timestamp = OffsetDateTime::now_utc().unix_timestamp().to_string();
    let secret = access_key_cipher
        .decrypt(&endpoint.secret_ciphertext, endpoint.secret_key_version)
        .map_err(|_| WebhookAttemptError::without_response("webhook secret decryption failed"))?;
    let mut mac =
        HmacSha256::new_from_slice(&secret).expect("HMAC accepts webhook secrets of every length");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(&body);
    let signature = hex::encode(mac.finalize().into_bytes());
    let response = client
        .post(url)
        .header("X-MediaHub-Event-Id", &event.id)
        .header("X-MediaHub-Event-Type", &event.event_type)
        .header("X-MediaHub-Timestamp", &timestamp)
        .header("X-MediaHub-Signature", format!("v1={signature}"))
        .header(CONTENT_TYPE, "application/json")
        .body(body)
        .send()
        .await
        .map_err(|_| WebhookAttemptError::without_response("webhook HTTP request failed"))?;
    let response_status = response.status().as_u16();
    if !response.status().is_success() {
        return Err(WebhookAttemptError {
            summary: format!("webhook endpoint returned HTTP {}", response.status()),
            response_status: Some(response_status),
        });
    }
    Ok(response_status)
}

async fn deliver_webhook_delivery_with_timeout(
    access_key_cipher: &AccessKeyCipher,
    delivery: &WebhookDelivery,
) -> Result<u16, WebhookAttemptError> {
    webhook_attempt_with_timeout(
        WEBHOOK_ATTEMPT_TIMEOUT,
        deliver_webhook_delivery(access_key_cipher, delivery),
    )
    .await
}

async fn webhook_attempt_with_timeout<T>(
    timeout: std::time::Duration,
    attempt: impl std::future::Future<Output = Result<T, WebhookAttemptError>>,
) -> Result<T, WebhookAttemptError> {
    tokio::time::timeout(timeout, attempt)
        .await
        .map_err(|_| WebhookAttemptError::without_response("webhook delivery attempt timed out"))?
}

struct WebhookAttemptError {
    summary: String,
    response_status: Option<u16>,
}

impl WebhookAttemptError {
    fn without_response(summary: impl Into<String>) -> Self {
        Self {
            summary: summary.into(),
            response_status: None,
        }
    }
}

async fn webhook_client_for_url(value: &str) -> Result<(reqwest::Client, Url), String> {
    let url = Url::parse(value).map_err(|_| "webhook endpoint URL is invalid".to_owned())?;
    validate_webhook_url_parsed(&url)
        .map_err(|()| "webhook endpoint URL is forbidden".to_owned())?;
    let host = url
        .host_str()
        .ok_or_else(|| "webhook endpoint URL has no host".to_owned())?;
    let port = url.port_or_known_default().unwrap_or(443);
    let addresses = tokio::time::timeout(
        WEBHOOK_DNS_TIMEOUT,
        tokio::net::lookup_host((host, port)),
    )
        .await
        .map_err(|_| "webhook endpoint DNS lookup timed out".to_owned())?
        .map_err(|_| "webhook endpoint DNS lookup failed".to_owned())?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err("webhook endpoint DNS lookup returned no addresses".to_owned());
    }
    for address in &addresses {
        if !is_public_webhook_ip(address.ip()) {
            return Err("webhook endpoint URL resolves to a forbidden address".to_owned());
        }
    }
    let mut builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none());
    if matches!(url.host(), Some(Host::Domain(_))) {
        builder = builder.resolve(host, addresses[0]);
    }
    let client = builder
        .build()
        .map_err(|_| "webhook HTTP client initialization failed".to_owned())?;
    Ok((client, url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn webhook_attempt_timeout_is_strictly_shorter_than_its_lease() {
        let lease = std::time::Duration::from_secs(
            WEBHOOK_DELIVERY_LEASE_SECONDS
                .try_into()
                .expect("positive webhook lease"),
        );
        assert!(WEBHOOK_DNS_TIMEOUT < WEBHOOK_ATTEMPT_TIMEOUT);
        assert!(WEBHOOK_ATTEMPT_TIMEOUT < lease);

        let result = webhook_attempt_with_timeout(
            std::time::Duration::ZERO,
            std::future::pending::<Result<u16, WebhookAttemptError>>(),
        )
        .await;
        let Err(error) = result else {
            panic!("a stalled webhook attempt must time out");
        };
        assert_eq!(error.summary, "webhook delivery attempt timed out");
        assert_eq!(error.response_status, None);
    }
}

