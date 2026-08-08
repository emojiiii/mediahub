// Version-aware regular S3 object HTTP operations.

const MAX_S3_CONTENT_TYPE_BYTES: usize = 1_024;
const MAX_S3_USER_METADATA_NAME_BYTES: usize = 128;
const MAX_S3_USER_METADATA_BYTES: usize = 2 * 1_024;

fn validate_s3_object_size(size: u64) -> Result<(), ApiError> {
    if size > MAX_UPLOAD_OBJECT_BYTES {
        Err(ApiError::payload_too_large(
            "object size exceeds the 2 GiB S3 object limit",
        ))
    } else {
        Ok(())
    }
}

type RuntimeS3ObjectService = S3ObjectService<
    mediahub_adapter_postgres::PostgresRepository,
    mediahub_adapter_postgres::PostgresRepository,
    mediahub_adapter_postgres::PostgresRepository,
    super::runtime_storage::RuntimeObjectStore,
    SystemClock,
>;

fn runtime_s3_object_service(
    state: &AppState,
    resource: &str,
    request_id: &str,
) -> Result<RuntimeS3ObjectService, S3ApiError> {
    S3ObjectService::new(
        state.repository.clone(),
        state.repository.clone(),
        state.repository.clone(),
        state.object_store.clone(),
        SystemClock,
    )
    .with_gc_grace(state.s3_gc_grace)
    .map_err(|error| map_s3_object_service_error(error, resource, request_id))
}

