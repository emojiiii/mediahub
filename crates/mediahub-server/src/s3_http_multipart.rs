// S3 multipart upload operations.

async fn s3_create_multipart_upload(
    operation: S3ObjectOperation<'_>,
    headers: &HeaderMap,
    source_ip: Option<std::net::IpAddr>,
) -> Result<Response, S3ApiError> {
    let S3ObjectOperation {
        state,
        auth,
        bucket_name,
        object_key,
        uri,
        request_id,
    } = operation;
    let resource = uri.path();
    validate_s3_object_key(object_key, resource, request_id)?;
    let canned_acl = s3_canned_acl(headers, resource, request_id)?;
    let content_type = s3_object_content_type(headers, resource, request_id)?;
    let user_metadata = s3_user_metadata(headers, resource, request_id)?;
    let requested_object_tags = parse_s3_tagging_header(headers, resource, request_id)?;
    let authorization = authorize_s3_signed_data_request(
        state,
        auth,
        S3DataAuthorizationInput {
            action: S3PolicyAction::PutObject,
            bucket_name,
            object_key: Some(object_key),
            version_id: None,
            prefix: None,
            delimiter: None,
            max_keys: None,
            secure_transport: s3_data_secure_transport(uri),
            source_ip,
        },
        resource,
        request_id,
    )
    .await?;
    if requested_object_tags.is_some() {
        let tagging_authorization = authorize_s3_signed_data_request(
            state,
            auth,
            S3DataAuthorizationInput {
                action: S3PolicyAction::PutObjectTagging,
                bucket_name,
                object_key: Some(object_key),
                version_id: None,
                prefix: None,
                delimiter: None,
                max_keys: None,
                secure_transport: s3_data_secure_transport(uri),
                source_ip,
            },
            resource,
            request_id,
        )
        .await?;
        ensure_same_s3_multipart_bucket(
            &authorization,
            &tagging_authorization,
            resource,
            request_id,
        )?;
    }
    if canned_acl.is_some() {
        let acl_authorization = authorize_s3_signed_data_request(
            state,
            auth,
            S3DataAuthorizationInput {
                action: S3PolicyAction::PutObjectAcl,
                bucket_name,
                object_key: Some(object_key),
                version_id: None,
                prefix: None,
                delimiter: None,
                max_keys: None,
                secure_transport: s3_data_secure_transport(uri),
                source_ip,
            },
            resource,
            request_id,
        )
        .await?;
        ensure_same_s3_multipart_bucket(&authorization, &acl_authorization, resource, request_id)?;
    }
    let object_tags = requested_object_tags.unwrap_or_default();
    let now = OffsetDateTime::now_utc();
    let upload_id = format!("mh_mpu_{}", uuid::Uuid::now_v7().simple());
    let upload = state
        .repository
        .create_multipart_upload(NewS3MultipartUpload {
            upload_id: upload_id.clone(),
            application_id: authorization.bucket.application_id,
            bucket_id: authorization.bucket.bucket_id,
            object_key: object_key.to_owned(),
            content_type,
            user_metadata,
            object_tags,
            storage_backend: state.object_store.backend_name().to_owned(),
            expires_at: now + time::Duration::seconds(S3_MULTIPART_UPLOAD_SECONDS),
            created_at: now,
        })
        .await
        .map_err(ApiError::from_repository)
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    super::record_s3_resource_audit(
        state,
        auth,
        authorization.bucket.application_id,
        request_id,
        "s3.multipart_created",
        ("multipart_upload", upload.upload_id.clone()),
        serde_json::json!({
            "protocol": "s3",
            "bucket": bucket_name,
            "object_key": object_key,
        }),
    )
    .await;
    let body = initiate_multipart_upload_result_xml(bucket_name, object_key, &upload_id)
        .map_err(|error| S3ApiError::from_xml(error, resource, request_id))?;
    Ok(s3_xml_response(StatusCode::OK, body, request_id))
}

