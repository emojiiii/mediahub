// S3 server-side copy operations. Source resolution is version-aware and the
// destination is committed through the same durable UploadIntent state
// machine used by PutObject.

#[derive(Clone, Debug, PartialEq, Eq)]
struct S3CopySource {
    bucket_name: String,
    object_key: String,
    version_id: Option<S3VersionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum S3MetadataDirective {
    Copy,
    Replace,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum S3TaggingDirective {
    Copy,
    Replace,
}

async fn s3_copy_object(
    operation: S3ObjectOperation<'_>,
    signature: &ParsedSigV4,
    headers: &HeaderMap,
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
    verify_s3_copy_body(signature, content, resource, request_id).await?;
    auth.authorize("media:read")
        .and_then(|_| auth.authorize("media:upload"))
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    reject_unsupported_s3_copy_headers(headers, false, resource, request_id)?;
    validate_s3_object_key(object_key, resource, request_id)?;
    if s3_query_value(uri, "versionId", request_id)?.is_some() {
        return Err(S3ApiError::invalid_argument(
            "versionId is not valid for CopyObject.",
            resource,
            request_id,
        ));
    }
    let source = parse_s3_copy_source(headers, resource, request_id)?;
    let directive = parse_s3_metadata_directive(headers, resource, request_id)?;
    let tagging_directive = parse_s3_tagging_directive(headers, resource, request_id)?;
    let replacement_tags = parse_s3_tagging_header(headers, resource, request_id)?;
    if tagging_directive == S3TaggingDirective::Copy && replacement_tags.is_some() {
        return Err(S3ApiError::invalid_request(
            "x-amz-tagging requires x-amz-tagging-directive=REPLACE for CopyObject.",
            resource,
            request_id,
        ));
    }
    if source.bucket_name == bucket_name
        && source.object_key == object_key
        && source.version_id.is_none()
        && directive == S3MetadataDirective::Copy
        && tagging_directive == S3TaggingDirective::Copy
    {
        return Err(S3ApiError::invalid_argument(
            "This copy request is illegal because it is copying an object to itself without changing its metadata.",
            resource,
            request_id,
        ));
    }

    let service = runtime_s3_object_service(state, resource, request_id)?;
    let source_head = service
        .head(&S3ObjectRequest {
            application_id: auth.application.id,
            bucket_name: source.bucket_name.clone(),
            object_key: source.object_key.clone(),
            version_id: source.version_id.clone(),
        })
        .await
        .map_err(|error| map_s3_object_service_error(error, resource, request_id))?;
    let source_payload = stored_object_payload(&source_head.version, resource, request_id)?;
    validate_s3_copy_source_conditions(
        headers,
        &source_head.version,
        source_payload,
        resource,
        request_id,
    )?;
    validate_s3_object_size(source_payload.size_bytes())
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;

    let (content_type, user_metadata) = match directive {
        S3MetadataDirective::Copy => (
            source_payload
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_owned(),
            source_payload.user_metadata().clone(),
        ),
        S3MetadataDirective::Replace => (
            s3_object_content_type(headers, resource, request_id)?,
            s3_user_metadata(headers, resource, request_id)?,
        ),
    };
    let object_tags = match tagging_directive {
        S3TaggingDirective::Copy => source_head.tags.clone(),
        S3TaggingDirective::Replace => replacement_tags.unwrap_or_default(),
    };
    let receipt = service
        .begin_put(&BeginPutObjectRequest {
            application_id: auth.application.id,
            bucket_name: bucket_name.to_owned(),
            object_key: object_key.to_owned(),
            expected_size_bytes: source_payload.size_bytes(),
            content_type: Some(content_type.clone()),
            user_metadata,
            object_tags,
            expires_at: None,
        })
        .await
        .map_err(|error| map_s3_object_service_error(error, resource, request_id))?;
    let intent_id = receipt.intent.id();
    let streamed = match state
        .object_store
        .copy_committed_to_temporary(
            source_payload.storage_key(),
            source_payload.size_bytes(),
            None,
            receipt.intent.temporary_storage_key(),
            &content_type,
        )
        .await
    {
        Ok(streamed) => streamed,
        Err(error) => {
            abort_s3_staged_put(state, &service, auth.application.id, intent_id).await;
            return Err(s3_streaming_upload_error(
                error,
                source_payload.size_bytes(),
                resource,
                request_id,
            ));
        }
    };
    let completed = match service
        .complete_put(&CompletePutObjectRequest {
            application_id: auth.application.id,
            intent_id,
            streamed,
            created_by: auth.actor_id.clone(),
            source_protocol: mediahub_core::SourceProtocol::S3,
        })
        .await
    {
        Ok(completed) => completed,
        Err(error) => {
            abort_s3_staged_put(state, &service, auth.application.id, intent_id).await;
            return Err(map_s3_object_service_error(error, resource, request_id));
        }
    };
    let destination_payload = stored_object_payload(&completed.version, resource, request_id)?;
    let last_modified = completed
        .version
        .created_at()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| S3ApiError::service_unavailable(resource, request_id))?;
    let body = copy_object_result_xml(
        &last_modified,
        &format!("\"{}\"", destination_payload.etag().as_str()),
    )
    .map_err(|error| S3ApiError::from_xml(error, resource, request_id))?;
    let mut response = s3_xml_response(StatusCode::OK, body, request_id);
    insert_s3_version_id(
        response.headers_mut(),
        completed.version.external_version_id(),
    )?;
    insert_s3_copy_source_version_id(
        response.headers_mut(),
        source_head.version.external_version_id(),
        resource,
        request_id,
    )?;
    record_audit(
        state,
        auth,
        request_id,
        "s3.object.copied",
        "s3_object",
        completed.object.id().to_string(),
        serde_json::json!({
            "protocol": "s3",
            "source_bucket": source.bucket_name,
            "source_key": source.object_key,
            "source_version_id": source_head.version.external_version_id().as_str(),
            "bucket": bucket_name,
            "object_key": object_key,
            "version_id": completed.version.external_version_id().as_str(),
        }),
    )
    .await;
    Ok(response)
}

async fn s3_upload_part_copy(
    operation: S3ObjectOperation<'_>,
    upload_id: &str,
    part_number: u16,
    signature: &ParsedSigV4,
    headers: &HeaderMap,
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
    verify_s3_copy_body(signature, content, resource, request_id).await?;
    auth.authorize("media:read")
        .and_then(|_| auth.authorize("media:upload"))
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    reject_unsupported_s3_copy_headers(headers, true, resource, request_id)?;
    let upload = find_s3_multipart_upload(
        state,
        auth,
        bucket_name,
        object_key,
        upload_id,
        resource,
        request_id,
    )
    .await?;
    if upload.state != S3MultipartUploadState::Pending
        || upload.expires_at <= OffsetDateTime::now_utc()
    {
        return Err(S3ApiError::no_such_upload(resource, request_id));
    }
    let source = parse_s3_copy_source(headers, resource, request_id)?;
    let service = runtime_s3_object_service(state, resource, request_id)?;
    let source_head = service
        .head(&S3ObjectRequest {
            application_id: auth.application.id,
            bucket_name: source.bucket_name,
            object_key: source.object_key,
            version_id: source.version_id,
        })
        .await
        .map_err(|error| map_s3_object_service_error(error, resource, request_id))?;
    let source_payload = stored_object_payload(&source_head.version, resource, request_id)?;
    validate_s3_copy_source_conditions(
        headers,
        &source_head.version,
        source_payload,
        resource,
        request_id,
    )?;
    let copy_range =
        parse_s3_copy_source_range(headers, source_payload.size_bytes(), resource, request_id)?;
    let copied_size = copy_range
        .map(S3ByteRange::length)
        .unwrap_or_else(|| source_payload.size_bytes());
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 UploadPartCopy bucket policy lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(resource, request_id))?;
    let maximum_upload_size = bucket
        .policy()
        .max_object_size()
        .unwrap_or(MAX_UPLOAD_OBJECT_BYTES)
        .min(MAX_UPLOAD_OBJECT_BYTES);
    if copied_size > maximum_upload_size {
        return Err(S3ApiError::entity_too_large(resource, request_id));
    }

