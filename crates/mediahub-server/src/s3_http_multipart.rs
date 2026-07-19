// S3 multipart upload operations.

async fn s3_create_multipart_upload(
    state: &AppState,
    auth: &ApplicationAuth,
    bucket_name: &str,
    object_key: &str,
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &str,
) -> Result<Response, S3ApiError> {
    auth.authorize("media:upload")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    let _ = s3_object_names(object_key, uri.path(), request_id)?;
    let visibility_override = s3_canned_acl(headers, uri.path(), request_id)?;
    if visibility_override.is_some() {
        auth.authorize("media:update")
            .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    }
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), request_id))?;
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("application/octet-stream");
    let content_type = normalized_mime(content_type)
        .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    let now = OffsetDateTime::now_utc();
    let upload_id = format!("mh_mpu_{}", uuid::Uuid::now_v7().simple());
    let upload = state
        .repository
        .create_multipart_upload(NewS3MultipartUpload {
            upload_id: upload_id.clone(),
            application_id: auth.application.id,
            bucket_id: bucket.id(),
            object_key: object_key.to_owned(),
            content_type,
            visibility_override,
            expires_at: now + time::Duration::seconds(S3_MULTIPART_UPLOAD_SECONDS),
            created_at: now,
        })
        .await
        .map_err(ApiError::from_repository)
        .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    record_audit(
        state,
        auth,
        request_id,
        "s3.multipart_created",
        "multipart_upload",
        upload.upload_id.clone(),
        serde_json::json!({
            "protocol": "s3",
            "bucket": bucket_name,
            "object_key": object_key,
        }),
    )
    .await;
    let body = initiate_multipart_upload_result_xml(bucket_name, object_key, &upload_id)
        .map_err(|error| S3ApiError::from_xml(error, uri.path(), request_id))?;
    Ok(s3_xml_response(StatusCode::OK, body, request_id))
}

struct S3ObjectOperation<'a> {
    state: &'a AppState,
    auth: &'a ApplicationAuth,
    bucket_name: &'a str,
    object_key: &'a str,
    uri: &'a Uri,
    request_id: &'a str,
}

async fn s3_upload_part(
    operation: S3ObjectOperation<'_>,
    upload_id: &str,
    part_number: u16,
    content: Bytes,
) -> Result<Response, S3ApiError> {
    let S3ObjectOperation {
        state,
        auth,
        bucket_name,
        object_key,
        uri,
        request_id,
    } = operation;
    auth.authorize("media:upload")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    let upload = find_s3_multipart_upload(
        state,
        auth,
        bucket_name,
        object_key,
        upload_id,
        uri.path(),
        request_id,
    )
    .await?;
    if upload.state != S3MultipartUploadState::Pending
        || upload.expires_at <= OffsetDateTime::now_utc()
    {
        return Err(S3ApiError::no_such_upload(uri.path(), request_id));
    }
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Multipart Bucket policy lookup failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), request_id))?;
    let maximum_upload_size = bucket
        .policy()
        .max_object_size()
        .unwrap_or(MAX_UPLOAD_OBJECT_BYTES)
        .min(MAX_UPLOAD_OBJECT_BYTES);
    let sha256 = format!("{:x}", Sha256::digest(&content));
    let etag = format!("\"{sha256}\"");
    let storage_key = new_multipart_part_storage_key(upload_id, part_number);
    state
        .object_store
        .put_temporary(&storage_key, &content, "application/octet-stream")
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart part storage failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?;
    let result = state
        .repository
        .put_multipart_part(
            upload_id,
            NewS3MultipartPart {
                part_number,
                size: content.len() as u64,
                sha256,
                etag: etag.clone(),
                storage_key: storage_key.clone(),
            },
            maximum_upload_size,
            OffsetDateTime::now_utc(),
        )
        .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = state.object_store.delete(&storage_key).await;
            return Err(S3ApiError::from_api(
                ApiError::from_repository(error),
                uri.path(),
                request_id,
            ));
        }
    };
    match result {
        S3MultipartPartPut::Stored {
            replaced_storage_key,
            ..
        } => {
            if let Some(replaced) = replaced_storage_key
                && let Err(error) = state.object_store.delete(&replaced).await
            {
                warn!(error = %error, storage_key = %replaced, "replaced S3 multipart part cleanup failed");
            }
            let mut response = s3_empty_response(StatusCode::OK, request_id);
            response.headers_mut().insert(
                ETAG,
                HeaderValue::from_str(&etag)
                    .unwrap_or_else(|_| HeaderValue::from_static("\"invalid-etag\"")),
            );
            Ok(response)
        }
        S3MultipartPartPut::NotPending(_) => {
            let _ = state.object_store.delete(&storage_key).await;
            Err(S3ApiError::no_such_upload(uri.path(), request_id))
        }
        S3MultipartPartPut::Expired { upload, .. } => {
            if cleanup_multipart_storage(&state.object_store, &upload.upload_id).await {
                let _ = state
                    .repository
                    .clear_multipart_parts(&upload.upload_id)
                    .await;
            }
            Err(S3ApiError::no_such_upload(uri.path(), request_id))
        }
    }
}