fn ensure_same_s3_multipart_bucket(
    expected: &S3AuthorizedDataRequest,
    actual: &S3AuthorizedDataRequest,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    if expected.bucket.application_id == actual.bucket.application_id
        && expected.bucket.bucket_id == actual.bucket.bucket_id
    {
        Ok(())
    } else {
        Err(S3ApiError::service_unavailable(resource, request_id))
    }
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
    signature: ParsedSigV4,
    headers: &HeaderMap,
    source_ip: Option<std::net::IpAddr>,
    content: Body,
) -> Result<Response, S3ApiError> {
    let S3ObjectOperation {
        state,
        auth,
        bucket_name,
        object_key,
        uri,
        request_id,
    } = operation;
    let resource = uri.path();
    validate_s3_multipart_upload_id(upload_id, resource, request_id)?;
    let expected_size = s3_content_length(headers, resource, request_id)?;
    if expected_size > MAX_UPLOAD_OBJECT_BYTES {
        return Err(S3ApiError::entity_too_large(resource, request_id));
    }
    let expected_md5 = parse_content_md5(headers, resource, request_id)?;
    let storage_key = new_multipart_part_storage_key(upload_id, part_number);
    let streamed = match state
        .object_store
        .put_temporary_stream(
            &storage_key,
            content.into_data_stream(),
            expected_size,
            "application/octet-stream",
        )
        .await
    {
        Ok(streamed) => streamed,
        Err(error) => {
            discard_unbound_multipart_temporary(state, &storage_key).await;
            return Err(s3_streaming_upload_error(
                error,
                expected_size,
                resource,
                request_id,
            ));
        }
    };
    if let Err(error) = signature.verify_payload_sha256(&streamed.sha256) {
        discard_unbound_multipart_temporary(state, &storage_key).await;
        return Err(S3ApiError::from_sigv4(error, resource, request_id));
    }
    if expected_md5
        .as_deref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&streamed.md5))
    {
        discard_unbound_multipart_temporary(state, &storage_key).await;
        return Err(S3ApiError::bad_digest(resource, request_id));
    }

    let authorization =
        match authorize_s3_multipart_upload_request(S3MultipartAuthorizationRequest {
            state,
            auth,
            action: S3PolicyAction::PutObject,
            bucket_name,
            object_key,
            uri,
            source_ip,
            request_id,
        })
        .await
        {
            Ok(authorization) => authorization,
            Err(error) => {
                discard_unbound_multipart_temporary(state, &storage_key).await;
                return Err(error);
            }
        };
    let upload = match find_s3_multipart_upload(
        state,
        &authorization,
        bucket_name,
        object_key,
        upload_id,
        resource,
        request_id,
    )
    .await
    {
        Ok(upload) => upload,
        Err(error) => {
            discard_unbound_multipart_temporary(state, &storage_key).await;
            return Err(error);
        }
    };
    if upload.state != S3MultipartUploadState::Pending
        || upload.expires_at <= OffsetDateTime::now_utc()
    {
        persist_multipart_temporary_gc(state, &upload, storage_key, OffsetDateTime::now_utc())
            .await
            .map_err(|error| {
                warn!(error = %error, "failed to persist terminal multipart part cleanup");
                S3ApiError::service_unavailable(resource, request_id)
            })?;
        return Err(S3ApiError::no_such_upload(resource, request_id));
    }
    let bucket = state
        .repository
        .find_bucket_by_name(authorization.application_id(), bucket_name)
        .await;
    let bucket = match bucket {
        Ok(Some(bucket)) if bucket.id() == upload.bucket_id => bucket,
        Ok(_) => {
            persist_multipart_temporary_gc(state, &upload, storage_key, OffsetDateTime::now_utc())
                .await
                .map_err(|error| {
                    warn!(error = %error, "failed to persist missing-bucket multipart cleanup");
                    S3ApiError::service_unavailable(resource, request_id)
                })?;
            return Err(S3ApiError::no_such_upload(resource, request_id));
        }
        Err(error) => {
            persist_multipart_temporary_gc(state, &upload, storage_key, OffsetDateTime::now_utc())
                .await
                .map_err(|gc_error| {
                    warn!(error = %gc_error, "failed to persist bucket-error multipart cleanup");
                    S3ApiError::service_unavailable(resource, request_id)
                })?;
            warn!(error = %error, "S3 Multipart bucket policy lookup failed");
            return Err(S3ApiError::service_unavailable(resource, request_id));
        }
    };
    let maximum_upload_size = bucket
        .policy()
        .max_object_size()
        .unwrap_or(MAX_UPLOAD_OBJECT_BYTES)
        .min(MAX_UPLOAD_OBJECT_BYTES);
    if expected_size > maximum_upload_size {
        persist_multipart_temporary_gc(state, &upload, storage_key, OffsetDateTime::now_utc())
            .await
            .map_err(|error| {
                warn!(error = %error, "failed to persist oversized multipart part cleanup");
                S3ApiError::service_unavailable(resource, request_id)
            })?;
        return Err(S3ApiError::entity_too_large(resource, request_id));
    }

    let etag = streamed.md5.clone();
    let result = state
        .repository
        .put_multipart_part(
            upload_id,
            NewS3MultipartPart {
                part_number,
                size: streamed.size,
                sha256: streamed.sha256,
                md5: etag.clone(),
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
            persist_multipart_temporary_gc(state, &upload, storage_key, OffsetDateTime::now_utc())
                .await
                .map_err(|gc_error| {
                    warn!(error = %gc_error, "failed to persist rejected part cleanup");
                    S3ApiError::service_unavailable(resource, request_id)
                })?;
            return Err(S3ApiError::from_api(
                ApiError::from_repository(error),
                resource,
                request_id,
            ));
        }
    };
    match result {
        S3MultipartPartPut::Stored(_) => {
            let mut response = s3_empty_response(StatusCode::OK, request_id);
            response.headers_mut().insert(
                ETAG,
                HeaderValue::from_str(&format!("\"{etag}\""))
                    .map_err(|_| S3ApiError::service_unavailable(resource, request_id))?,
            );
            Ok(response)
        }
        S3MultipartPartPut::NotPending(_) | S3MultipartPartPut::Expired(_) => {
            persist_multipart_temporary_gc(state, &upload, storage_key, OffsetDateTime::now_utc())
                .await
                .map_err(|error| {
                    warn!(error = %error, "failed to persist terminal multipart part cleanup");
                    S3ApiError::service_unavailable(resource, request_id)
                })?;
            Err(S3ApiError::no_such_upload(resource, request_id))
        }
    }
}