async fn s3_put_regular_object(
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
    auth.authorize("media:upload")
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    validate_s3_object_key(object_key, resource, request_id)?;
    if s3_query_value(uri, "versionId", request_id)?.is_some() {
        return Err(S3ApiError::invalid_argument(
            "versionId is not valid for PutObject.",
            resource,
            request_id,
        ));
    }

    let expected_size = s3_content_length(headers, resource, request_id)?;
    validate_s3_object_size(expected_size)
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    let content_type = s3_object_content_type(headers, resource, request_id)?;
    let user_metadata = s3_user_metadata(headers, resource, request_id)?;
    let object_tags = parse_s3_tagging_header(headers, resource, request_id)?.unwrap_or_default();
    let expected_md5 = parse_content_md5(headers, resource, request_id)?;
    let object_lock =
        parse_put_object_lock_headers(headers, uri, request_id, OffsetDateTime::now_utc())?;
    ensure_put_object_lock_bucket(
        state,
        auth,
        bucket_name,
        object_lock,
        resource,
        request_id,
    )
    .await?;

    // The durable owner is created before the request body stream is touched.
    let service = runtime_s3_object_service(state, resource, request_id)?;
    let receipt = service
        .begin_put(&BeginPutObjectRequest {
            application_id: auth.application.id,
            bucket_name: bucket_name.to_owned(),
            object_key: object_key.to_owned(),
            expected_size_bytes: expected_size,
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
        .put_temporary_stream(
            receipt.intent.temporary_storage_key(),
            content.into_data_stream(),
            expected_size,
            &content_type,
        )
        .await
    {
        Ok(streamed) => streamed,
        Err(error) => {
            abort_s3_staged_put(state, &service, auth.application.id, intent_id).await;
            return Err(s3_streaming_upload_error(
                error,
                expected_size,
                resource,
                request_id,
            ));
        }
    };

    if let Err(error) = signature.verify_payload_sha256(&streamed.sha256) {
        abort_s3_staged_put(state, &service, auth.application.id, intent_id).await;
        return Err(S3ApiError::from_sigv4(error, resource, request_id));
    }
    if expected_md5
        .as_deref()
        .is_some_and(|expected| !expected.eq_ignore_ascii_case(&streamed.md5))
    {
        abort_s3_staged_put(state, &service, auth.application.id, intent_id).await;
        return Err(S3ApiError::bad_digest(resource, request_id));
    }

    let streamed_size = streamed.size;
    let completed = match service
        .complete_put_with_object_lock(
            &CompletePutObjectRequest {
                application_id: auth.application.id,
                intent_id,
                streamed,
                created_by: auth.actor_id.clone(),
                source_protocol: mediahub_core::SourceProtocol::S3,
            },
            object_lock,
        )
        .await
    {
        Ok(completed) => completed,
        Err(error) => {
            abort_s3_staged_put(state, &service, auth.application.id, intent_id).await;
            return Err(map_s3_object_service_error(error, resource, request_id));
        }
    };
    let payload = stored_object_payload(&completed.version, resource, request_id)?;

    state
        .http_metrics
        .uploaded_bytes
        .fetch_add(streamed_size, Ordering::Relaxed);
    record_audit(
        state,
        auth,
        request_id,
        "s3.object.uploaded",
        "s3_object",
        completed.object.id().to_string(),
        serde_json::json!({
            "bucket": bucket_name,
            "object_key": object_key,
            "size": streamed_size,
            "version_id": completed.version.external_version_id().as_str(),
            "protocol": "s3",
        }),
    )
    .await;

    let mut response = s3_empty_response(StatusCode::OK, request_id);
    response
        .headers_mut()
        .insert(ETAG, entity_tag_header_value(payload.etag().as_str()));
    insert_s3_version_id(
        response.headers_mut(),
        completed.version.external_version_id(),
    )?;
    insert_s3_object_lock_headers(response.headers_mut(), &completed.version)?;
    Ok(response)
}

async fn abort_s3_staged_put(
    state: &AppState,
    service: &RuntimeS3ObjectService,
    application_id: ApplicationId,
    intent_id: UploadIntentId,
) {
    if let Err(error) = service
        .abort_staged_put(&AbortStagedPutRequest {
            application_id,
            intent_id,
        })
        .await
    {
        warn!(%intent_id, error = %error, "failed to abort rejected S3 staged PutObject");
        return;
    }
    // The durable GC task owns physical cleanup. This probe only makes the
    // state dependency explicit and keeps the state parameter useful in logs.
    let _ = state.object_store.backend_name();
}

async fn s3_read_regular_object(
    operation: S3ObjectOperation<'_>,
    method: Method,
    headers: HeaderMap,
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
    auth.authorize("media:read")
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    validate_s3_object_key(object_key, resource, request_id)?;
    let version_id = parse_s3_version_id(uri, request_id)?;
    let service = runtime_s3_object_service(state, resource, request_id)?;
    let head = service
        .head(&S3ObjectRequest {
            application_id: auth.application.id,
            bucket_name: bucket_name.to_owned(),
            object_key: object_key.to_owned(),
            version_id,
        })
        .await
        .map_err(|error| map_s3_object_service_error(error, resource, request_id))?;
    let payload = stored_object_payload(&head.version, resource, request_id)?;
    let etag = payload.etag().as_str();

    if !if_match_satisfied(&headers, etag, resource, request_id)? {
        return Err(S3ApiError::precondition_failed(resource, request_id));
    }
    if if_none_match_matches_s3(&headers, etag, resource, request_id)? {
        let mut response = s3_empty_response(StatusCode::NOT_MODIFIED, request_id);
        insert_s3_object_headers(
            response.headers_mut(),
            &head.version,
            payload,
            None,
            head.tags.len(),
        )?;
        return Ok(response);
    }

    let range = headers
        .get(RANGE)
        .map(|value| {
            value
                .to_str()
                .map_err(|_| S3ApiError::invalid_range(resource, request_id))
                .and_then(|value| {
                    parse_s3_single_range(value, payload.size_bytes(), resource, request_id)
                })
        })
        .transpose()?;
    let head_only = method == Method::HEAD;
    let body = if head_only {
        Body::empty()
    } else if let Some(range) = &range {
        state
            .object_store
            .read_range(payload.storage_key(), range.start..range.end_exclusive)
            .await
            .map(Body::from)
            .map_err(|error| map_s3_object_store_read_error(error, resource, request_id))?
    } else if let Some(local_store) = state.object_store.local_store() {
        local_store
            .open_file(payload.storage_key())
            .await
            .map(local_file_body)
            .map_err(|error| map_s3_object_store_read_error(error, resource, request_id))?
    } else {
        state
            .object_store
            .read(payload.storage_key())
            .await
            .map(Body::from)
            .map_err(|error| map_s3_object_store_read_error(error, resource, request_id))?
    };
    let status = if range.is_some() {
        StatusCode::PARTIAL_CONTENT
    } else {
        StatusCode::OK
    };
    let mut response = (status, body).into_response();
    insert_s3_request_id(&mut response, request_id);
    insert_s3_object_headers(
        response.headers_mut(),
        &head.version,
        payload,
        range.as_ref(),
        head.tags.len(),
    )?;
    Ok(response)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct S3ByteRange {
    start: u64,
    end_exclusive: u64,
}

impl S3ByteRange {
    fn length(self) -> u64 {
        self.end_exclusive - self.start
    }
}

fn parse_s3_single_range(
    value: &str,
    total: u64,
    resource: &str,
    request_id: &str,
) -> Result<S3ByteRange, S3ApiError> {
    let value = value
        .strip_prefix("bytes=")
        .filter(|value| !value.contains(','))
        .ok_or_else(|| S3ApiError::invalid_range(resource, request_id))?;
    let (start, end) = value
        .split_once('-')
        .ok_or_else(|| S3ApiError::invalid_range(resource, request_id))?;
    if total == 0 {
        return Err(S3ApiError::invalid_range(resource, request_id));
    }

    if start.is_empty() {
        let suffix = end
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| S3ApiError::invalid_range(resource, request_id))?;
        return Ok(S3ByteRange {
            start: total.saturating_sub(suffix),
            end_exclusive: total,
        });
    }

    let start = start
        .parse::<u64>()
        .ok()
        .filter(|start| *start < total)
        .ok_or_else(|| S3ApiError::invalid_range(resource, request_id))?;
    let end_inclusive = if end.is_empty() {
        total - 1
    } else {
        end.parse::<u64>()
            .ok()
            .filter(|end| *end >= start)
            .ok_or_else(|| S3ApiError::invalid_range(resource, request_id))?
            .min(total - 1)
    };
    Ok(S3ByteRange {
        start,
        end_exclusive: end_inclusive + 1,
    })
}

fn s3_object_content_type(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<String, S3ApiError> {
    let mut values = headers.get_all(CONTENT_TYPE).iter();
    let Some(value) = values.next() else {
        return Ok("application/octet-stream".to_owned());
    };
    if values.next().is_some() {
        return Err(S3ApiError::invalid_argument(
            "Content-Type must not occur more than once.",
            resource,
            request_id,
        ));
    }
    let value = value.to_str().map_err(|_| {
        S3ApiError::invalid_argument("Content-Type is invalid.", resource, request_id)
    })?;
    if value.is_empty() || value.len() > MAX_S3_CONTENT_TYPE_BYTES {
        return Err(S3ApiError::invalid_argument(
            "Content-Type is invalid.",
            resource,
            request_id,
        ));
    }
    normalized_mime(value).map_err(|error| S3ApiError::from_api(error, resource, request_id))
}

fn s3_user_metadata(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<serde_json::Value, S3ApiError> {
    let mut metadata = serde_json::Map::new();
    let mut total_bytes = 0_usize;
    for name in headers.keys() {
        let Some(key) = name.as_str().strip_prefix("x-amz-meta-") else {
            continue;
        };
        if key.is_empty() || key.len() > MAX_S3_USER_METADATA_NAME_BYTES {
            return Err(S3ApiError::invalid_argument(
                "An x-amz-meta-* header name is invalid.",
                resource,
                request_id,
            ));
        }
        let mut values = headers.get_all(name).iter();
        let value = values
            .next()
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                S3ApiError::invalid_argument(
                    "An x-amz-meta-* header value is invalid.",
                    resource,
                    request_id,
                )
            })?;
        if values.next().is_some() {
            return Err(S3ApiError::invalid_argument(
                "An x-amz-meta-* header must not occur more than once.",
                resource,
                request_id,
            ));
        }
        total_bytes = total_bytes
            .checked_add(key.len() + value.len())
            .ok_or_else(|| S3ApiError::metadata_too_large(resource, request_id))?;
        if total_bytes > MAX_S3_USER_METADATA_BYTES {
            return Err(S3ApiError::metadata_too_large(resource, request_id));
        }
        metadata.insert(key.to_owned(), serde_json::Value::String(value.to_owned()));
    }
    Ok(serde_json::Value::Object(metadata))
}

fn parse_content_md5(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<Option<String>, S3ApiError> {
    let mut values = headers.get_all("content-md5").iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err(S3ApiError::invalid_digest(resource, request_id));
    }
    let decoded = STANDARD
        .decode(value.as_bytes())
        .map_err(|_| S3ApiError::invalid_digest(resource, request_id))?;
    if decoded.len() != 16 {
        return Err(S3ApiError::invalid_digest(resource, request_id));
    }
    Ok(Some(hex::encode(decoded)))
}

fn parse_s3_version_id(uri: &Uri, request_id: &str) -> Result<Option<S3VersionId>, S3ApiError> {
    s3_query_value(uri, "versionId", request_id)?
        .map(|value| {
            S3VersionId::new(value).map_err(|_| {
                S3ApiError::invalid_argument("versionId is invalid.", uri.path(), request_id)
            })
        })
        .transpose()
}

fn stored_object_payload<'a>(
    version: &'a ObjectVersion,
    resource: &str,
    request_id: &str,
) -> Result<&'a StoredObjectVersion, S3ApiError> {
    match version.payload() {
        ObjectVersionPayload::Object(payload) => Ok(payload),
        ObjectVersionPayload::DeleteMarker => Err(S3ApiError::delete_marker(
            version.external_version_id(),
            false,
            resource,
            request_id,
        )),
    }
}