async fn find_s3_multipart_upload(
    state: &AppState,
    auth: &ApplicationAuth,
    bucket_name: &str,
    object_key: &str,
    upload_id: &str,
    resource: &str,
    request_id: &str,
) -> Result<S3MultipartUpload, S3ApiError> {
    let upload = state
        .repository
        .find_multipart_upload(upload_id)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart upload lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .ok_or_else(|| S3ApiError::no_such_upload(resource, request_id))?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Bucket lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(resource, request_id))?;
    if upload.application_id != auth.application.id
        || upload.bucket_id != bucket.id()
        || upload.object_key != object_key
    {
        return Err(S3ApiError::no_such_upload(resource, request_id));
    }
    Ok(upload)
}

async fn s3_list_parts(
    state: &AppState,
    auth: &ApplicationAuth,
    bucket_name: &str,
    object_key: &str,
    upload_id: &str,
    uri: &Uri,
    request_id: &str,
) -> Result<Response, S3ApiError> {
    auth.authorize("media:upload")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    let upload = find_s3_multipart_upload(
        state,
        auth,
        bucket_name,
        object_key,
        upload_id,
        uri.path(),
        request_id,
    )
    .await?;
    if matches!(
        upload.state,
        S3MultipartUploadState::Completed | S3MultipartUploadState::Aborted
    ) || upload.expires_at <= OffsetDateTime::now_utc()
    {
        return Err(S3ApiError::no_such_upload(uri.path(), request_id));
    }
    let marker = s3_query_value(uri, "part-number-marker", request_id)?
        .as_deref()
        .unwrap_or("0")
        .parse::<u16>()
        .map_err(|_| {
            S3ApiError::invalid_argument("part-number-marker is invalid.", uri.path(), request_id)
        })?;
    let max_parts = s3_query_value(uri, "max-parts", request_id)?
        .as_deref()
        .unwrap_or("1000")
        .parse::<u16>()
        .ok()
        .filter(|value| (1..=1_000).contains(value))
        .ok_or_else(|| {
            S3ApiError::invalid_argument(
                "max-parts must be between 1 and 1000.",
                uri.path(),
                request_id,
            )
        })?;
    let mut parts = state
        .repository
        .list_multipart_parts(upload_id)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart parts listing failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?
        .into_iter()
        .filter(|part| part.part_number > marker)
        .take(usize::from(max_parts) + 1)
        .collect::<Vec<_>>();
    let is_truncated = parts.len() > usize::from(max_parts);
    parts.truncate(usize::from(max_parts));
    let next_marker = if is_truncated {
        parts.last().map_or(marker, |part| part.part_number)
    } else {
        marker
    };
    let body = list_parts_result_xml(&ListPartsResult {
        bucket: bucket_name.to_owned(),
        key: object_key.to_owned(),
        upload_id: upload_id.to_owned(),
        part_number_marker: marker,
        next_part_number_marker: next_marker,
        max_parts,
        is_truncated,
        parts: parts
            .into_iter()
            .map(|part| {
                let last_modified = part
                    .updated_at
                    .format(&time::format_description::well_known::Rfc3339)
                    .map_err(|_| S3ApiError::service_unavailable(uri.path(), request_id))?;
                Ok(ListedPart {
                    part_number: part.part_number,
                    last_modified,
                    etag: part.etag,
                    size: part.size,
                })
            })
            .collect::<Result<Vec<_>, S3ApiError>>()?,
    })
    .map_err(|error| S3ApiError::from_xml(error, uri.path(), request_id))?;
    Ok(s3_xml_response(StatusCode::OK, body, request_id))
}