fn validate_s3_multipart_upload_id(
    upload_id: &str,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    let suffix = upload_id.strip_prefix("mh_mpu_");
    if suffix.is_some_and(|value| {
        value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) {
        Ok(())
    } else {
        Err(S3ApiError::no_such_upload(resource, request_id))
    }
}

async fn discard_unbound_multipart_temporary(state: &AppState, storage_key: &str) {
    if let Err(error) = state.object_store.delete(storage_key).await {
        warn!(storage_key, error = %error, "failed to delete unbound multipart temporary object");
    }
}

async fn persist_multipart_temporary_gc(
    state: &AppState,
    upload: &S3MultipartUpload,
    storage_key: String,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    state
        .repository
        .enqueue_storage_gc_task(NewStorageGcTask {
            id: StorageGcTaskId::new(),
            application_id: upload.application_id,
            bucket_id: upload.bucket_id,
            object_version_id: None,
            upload_intent_id: None,
            multipart_upload_id: Some(upload.upload_id.clone()),
            storage_backend: upload.storage_backend.clone(),
            storage_key,
            reason: StorageGcReason::MultipartTemporary,
            not_before: now,
            max_attempts: DEFAULT_S3_MULTIPART_GC_MAX_ATTEMPTS,
            created_at: now,
        })
        .await
}

async fn find_s3_multipart_upload(
    state: &AppState,
    authorization: &S3AuthorizedDataRequest,
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
    if authorization.bucket.bucket_name != bucket_name
        || upload.application_id != authorization.bucket.application_id
        || upload.bucket_id != authorization.bucket.bucket_id
        || upload.object_key != object_key
    {
        return Err(S3ApiError::no_such_upload(resource, request_id));
    }
    Ok(upload)
}

struct S3MultipartAuthorizationRequest<'a> {
    state: &'a AppState,
    auth: &'a ApplicationAuth,
    action: S3PolicyAction,
    bucket_name: &'a str,
    object_key: &'a str,
    uri: &'a Uri,
    source_ip: Option<std::net::IpAddr>,
    request_id: &'a str,
}