fn insert_s3_object_headers(
    headers: &mut HeaderMap,
    version: &ObjectVersion,
    payload: &StoredObjectVersion,
    range: Option<&S3ByteRange>,
    tag_count: usize,
) -> Result<(), S3ApiError> {
    let resource = version.external_version_id().as_str();
    headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(
            &range
                .copied()
                .map(S3ByteRange::length)
                .unwrap_or_else(|| payload.size_bytes())
                .to_string(),
        )
        .expect("u64 content length is a valid header"),
    );
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(payload.content_type().unwrap_or("application/octet-stream"))
            .map_err(|_| S3ApiError::internal_error(resource, "response"))?,
    );
    headers.insert(ETAG, entity_tag_header_value(payload.etag().as_str()));
    headers.insert(
        LAST_MODIFIED,
        HeaderValue::from_str(&s3_http_date(version.created_at()))
            .expect("domain timestamps format as an HTTP date"),
    );
    if let Some(range) = range {
        headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&format!(
                "bytes {}-{}/{}",
                range.start,
                range.end_exclusive - 1,
                payload.size_bytes()
            ))
            .expect("validated range is a valid header"),
        );
    }
    insert_s3_version_id(headers, version.external_version_id())?;
    insert_s3_object_lock_headers(headers, version)?;
    if tag_count > 0 {
        headers.insert(
            HeaderName::from_static("x-amz-tagging-count"),
            HeaderValue::from_str(&tag_count.to_string())
                .expect("a tag count is a valid header value"),
        );
    }
    insert_s3_user_metadata_headers(headers, payload.user_metadata(), resource)
}