async fn s3_abort_multipart_upload(
    state: &AppState,
    auth: &ApplicationAuth,
    bucket_name: &str,
    object_key: &str,
    upload_id: &str,
    uri: &Uri,
    request_id: &str,
) -> Result<Response, S3ApiError> {
    auth.authorize("media:upload")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    find_s3_multipart_upload(
        state,
        auth,
        bucket_name,
        object_key,
        upload_id,
        uri.path(),
        request_id,
    )
    .await?;
    let result = state
        .repository
        .abort_multipart_upload(upload_id, OffsetDateTime::now_utc())
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart abort failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?;
    match result {
        S3MultipartAbort::Aborted { .. } | S3MultipartAbort::AlreadyAborted { .. } => {}
        S3MultipartAbort::Completing(_) => {
            return Err(S3ApiError::operation_aborted(
                "The multipart upload is currently completing.",
                uri.path(),
                request_id,
            ));
        }
        S3MultipartAbort::Completed(_) => {
            return Err(S3ApiError::no_such_upload(uri.path(), request_id));
        }
    }
    if !cleanup_multipart_storage(&state.object_store, upload_id).await {
        return Err(S3ApiError::service_unavailable(uri.path(), request_id));
    }
    state
        .repository
        .clear_multipart_parts(upload_id)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart part metadata cleanup failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?;
    record_audit(
        state,
        auth,
        request_id,
        "s3.multipart_aborted",
        "multipart_upload",
        upload_id.to_owned(),
        serde_json::json!({
            "protocol": "s3",
            "bucket": bucket_name,
            "object_key": object_key,
        }),
    )
    .await;
    Ok(s3_empty_response(StatusCode::NO_CONTENT, request_id))
}