async fn authorize_s3_multipart_upload_request(
    request: S3MultipartAuthorizationRequest<'_>,
) -> Result<S3AuthorizedDataRequest, S3ApiError> {
    let S3MultipartAuthorizationRequest {
        state,
        auth,
        action,
        bucket_name,
        object_key,
        uri,
        source_ip,
        request_id,
    } = request;
    let authorized = authorize_s3_signed_data_request(
        state,
        auth,
        S3DataAuthorizationInput {
            action,
            bucket_name,
            object_key: Some(object_key),
            version_id: None,
            prefix: None,
            delimiter: None,
            max_keys: None,
            secure_transport: s3_data_secure_transport(uri),
            source_ip,
        },
        uri.path(),
        request_id,
    )
    .await?;
    Ok(authorized)
}

async fn s3_list_parts(
    operation: S3ObjectOperation<'_>,
    upload_id: &str,
    source_ip: Option<std::net::IpAddr>,
) -> Result<Response, S3ApiError> {
    let S3ObjectOperation {
        state,
        auth,
        bucket_name,
        object_key,
        uri,
        request_id,
    } = operation;
    let authorization = authorize_s3_multipart_upload_request(S3MultipartAuthorizationRequest {
        state,
        auth,
        action: S3PolicyAction::ListMultipartUploadParts,
        bucket_name,
        object_key,
        uri,
        source_ip,
        request_id,
    })
    .await?;
    let upload = find_s3_multipart_upload(
        state,
        &authorization,
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
                    etag: format!("\"{}\"", part.etag),
                    size: part.size,
                })
            })
            .collect::<Result<Vec<_>, S3ApiError>>()?,
    })
    .map_err(|error| S3ApiError::from_xml(error, uri.path(), request_id))?;
    Ok(s3_xml_response(StatusCode::OK, body, request_id))
}