fn insert_s3_version_id(
    headers: &mut HeaderMap,
    version_id: &S3VersionId,
) -> Result<(), S3ApiError> {
    headers.insert(
        HeaderName::from_static("x-amz-version-id"),
        HeaderValue::from_str(version_id.as_str())
            .map_err(|_| S3ApiError::internal_error("versionId", "response"))?,
    );
    Ok(())
}

fn insert_s3_user_metadata_headers(
    headers: &mut HeaderMap,
    metadata: &serde_json::Value,
    resource: &str,
) -> Result<(), S3ApiError> {
    let metadata = metadata
        .as_object()
        .ok_or_else(|| S3ApiError::internal_error(resource, "response"))?;
    for (key, value) in metadata {
        let value = value
            .as_str()
            .ok_or_else(|| S3ApiError::internal_error(resource, "response"))?;
        let name = HeaderName::from_bytes(format!("x-amz-meta-{key}").as_bytes())
            .map_err(|_| S3ApiError::internal_error(resource, "response"))?;
        let value = HeaderValue::from_str(value)
            .map_err(|_| S3ApiError::internal_error(resource, "response"))?;
        headers.insert(name, value);
    }
    Ok(())
}

fn if_match_satisfied(
    headers: &HeaderMap,
    etag: &str,
    resource: &str,
    request_id: &str,
) -> Result<bool, S3ApiError> {
    let Some(value) = headers.get(IF_MATCH) else {
        return Ok(true);
    };
    let value = value
        .to_str()
        .map_err(|_| S3ApiError::invalid_argument("If-Match is invalid.", resource, request_id))?;
    Ok(value.trim() == "*"
        || value
            .split(',')
            .map(str::trim)
            .any(|candidate| !candidate.starts_with("W/") && candidate == format!("\"{etag}\"")))
}