    let storage_key = new_multipart_part_storage_key(upload_id, part_number);
    let streamed = match state
        .object_store
        .copy_committed_to_temporary(
            source_payload.storage_key(),
            source_payload.size_bytes(),
            copy_range.map(|range| range.start..range.end_exclusive),
            &storage_key,
            "application/octet-stream",
        )
        .await
    {
        Ok(streamed) => streamed,
        Err(error) => {
            persist_multipart_temporary_gc(state, &upload, storage_key, OffsetDateTime::now_utc())
                .await
                .map_err(|gc_error| {
                    warn!(error = %gc_error, "failed to persist rejected UploadPartCopy cleanup");
                    S3ApiError::service_unavailable(resource, request_id)
                })?;
            return Err(s3_streaming_upload_error(
                error,
                copied_size,
                resource,
                request_id,
            ));
        }
    };
    let etag = streamed.md5.clone();
    let now = OffsetDateTime::now_utc();
    let stored = state
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
            now,
        )
        .await;
    match stored {
        Ok(S3MultipartPartPut::Stored(_)) => {}
        Ok(S3MultipartPartPut::NotPending(_)) | Ok(S3MultipartPartPut::Expired(_)) => {
            persist_multipart_temporary_gc(state, &upload, storage_key, now)
                .await
                .map_err(|error| {
                    warn!(error = %error, "failed to persist terminal UploadPartCopy cleanup");
                    S3ApiError::service_unavailable(resource, request_id)
                })?;
            return Err(S3ApiError::no_such_upload(resource, request_id));
        }
        Err(error) => {
            persist_multipart_temporary_gc(state, &upload, storage_key, now)
                .await
                .map_err(|gc_error| {
                    warn!(error = %gc_error, "failed to persist rejected UploadPartCopy cleanup");
                    S3ApiError::service_unavailable(resource, request_id)
                })?;
            return Err(S3ApiError::from_api(
                ApiError::from_repository(error),
                resource,
                request_id,
            ));
        }
    }
    let last_modified = now
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| S3ApiError::service_unavailable(resource, request_id))?;
    let body = copy_part_result_xml(&last_modified, &format!("\"{etag}\""))
        .map_err(|error| S3ApiError::from_xml(error, resource, request_id))?;
    let mut response = s3_xml_response(StatusCode::OK, body, request_id);
    insert_s3_copy_source_version_id(
        response.headers_mut(),
        source_head.version.external_version_id(),
        resource,
        request_id,
    )?;
    Ok(response)
}