async fn s3_abort_multipart_upload(
    operation: S3ObjectOperation<'_>,
    upload_id: &str,
    source_ip: Option<std::net::IpAddr>,
) -> Result<Response, S3ApiError> {
    let S3ObjectOperation {
        state,
        auth,
        bucket_name,
        object_key,
        uri,
        request_id,
    } = operation;
    let authorization = authorize_s3_multipart_upload_request(S3MultipartAuthorizationRequest {
        state,
        auth,
        action: S3PolicyAction::AbortMultipartUpload,
        bucket_name,
        object_key,
        uri,
        source_ip,
        request_id,
    })
    .await?;
    find_s3_multipart_upload(
        state,
        &authorization,
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
        S3MultipartAbort::Aborted(_) | S3MultipartAbort::AlreadyAborted(_) => {}
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
    super::record_s3_resource_audit(
        state,
        auth,
        authorization.bucket.application_id,
        request_id,
        "s3.multipart_aborted",
        ("multipart_upload", upload_id.to_owned()),
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
    source_ip: Option<std::net::IpAddr>,
) -> Result<Response, S3ApiError> {
    let S3ObjectOperation {
        state,
        auth,
        bucket_name,
        object_key,
        uri,
        request_id,
    } = operation;
    let resource = uri.path();
    validate_s3_object_key(object_key, resource, request_id)?;
    let authorization = authorize_s3_multipart_upload_request(S3MultipartAuthorizationRequest {
        state,
        auth,
        action: S3PolicyAction::PutObject,
        bucket_name,
        object_key,
        uri,
        source_ip,
        request_id,
    })
    .await?;
    let upload = find_s3_multipart_upload(
        state,
        &authorization,
        bucket_name,
        object_key,
        upload_id,
        resource,
        request_id,
    )
    .await?;
    if upload.state == S3MultipartUploadState::Completed {
        return completed_multipart_response(
            state,
            &upload,
            resource,
            bucket_name,
            object_key,
            request_id,
        )
        .await;
    }

    let request = parse_complete_multipart_upload_xml(content)
        .map_err(|error| S3ApiError::from_xml(error, resource, request_id))?;
    let manifest = request
        .parts
        .into_iter()
        .map(|part| {
            let etag = canonical_completed_part_etag(&part.etag).ok_or_else(|| {
                S3ApiError::from_multipart_manifest(
                    S3MultipartManifestError::EtagMismatch(part.part_number),
                    resource,
                    request_id,
                )
            })?;
            Ok(CompletedS3MultipartPart {
                part_number: part.part_number,
                etag,
            })
        })
        .collect::<Result<Vec<_>, S3ApiError>>()?;
    let completion_token = uuid::Uuid::now_v7().simple().to_string();
    let now = OffsetDateTime::now_utc();
    let lease_until = now + time::Duration::seconds(S3_MULTIPART_COMPLETION_LEASE_SECONDS);
    let claim = state
        .repository
        .claim_multipart_completion(upload_id, &manifest, &completion_token, lease_until, now)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart completion claim failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?;
    let manifest = match claim {
        S3MultipartCompletionClaim::Claimed(manifest) => manifest,
        S3MultipartCompletionClaim::AlreadyCompleted(upload) => {
            return completed_multipart_response(
                state,
                &upload,
                resource,
                bucket_name,
                object_key,
                request_id,
            )
            .await;
        }
        S3MultipartCompletionClaim::InProgress(_) => {
            return Err(S3ApiError::operation_aborted(
                "A conflicting multipart completion is in progress.",
                resource,
                request_id,
            ));
        }
        S3MultipartCompletionClaim::Aborted(_) | S3MultipartCompletionClaim::Expired(_) => {
            return Err(S3ApiError::no_such_upload(resource, request_id));
        }
        S3MultipartCompletionClaim::InvalidManifest(error) => {
            return Err(S3ApiError::from_multipart_manifest(
                error, resource, request_id,
            ));
        }
    };
    if manifest
        .parts
        .iter()
        .take(manifest.parts.len().saturating_sub(1))
        .any(|part| part.size < MIN_S3_MULTIPART_PART_BYTES)
    {
        if manifest.upload.upload_intent_id.is_none() {
            release_multipart_claim(state, upload_id, &completion_token).await;
        }
        return Err(S3ApiError::entity_too_small(resource, request_id));
    }
    if let Err(error) = validate_s3_object_size(manifest.total_size) {
        if manifest.upload.upload_intent_id.is_none() {
            release_multipart_claim(state, upload_id, &completion_token).await;
        }
        return Err(S3ApiError::from_api(error, resource, request_id));
    }
    let final_entity_tag = multipart_entity_tag(&manifest.parts)
        .ok_or_else(|| S3ApiError::service_unavailable(resource, request_id))?;
    let service = runtime_s3_object_service(state, resource, request_id)?;
    let resumed = claim_existing_multipart_intent(MultipartIntentTakeover {
        state,
        manifest: &manifest,
        final_entity_tag: &final_entity_tag,
        completion_token: &completion_token,
        lease_until,
        now,
        resource,
        request_id,
    })
    .await?;
    let (committing, final_checksum, composed_size) = if let Some(resumed) = resumed {
        resumed
    } else {
        let intent = match service
            .begin_put(&BeginPutObjectRequest {
                application_id: authorization.application_id(),
                bucket_name: bucket_name.to_owned(),
                object_key: object_key.to_owned(),
                expected_size_bytes: manifest.total_size,
                content_type: Some(manifest.upload.content_type.clone()),
                user_metadata: manifest.upload.user_metadata.clone(),
                object_tags: manifest.upload.object_tags.clone(),
                expires_at: Some(manifest.upload.expires_at),
            })
            .await
        {
            Ok(receipt) => receipt.intent,
            Err(error) => {
                release_multipart_claim(state, upload_id, &completion_token).await;
                return Err(map_s3_object_service_error(error, resource, request_id));
            }
        };
        let source_keys = manifest
            .parts
            .iter()
            .map(|part| part.storage_key.clone())
            .collect::<Vec<_>>();
        let composed = match state
            .object_store
            .compose_temporary(
                intent.temporary_storage_key(),
                &source_keys,
                &manifest.upload.content_type,
            )
            .await
        {
            Ok(composed) => composed,
            Err(error) => {
                abort_s3_staged_put(state, &service, authorization.application_id(), intent.id())
                    .await;
                release_multipart_claim(state, upload_id, &completion_token).await;
                warn!(error = %error, "S3 multipart composition failed");
                return Err(S3ApiError::service_unavailable(resource, request_id));
            }
        };
        if composed.size != manifest.total_size {
            abort_s3_staged_put(state, &service, authorization.application_id(), intent.id()).await;
            release_multipart_claim(state, upload_id, &completion_token).await;
            return Err(S3ApiError::service_unavailable(resource, request_id));
        }
        let final_checksum = Checksum::sha256_hex(&composed.sha256)
            .map_err(|_| S3ApiError::service_unavailable(resource, request_id))?;
        if let Err(error) = state
            .repository
            .complete_upload_intent_staging(
                intent.id(),
                &final_entity_tag,
                &final_checksum,
                composed.size,
                OffsetDateTime::now_utc(),
            )
            .await
        {
            abort_s3_staged_put(state, &service, authorization.application_id(), intent.id()).await;
            release_multipart_claim(state, upload_id, &completion_token).await;
            return Err(S3ApiError::from_api(
                ApiError::from_repository(error),
                resource,
                request_id,
            ));
        }
        let committing = match state
            .repository
            .claim_upload_intent(
                intent.id(),
                &completion_token,
                lease_until,
                OffsetDateTime::now_utc(),
            )
            .await
        {
            Ok(intent) => intent,
            Err(error) => {
                abort_s3_staged_put(state, &service, authorization.application_id(), intent.id())
                    .await;
                release_multipart_claim(state, upload_id, &completion_token).await;
                return Err(S3ApiError::from_api(
                    ApiError::from_repository(error),
                    resource,
                    request_id,
                ));
            }
        };
        (committing, final_checksum, composed.size)
    };
    if let Err(error) = state
        .repository
        .attach_multipart_upload_intent(
            upload_id,
            &completion_token,
            committing.id(),
            OffsetDateTime::now_utc(),
        )
        .await
    {
        return Err(S3ApiError::from_api(
            ApiError::from_repository(error),
            resource,
            request_id,
        ));
    }
    if let Err(error) = service
        .promote_claimed_upload_intent(&committing, &completion_token)
        .await
    {
        return Err(map_s3_object_service_error(error, resource, request_id));
    }
    let prepared = service
        .prepare_claimed_upload_commit(PrepareClaimedUploadCommitRequest {
            intent: &committing,
            lease_token: &completion_token,
            entity_tag: &final_entity_tag,
            checksum: &final_checksum,
            size_bytes: composed_size,
            created_by: &auth.actor_id,
            source_protocol: mediahub_core::SourceProtocol::S3,
        })
        .await
        .map_err(|error| map_s3_object_service_error(error, resource, request_id))?;
    let version_id = prepared.version.id();
    let object = state
        .repository
        .commit_multipart_object_version(
            upload_id,
            &completion_token,
            prepared.commit,
            &final_entity_tag,
            &final_checksum,
        )
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 multipart object-version commit failed");
            S3ApiError::from_api(ApiError::from_repository(error), resource, request_id)
        })?;
    let version = state
        .repository
        .find_s3_object_version_by_id(version_id)
        .await
        .map_err(|error| {
            warn!(error = %error, "committed multipart ObjectVersion lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .filter(|version| {
            version.object_id() == object.id()
                && version.application_id() == authorization.application_id()
                && version.bucket_id() == object.bucket_id()
        })
        .ok_or_else(|| S3ApiError::service_unavailable(resource, request_id))?;
    state
        .http_metrics
        .uploaded_bytes
        .fetch_add(composed_size, Ordering::Relaxed);
    super::record_s3_resource_audit(
        state,
        auth,
        authorization.bucket.application_id,
        request_id,
        "s3.object.uploaded",
        ("object_version", version.id().to_string()),
        serde_json::json!({
            "bucket": bucket_name,
            "object_key": object_key,
            "object_id": object.id().to_string(),
            "version_id": version.external_version_id().as_str(),
            "size": composed_size,
            "protocol": "s3",
            "multipart": true,
        }),
    )
    .await;
    s3_complete_multipart_response(
        resource,
        bucket_name,
        object_key,
        final_entity_tag.as_str(),
        version.external_version_id(),
        request_id,
    )
}

struct MultipartIntentTakeover<'a> {
    state: &'a AppState,
    manifest: &'a S3MultipartManifest,
    final_entity_tag: &'a EntityTag,
    completion_token: &'a str,
    lease_until: OffsetDateTime,
    now: OffsetDateTime,
    resource: &'a str,
    request_id: &'a str,
}