fn if_none_match_matches_s3(
    headers: &HeaderMap,
    etag: &str,
    resource: &str,
    request_id: &str,
) -> Result<bool, S3ApiError> {
    let Some(value) = headers.get(IF_NONE_MATCH) else {
        return Ok(false);
    };
    let value = value.to_str().map_err(|_| {
        S3ApiError::invalid_argument("If-None-Match is invalid.", resource, request_id)
    })?;
    Ok(value.trim() == "*"
        || value
            .split(',')
            .map(str::trim)
            .map(|candidate| candidate.strip_prefix("W/").unwrap_or(candidate))
            .any(|candidate| candidate == format!("\"{etag}\"")))
}

fn s3_http_date(value: OffsetDateTime) -> String {
    let weekday = match value.weekday() {
        time::Weekday::Monday => "Mon",
        time::Weekday::Tuesday => "Tue",
        time::Weekday::Wednesday => "Wed",
        time::Weekday::Thursday => "Thu",
        time::Weekday::Friday => "Fri",
        time::Weekday::Saturday => "Sat",
        time::Weekday::Sunday => "Sun",
    };
    let month = match value.month() {
        time::Month::January => "Jan",
        time::Month::February => "Feb",
        time::Month::March => "Mar",
        time::Month::April => "Apr",
        time::Month::May => "May",
        time::Month::June => "Jun",
        time::Month::July => "Jul",
        time::Month::August => "Aug",
        time::Month::September => "Sep",
        time::Month::October => "Oct",
        time::Month::November => "Nov",
        time::Month::December => "Dec",
    };
    format!(
        "{weekday}, {:02} {month} {:04} {:02}:{:02}:{:02} GMT",
        value.day(),
        value.year(),
        value.hour(),
        value.minute(),
        value.second()
    )
}

fn map_s3_object_store_read_error(
    error: ObjectStoreError,
    resource: &str,
    request_id: &str,
) -> S3ApiError {
    match error {
        ObjectStoreError::NotFound => S3ApiError::no_such_key(resource, request_id),
        ObjectStoreError::InvalidRange => S3ApiError::invalid_range(resource, request_id),
        ObjectStoreError::AlreadyExists
        | ObjectStoreError::InvalidCursor
        | ObjectStoreError::InvalidLimit
        | ObjectStoreError::Unavailable(_) => {
            warn!(error = %error, "S3 committed object storage read failed");
            S3ApiError::service_unavailable(resource, request_id)
        }
    }
}