async fn verify_s3_copy_body(
    signature: &ParsedSigV4,
    content: Body,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    let content = to_bytes(content, MAX_ERROR_RESPONSE_BYTES)
        .await
        .map_err(|_| S3ApiError::entity_too_large(resource, request_id))?;
    if !content.is_empty() {
        return Err(S3ApiError::invalid_argument(
            "Copy requests must not include a request body.",
            resource,
            request_id,
        ));
    }
    signature
        .verify_payload_sha256(&hex::encode(Sha256::digest(&content)))
        .map_err(|error| S3ApiError::from_sigv4(error, resource, request_id))
}

fn parse_s3_copy_source(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<S3CopySource, S3ApiError> {
    let raw = required_single_s3_header(headers, "x-amz-copy-source", resource, request_id)?;
    let (raw_path, raw_query) = raw
        .split_once('?')
        .map_or((raw, None), |(path, query)| (path, Some(query)));
    let decoded = percent_encoding::percent_decode_str(raw_path)
        .decode_utf8()
        .map_err(|_| {
            S3ApiError::invalid_argument("x-amz-copy-source is invalid.", resource, request_id)
        })?;
    let decoded = decoded.strip_prefix('/').unwrap_or(&decoded);
    let (bucket_name, object_key) = decoded.split_once('/').ok_or_else(|| {
        S3ApiError::invalid_argument("x-amz-copy-source is invalid.", resource, request_id)
    })?;
    validate_s3_bucket_name(bucket_name).map_err(|()| {
        S3ApiError::invalid_argument("x-amz-copy-source bucket is invalid.", resource, request_id)
    })?;
    validate_s3_object_key(object_key, resource, request_id)?;
    let mut version_id = None;
    if let Some(query) = raw_query {
        for (name, value) in url::form_urlencoded::parse(query.as_bytes()) {
            if name != "versionId" || version_id.is_some() {
                return Err(S3ApiError::invalid_argument(
                    "x-amz-copy-source query is invalid.",
                    resource,
                    request_id,
                ));
            }
            version_id = Some(S3VersionId::new(value.into_owned()).map_err(|_| {
                S3ApiError::invalid_argument(
                    "x-amz-copy-source versionId is invalid.",
                    resource,
                    request_id,
                )
            })?);
        }
    }
    Ok(S3CopySource {
        bucket_name: bucket_name.to_owned(),
        object_key: object_key.to_owned(),
        version_id,
    })
}

fn parse_s3_metadata_directive(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<S3MetadataDirective, S3ApiError> {
    match optional_single_s3_header(headers, "x-amz-metadata-directive", resource, request_id)? {
        None | Some("COPY") => Ok(S3MetadataDirective::Copy),
        Some("REPLACE") => Ok(S3MetadataDirective::Replace),
        Some(_) => Err(S3ApiError::invalid_argument(
            "x-amz-metadata-directive must be COPY or REPLACE.",
            resource,
            request_id,
        )),
    }
}

fn parse_s3_tagging_directive(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<S3TaggingDirective, S3ApiError> {
    match optional_single_s3_header(headers, "x-amz-tagging-directive", resource, request_id)? {
        None | Some("COPY") => Ok(S3TaggingDirective::Copy),
        Some("REPLACE") => Ok(S3TaggingDirective::Replace),
        Some(_) => Err(S3ApiError::invalid_argument(
            "x-amz-tagging-directive must be COPY or REPLACE.",
            resource,
            request_id,
        )),
    }
}

fn reject_unsupported_s3_copy_headers(
    headers: &HeaderMap,
    part_copy: bool,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    if let Some(storage_class) =
        optional_single_s3_header(headers, "x-amz-storage-class", resource, request_id)?
        && storage_class != "STANDARD"
    {
        return Err(S3ApiError::not_implemented(
            "Only the STANDARD storage class is supported.",
            resource,
            request_id,
        ));
    }
    let unsupported = [
        "x-amz-checksum-algorithm",
        "x-amz-website-redirect-location",
        "x-amz-server-side-encryption",
        "x-amz-server-side-encryption-aws-kms-key-id",
        "x-amz-server-side-encryption-customer-algorithm",
        "x-amz-server-side-encryption-customer-key",
        "x-amz-server-side-encryption-customer-key-md5",
        "x-amz-copy-source-server-side-encryption-customer-algorithm",
        "x-amz-copy-source-server-side-encryption-customer-key",
        "x-amz-copy-source-server-side-encryption-customer-key-md5",
    ];
    if let Some(name) = unsupported
        .into_iter()
        .find(|name| headers.contains_key(*name))
    {
        return Err(S3ApiError::not_implemented(
            format!("{name} is not supported for copy operations."),
            resource,
            request_id,
        ));
    }
    if part_copy
        && let Some(name) = ["x-amz-tagging", "x-amz-tagging-directive"]
            .into_iter()
            .find(|name| headers.contains_key(*name))
    {
        return Err(S3ApiError::invalid_argument(
            format!("{name} is not valid for UploadPartCopy."),
            resource,
            request_id,
        ));
    }
    if part_copy && headers.contains_key("x-amz-metadata-directive") {
        return Err(S3ApiError::invalid_argument(
            "x-amz-metadata-directive is not valid for UploadPartCopy.",
            resource,
            request_id,
        ));
    }
    Ok(())
}

fn parse_s3_copy_source_range(
    headers: &HeaderMap,
    source_size: u64,
    resource: &str,
    request_id: &str,
) -> Result<Option<S3ByteRange>, S3ApiError> {
    let Some(value) =
        optional_single_s3_header(headers, "x-amz-copy-source-range", resource, request_id)?
    else {
        return Ok(None);
    };
    let (start, end) = value
        .strip_prefix("bytes=")
        .filter(|value| !value.contains(','))
        .and_then(|value| value.split_once('-'))
        .ok_or_else(|| S3ApiError::invalid_range(resource, request_id))?;
    let start = start
        .parse::<u64>()
        .map_err(|_| S3ApiError::invalid_range(resource, request_id))?;
    let end = end
        .parse::<u64>()
        .map_err(|_| S3ApiError::invalid_range(resource, request_id))?;
    if source_size == 0 || end < start || end >= source_size {
        return Err(S3ApiError::invalid_range(resource, request_id));
    }
    Ok(Some(S3ByteRange {
        start,
        end_exclusive: end + 1,
    }))
}

fn validate_s3_copy_source_conditions(
    headers: &HeaderMap,
    version: &ObjectVersion,
    payload: &StoredObjectVersion,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    let etag = payload.etag().as_str();
    let if_match =
        optional_single_s3_header(headers, "x-amz-copy-source-if-match", resource, request_id)?;
    if if_match.is_some_and(|value| !s3_copy_etag_matches(value, etag, false)) {
        return Err(S3ApiError::precondition_failed(resource, request_id));
    }
    if if_match.is_none()
        && let Some(value) = optional_single_s3_header(
            headers,
            "x-amz-copy-source-if-unmodified-since",
            resource,
            request_id,
        )?
    {
        let timestamp = parse_s3_copy_date(value, resource, request_id)?;
        if version.created_at().unix_timestamp() > timestamp {
            return Err(S3ApiError::precondition_failed(resource, request_id));
        }
    }

    let if_none_match = optional_single_s3_header(
        headers,
        "x-amz-copy-source-if-none-match",
        resource,
        request_id,
    )?;
    if if_none_match.is_some_and(|value| s3_copy_etag_matches(value, etag, true)) {
        return Err(S3ApiError::precondition_failed(resource, request_id));
    }
    if if_none_match.is_none()
        && let Some(value) = optional_single_s3_header(
            headers,
            "x-amz-copy-source-if-modified-since",
            resource,
            request_id,
        )?
    {
        let timestamp = parse_s3_copy_date(value, resource, request_id)?;
        if version.created_at().unix_timestamp() <= timestamp {
            return Err(S3ApiError::precondition_failed(resource, request_id));
        }
    }
    Ok(())
}

fn parse_s3_copy_date(value: &str, resource: &str, request_id: &str) -> Result<i64, S3ApiError> {
    chrono::DateTime::parse_from_rfc2822(value)
        .map(|value| value.timestamp())
        .map_err(|_| {
            S3ApiError::invalid_argument("A copy source date is invalid.", resource, request_id)
        })
}

fn s3_copy_etag_matches(value: &str, etag: &str, weak_allowed: bool) -> bool {
    value.trim() == "*"
        || value.split(',').map(str::trim).any(|candidate| {
            let candidate = if weak_allowed {
                candidate.strip_prefix("W/").unwrap_or(candidate)
            } else if candidate.starts_with("W/") {
                return false;
            } else {
                candidate
            };
            candidate
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                == Some(etag)
        })
}

fn required_single_s3_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
    resource: &str,
    request_id: &str,
) -> Result<&'a str, S3ApiError> {
    optional_single_s3_header(headers, name, resource, request_id)?.ok_or_else(|| {
        S3ApiError::invalid_argument(format!("{name} is required."), resource, request_id)
    })
}

fn optional_single_s3_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
    resource: &str,
    request_id: &str,
) -> Result<Option<&'a str>, S3ApiError> {
    let mut values = headers.get_all(name).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(S3ApiError::invalid_argument(
            format!("{name} must not occur more than once."),
            resource,
            request_id,
        ));
    }
    value.to_str().map(Some).map_err(|_| {
        S3ApiError::invalid_argument(format!("{name} is invalid."), resource, request_id)
    })
}

fn insert_s3_copy_source_version_id(
    headers: &mut HeaderMap,
    version_id: &S3VersionId,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    headers.insert(
        HeaderName::from_static("x-amz-copy-source-version-id"),
        HeaderValue::from_str(version_id.as_str())
            .map_err(|_| S3ApiError::internal_error(resource, request_id))?,
    );
    Ok(())
}