async fn claim_existing_multipart_intent(
    context: MultipartIntentTakeover<'_>,
) -> Result<Option<(UploadIntent, Checksum, u64)>, S3ApiError> {
    let MultipartIntentTakeover {
        state,
        manifest,
        final_entity_tag,
        completion_token,
        lease_until,
        now,
        resource,
        request_id,
    } = context;
    let Some(intent_id) = manifest.upload.upload_intent_id else {
        return Ok(None);
    };
    let intent = state
        .repository
        .find_upload_intent(intent_id)
        .await
        .map_err(|error| {
            warn!(upload_id = %manifest.upload.upload_id, intent_id = %intent_id, error = %error, "failed to load attached multipart upload intent");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .ok_or_else(|| S3ApiError::service_unavailable(resource, request_id))?;
    let state_is_recoverable = intent.state() == UploadIntentState::Ready
        || (intent.state() == UploadIntentState::Committing
            && intent.lease_until().is_some_and(|until| until <= now));
    let facts_match = intent.application_id() == manifest.upload.application_id
        && intent.bucket_id() == manifest.upload.bucket_id
        && intent.object_key() == manifest.upload.object_key
        && intent.storage_backend() == manifest.upload.storage_backend
        && intent.expected_size_bytes() == manifest.total_size
        && intent.size_bytes() == Some(manifest.total_size)
        && intent.content_type() == Some(manifest.upload.content_type.as_str())
        && intent.user_metadata() == &manifest.upload.user_metadata
        && intent.entity_tag() == Some(final_entity_tag)
        && intent
            .checksum()
            .is_some_and(|checksum| checksum.algorithm() == ChecksumAlgorithm::Sha256)
        && intent.expires_at() > now;
    if !state_is_recoverable || !facts_match {
        warn!(
            upload_id = %manifest.upload.upload_id,
            intent_id = %intent_id,
            intent_state = ?intent.state(),
            "attached multipart upload intent cannot be safely resumed"
        );
        return Err(S3ApiError::service_unavailable(resource, request_id));
    }
    let checksum = intent
        .checksum()
        .cloned()
        .ok_or_else(|| S3ApiError::service_unavailable(resource, request_id))?;
    let committing = state
        .repository
        .claim_upload_intent(intent.id(), completion_token, lease_until, now)
        .await
        .map_err(|error| {
            warn!(upload_id = %manifest.upload.upload_id, intent_id = %intent_id, error = %error, "failed to take over attached multipart upload intent");
            S3ApiError::service_unavailable(resource, request_id)
        })?;
    Ok(Some((committing, checksum, manifest.total_size)))
}
fn canonical_completed_part_etag(value: &str) -> Option<String> {
    let value = value.trim();
    let value = match (value.strip_prefix('"'), value.strip_suffix('"')) {
        (Some(without_prefix), Some(_)) => without_prefix.strip_suffix('"')?,
        (None, None) => value,
        _ => return None,
    };
    is_lowercase_md5_hex(value).then(|| value.to_owned())
}

async fn release_multipart_claim(state: &AppState, upload_id: &str, completion_token: &str) {
    if let Err(error) = state
        .repository
        .release_multipart_completion(upload_id, completion_token, OffsetDateTime::now_utc())
        .await
    {
        warn!(upload_id, error = %error, "failed to release multipart completion claim");
    }
}

async fn completed_multipart_response(
    state: &AppState,
    upload: &S3MultipartUpload,
    resource: &str,
    bucket_name: &str,
    object_key: &str,
    request_id: &str,
) -> Result<Response, S3ApiError> {
    let etag = upload
        .final_etag
        .as_ref()
        .ok_or_else(|| S3ApiError::service_unavailable(resource, request_id))?;
    let object_id = upload
        .object_id
        .ok_or_else(|| S3ApiError::service_unavailable(resource, request_id))?;
    let version_id = upload
        .object_version_id
        .ok_or_else(|| S3ApiError::service_unavailable(resource, request_id))?;
    let version = state
        .repository
        .find_s3_object_version_by_id(version_id)
        .await
        .map_err(|error| {
            warn!(error = %error, "completed multipart version lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .filter(|version| {
            version.object_id() == object_id
                && version.application_id() == upload.application_id
                && version.bucket_id() == upload.bucket_id
        })
        .ok_or_else(|| S3ApiError::service_unavailable(resource, request_id))?;
    s3_complete_multipart_response(
        resource,
        bucket_name,
        object_key,
        etag.as_str(),
        version.external_version_id(),
        request_id,
    )
}

#[cfg(test)]
include!("s3_multipart_policy_tests.rs");