fn s3_delete_lock_message(reason: S3DeleteLockReason) -> &'static str {
    match reason {
        S3DeleteLockReason::LegalHold => {
            "Object deletion is denied because an active legal hold is in effect."
        }
        S3DeleteLockReason::ComplianceRetention => {
            "Object deletion is denied because the compliance retention period is active."
        }
        S3DeleteLockReason::GovernanceRetention => {
            "Object deletion is denied because the governance retention period is active."
        }
    }
}

fn map_s3_object_service_error(
    error: S3ObjectServiceError,
    resource: &str,
    request_id: &str,
) -> S3ApiError {
    match error {
        S3ObjectServiceError::BucketNotFound => S3ApiError::no_such_bucket(resource, request_id),
        S3ObjectServiceError::ObjectNotFound => S3ApiError::no_such_key(resource, request_id),
        S3ObjectServiceError::VersionNotFound => S3ApiError::no_such_version(resource, request_id),
        S3ObjectServiceError::DeleteMarker {
            version_id,
            is_current,
        } => S3ApiError::delete_marker(&version_id, is_current, resource, request_id),
        S3ObjectServiceError::DeleteLocked(reason) => {
            S3ApiError::access_denied(s3_delete_lock_message(reason), resource, request_id)
        }
        S3ObjectServiceError::ObjectLockNotEnabled => {
            S3ApiError::invalid_bucket_object_lock_configuration(resource, request_id)
        }
        S3ObjectServiceError::ObjectRetentionNotFound => {
            S3ApiError::no_such_object_lock_configuration(resource, request_id)
        }
        S3ObjectServiceError::RetentionUpdateLocked(reason) => {
            S3ApiError::access_denied(s3_delete_lock_message(reason), resource, request_id)
        }
        S3ObjectServiceError::InvalidObjectRetention => S3ApiError::invalid_argument(
            "RetainUntilDate must be in the future.",
            resource,
            request_id,
        ),
        S3ObjectServiceError::CompletionInProgress
        | S3ObjectServiceError::UploadIntentNotFound
        | S3ObjectServiceError::UploadIntentOwnershipMismatch
        | S3ObjectServiceError::UploadIntentFactsMismatch
        | S3ObjectServiceError::InvalidUploadIntentState(_)
        | S3ObjectServiceError::Repository(RepositoryError::Conflict) => {
            S3ApiError::operation_aborted(
                "A conflicting conditional operation is currently in progress against this resource.",
                resource,
                request_id,
            )
        }
        S3ObjectServiceError::Repository(RepositoryError::NotFound) => {
            S3ApiError::no_such_key(resource, request_id)
        }
        S3ObjectServiceError::Model(error) => {
            S3ApiError::invalid_argument(error.to_string(), resource, request_id)
        }
        S3ObjectServiceError::InvalidGcGrace => S3ApiError::internal_error(resource, request_id),
        S3ObjectServiceError::Repository(RepositoryError::QuotaExceeded) => {
            S3ApiError::entity_too_large(resource, request_id)
        }
        S3ObjectServiceError::Repository(RepositoryError::Invariant(error)) => {
            warn!(error, "S3 object repository invariant failed");
            S3ApiError::internal_error(resource, request_id)
        }
        S3ObjectServiceError::Repository(RepositoryError::Unavailable(error)) => {
            warn!(error, "S3 object repository unavailable");
            S3ApiError::service_unavailable(resource, request_id)
        }
        S3ObjectServiceError::Promotion(error) | S3ObjectServiceError::Storage(error) => {
            warn!(error = %error, "S3 object storage unavailable");
            S3ApiError::service_unavailable(resource, request_id)
        }
        S3ObjectServiceError::PromotionRelease { promotion, release } => {
            warn!(error = %promotion, release = %release, "S3 promotion and lease release failed");
            S3ApiError::service_unavailable(resource, request_id)
        }
        S3ObjectServiceError::PromotionUncertain { promotion, probe } => {
            warn!(error = %promotion, probe = %probe, "S3 promotion result is uncertain");
            S3ApiError::service_unavailable(resource, request_id)
        }
    }
}