async fn s3_complete_multipart_upload(
    operation: S3ObjectOperation<'_>,
    upload_id: &str,
    content: &[u8],
) -> Result<Response, S3ApiError> {
    let S3ObjectOperation {
        state,
        auth,
        bucket_name,
        object_key,
        uri,
        request_id,
    } = operation;
    auth.authorize("media:upload")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    let (display_name, extension) = s3_object_names(object_key, uri.path(), request_id)?;
    let upload = find_s3_multipart_upload(
        state,
        auth,
        bucket_name,
        object_key,
        upload_id,
        uri.path(),
        request_id,
    )
    .await?;
    if upload.state == S3MultipartUploadState::Completed {
        let etag = upload
            .final_etag
            .as_deref()
            .ok_or_else(|| S3ApiError::service_unavailable(uri.path(), request_id))?;
        cleanup_completed_multipart(state, upload_id).await;
        return s3_complete_multipart_response(
            uri.path(),
            bucket_name,
            object_key,
            etag,
            request_id,
        );
    }
    let request = parse_complete_multipart_upload_xml(content)
        .map_err(|error| S3ApiError::from_xml(error, uri.path(), request_id))?;
    let manifest = request
        .parts
        .into_iter()
        .map(|part| CompletedS3MultipartPart {
            part_number: part.part_number,
            etag: part.etag,
        })
        .collect::<Vec<_>>();
    let completion_token = uuid::Uuid::now_v7().simple().to_string();
    let now = OffsetDateTime::now_utc();
    let claim = state
        .repository
        .claim_multipart_completion(
            upload_id,
            &manifest,
            &completion_token,
            now + time::Duration::seconds(S3_MULTIPART_COMPLETION_LEASE_SECONDS),
            now,
        )
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart completion claim failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?;
    let manifest = match claim {
        S3MultipartCompletionClaim::Claimed(manifest) => manifest,
        S3MultipartCompletionClaim::AlreadyCompleted(upload) => {
            let etag = upload
                .final_etag
                .as_deref()
                .ok_or_else(|| S3ApiError::service_unavailable(uri.path(), request_id))?;
            cleanup_completed_multipart(state, upload_id).await;
            return s3_complete_multipart_response(
                uri.path(),
                bucket_name,
                object_key,
                etag,
                request_id,
            );
        }
        S3MultipartCompletionClaim::InProgress(_) => {
            return Err(S3ApiError::operation_aborted(
                "A conflicting multipart completion is in progress.",
                uri.path(),
                request_id,
            ));
        }
        S3MultipartCompletionClaim::Aborted(_) => {
            return Err(S3ApiError::no_such_upload(uri.path(), request_id));
        }
        S3MultipartCompletionClaim::Expired {
            upload,
            storage_keys: _,
        } => {
            if cleanup_multipart_storage(&state.object_store, &upload.upload_id).await {
                let _ = state
                    .repository
                    .clear_multipart_parts(&upload.upload_id)
                    .await;
            }
            return Err(S3ApiError::no_such_upload(uri.path(), request_id));
        }
        S3MultipartCompletionClaim::InvalidManifest(error) => {
            return Err(S3ApiError::from_multipart_manifest(
                error,
                uri.path(),
                request_id,
            ));
        }
    };
    if manifest
        .parts
        .iter()
        .take(manifest.parts.len().saturating_sub(1))
        .any(|part| part.size < MIN_S3_MULTIPART_PART_BYTES)
    {
        let _ = state
            .repository
            .release_multipart_completion(upload_id, &completion_token, OffsetDateTime::now_utc())
            .await;
        return Err(S3ApiError::entity_too_small(uri.path(), request_id));
    }
    if let Err(error) = validate_upload_expected_size(manifest.total_size) {
        let _ = state
            .repository
            .release_multipart_completion(upload_id, &completion_token, OffsetDateTime::now_utc())
            .await;
        return Err(S3ApiError::from_api(error, uri.path(), request_id));
    }
    let source_keys = manifest
        .parts
        .iter()
        .map(|part| part.storage_key.clone())
        .collect::<Vec<_>>();
    let assembled_key = new_multipart_completion_storage_key(upload_id);
    let composed = match state
        .object_store
        .compose_temporary(&assembled_key, &source_keys, &manifest.upload.content_type)
        .await
    {
        Ok(composed) => composed,
        Err(error) => {
            warn!(error = %error, "S3 multipart composition failed");
            let _ = state
                .repository
                .release_multipart_completion(
                    upload_id,
                    &completion_token,
                    OffsetDateTime::now_utc(),
                )
                .await;
            let _ = state.object_store.delete(&assembled_key).await;
            return Err(S3ApiError::service_unavailable(uri.path(), request_id));
        }
    };
    if composed.size != manifest.total_size {
        let _ = state.object_store.delete(&assembled_key).await;
        let _ = state
            .repository
            .release_multipart_completion(upload_id, &completion_token, OffsetDateTime::now_utc())
            .await;
        return Err(S3ApiError::service_unavailable(uri.path(), request_id));
    }
    let service = UploadMediaService::new(
        state.object_store.clone(),
        state.repository.clone(),
        state.repository.clone(),
        SystemClock,
    );
    let staged = StagedUploadMediaRequest {
        application_id: auth.application.id,
        bucket_id: manifest.upload.bucket_id,
        object_key: object_key.to_owned(),
        original_name: Some(display_name.clone()),
        display_name,
        extension,
        mime: manifest.upload.content_type.clone(),
        temporary_key: assembled_key.clone(),
        size: composed.size,
        sha256: composed.sha256.clone(),
        visibility_override: manifest.upload.visibility_override,
        expire_at: None,
        metadata: ClientMetadata::default(),
    };
    let existing = state
        .repository
        .find_by_object_key(auth.application.id, manifest.upload.bucket_id, object_key)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart pre-commit recovery lookup failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?;
    let recovered = match existing {
        Some(media)
            if media.state().is_readable()
                && media.size() == composed.size
                && media.sha256() == composed.sha256 =>
        {
            let _ = state.object_store.delete(&assembled_key).await;
            Some(media)
        }
        Some(media) if media.state() == MediaState::Uploading => {
            if let Err(error) = state.object_store.delete(media.storage_key()).await {
                warn!(error = %error, media_id = %media.id(), "stale S3 multipart final object cleanup failed");
                let _ = state.object_store.delete(&assembled_key).await;
                let _ = state
                    .repository
                    .release_multipart_completion(
                        upload_id,
                        &completion_token,
                        OffsetDateTime::now_utc(),
                    )
                    .await;
                return Err(S3ApiError::service_unavailable(uri.path(), request_id));
            }
            if let Err(error) = state
                .repository
                .abort_uploading_for_multipart(upload_id, &completion_token, media.id())
                .await
            {
                warn!(error = %error, media_id = %media.id(), "stale S3 multipart Media cleanup failed");
                let _ = state.object_store.delete(&assembled_key).await;
                let _ = state
                    .repository
                    .release_multipart_completion(
                        upload_id,
                        &completion_token,
                        OffsetDateTime::now_utc(),
                    )
                    .await;
                return Err(S3ApiError::service_unavailable(uri.path(), request_id));
            }
            None
        }
        Some(_) => {
            let _ = state.object_store.delete(&assembled_key).await;
            let _ = state
                .repository
                .release_multipart_completion(
                    upload_id,
                    &completion_token,
                    OffsetDateTime::now_utc(),
                )
                .await;
            return Err(S3ApiError::operation_aborted(
                "The object key is already used by different content.",
                uri.path(),
                request_id,
            ));
        }
        None => None,
    };
    let (media, uploaded_now) = if let Some(media) = recovered {
        (media, false)
    } else {
        match service
            .upload_multipart_staged(upload_id, &completion_token, &staged)
            .await
        {
            Ok(receipt) => (receipt.media, true),
            Err(ApplicationError::ObjectAlreadyExists) => {
                let _ = state.object_store.delete(&assembled_key).await;
                let existing = state
                    .repository
                    .find_by_object_key(auth.application.id, manifest.upload.bucket_id, object_key)
                    .await
                    .map_err(|error| {
                        warn!(error = %error, "S3 multipart recovery lookup failed");
                        S3ApiError::service_unavailable(uri.path(), request_id)
                    })?
                    .filter(|media| {
                        media.state().is_readable()
                            && media.size() == composed.size
                            && media.sha256() == composed.sha256
                    });
                let Some(existing) = existing else {
                    let _ = state
                        .repository
                        .release_multipart_completion(
                            upload_id,
                            &completion_token,
                            OffsetDateTime::now_utc(),
                        )
                        .await;
                    return Err(S3ApiError::operation_aborted(
                        "The object key is already used by different content.",
                        uri.path(),
                        request_id,
                    ));
                };
                (existing, false)
            }
            Err(error) => {
                let _ = state.object_store.delete(&assembled_key).await;
                let _ = state
                    .repository
                    .release_multipart_completion(
                        upload_id,
                        &completion_token,
                        OffsetDateTime::now_utc(),
                    )
                    .await;
                return Err(S3ApiError::from_api(
                    ApiError::from_application(error),
                    uri.path(),
                    request_id,
                ));
            }
        }
    };
    let final_etag = media.etag().to_owned();
    let finish = state
        .repository
        .finish_multipart_completion(
            upload_id,
            &completion_token,
            media.id(),
            &final_etag,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart completion finalize failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?;
    match finish {
        S3MultipartCompletionFinish::Completed(_) => {
            state
                .http_metrics
                .uploaded_bytes
                .fetch_add(media.size(), Ordering::Relaxed);
            record_audit(
                state,
                auth,
                request_id,
                "media.uploaded",
                "media",
                media.id().to_string(),
                serde_json::json!({
                    "bucket": bucket_name,
                    "object_key": object_key,
                    "size": media.size(),
                    "protocol": "s3",
                    "multipart": true,
                    "recovered": !uploaded_now,
                }),
            )
            .await;
        }
        S3MultipartCompletionFinish::AlreadyCompleted(_) => {}
        S3MultipartCompletionFinish::OwnershipLost(_)
        | S3MultipartCompletionFinish::NotCompleting(_) => {
            return Err(S3ApiError::operation_aborted(
                "The multipart completion lease was lost.",
                uri.path(),
                request_id,
            ));
        }
    }
    if cleanup_multipart_storage(&state.object_store, upload_id).await {
        let _ = state.repository.clear_multipart_parts(upload_id).await;
    }
    s3_complete_multipart_response(uri.path(), bucket_name, object_key, &final_etag, request_id)
}

async fn cleanup_completed_multipart(state: &AppState, upload_id: &str) {
    if cleanup_multipart_storage(&state.object_store, upload_id).await {
        let _ = state.repository.clear_multipart_parts(upload_id).await;
    }
}

