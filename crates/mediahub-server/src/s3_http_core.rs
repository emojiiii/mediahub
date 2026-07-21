// S3 authentication, listing, and regular object operations.

async fn authenticate_s3_application(
    state: &AppState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: &[u8],
    request_id: &str,
) -> Result<ApplicationAuth, S3ApiError> {
    let (signature, secret, auth) =
        load_s3_authentication(state, method, uri, headers, request_id).await?;
    signature
        .verify(&secret, body)
        .map_err(|error| S3ApiError::from_sigv4(error, uri.path(), request_id))?;
    Ok(auth)
}

async fn authenticate_s3_streaming_application(
    state: &AppState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    request_id: &str,
) -> Result<(ApplicationAuth, ParsedSigV4), S3ApiError> {
    let (signature, secret, auth) =
        load_s3_authentication(state, method, uri, headers, request_id).await?;
    signature
        .verify_streaming_signature(&secret)
        .map_err(|error| S3ApiError::from_sigv4(error, uri.path(), request_id))?;
    Ok((auth, signature))
}

async fn load_s3_authentication(
    state: &AppState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    request_id: &str,
) -> Result<(ParsedSigV4, String, ApplicationAuth), S3ApiError> {
    let resource = uri.path();
    let signature = ParsedSigV4::parse(method, uri, headers, std::time::SystemTime::now())
        .map_err(|error| S3ApiError::from_sigv4(error, resource, request_id))?;
    let access_key = state
        .repository
        .find_active_access_key(signature.access_key_id(), OffsetDateTime::now_utc())
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 access key lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .ok_or_else(|| S3ApiError::invalid_access_key(resource, request_id))?;
    let secret = state
        .access_key_cipher
        .decrypt(&access_key.secret_ciphertext, access_key.secret_key_version)
        .map_err(|error| {
            warn!(error = %error, "S3 access key decryption failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?;
    let secret = std::str::from_utf8(&secret).map_err(|_| {
        warn!("S3 access key secret is not valid UTF-8");
        S3ApiError::service_unavailable(resource, request_id)
    })?.to_owned();
    let application = state
        .repository
        .find_application_by_id(access_key.application_id)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 application lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .ok_or_else(|| S3ApiError::invalid_access_key(resource, request_id))?;
    let auth = ApplicationAuth {
        application,
        actor_type: "access_key",
        actor_id: access_key.access_key_id.clone(),
        hmac_identity: Some(HmacIdentity {
            application_id: access_key.application_id,
            access_key_id: access_key.access_key_id,
            permissions: access_key.permissions,
        }),
    };
    Ok((signature, secret, auth))
}

pub(super) async fn s3_list_objects(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    auth.authorize("media:list")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let query = ListObjectsV2Query::parse(uri.query())
        .map_err(|error| S3ApiError::from_list(error, uri.path(), &request_id.0.0))?;
    let codec = s3_list_token_codec(&state);
    let cursor = query
        .decode_continuation_cursor(&codec, &bucket_name)
        .map_err(|error| S3ApiError::from_list(error, uri.path(), &request_id.0.0))?;
    let page = if query.max_keys == 0 {
        mediahub_app::S3MediaPage {
            items: Vec::new(),
            common_prefixes: Vec::new(),
            next_cursor: None,
        }
    } else {
        state
            .repository
            .list_s3_media_page(
                auth.application.id,
                &S3MediaListQuery {
                    bucket_id: bucket.id(),
                    object_key_prefix: query.prefix.clone(),
                    start_after: cursor.or_else(|| query.start_after.clone()),
                    delimiter: query.delimiter.is_some(),
                    limit: query.max_keys,
                },
            )
            .await
            .map_err(|error| {
                warn!(error = %error, "S3 object listing failed");
                S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
            })?
    };
    let result = ListObjectsV2Result {
        bucket: bucket_name,
        query,
        items: page
            .items
            .into_iter()
            .map(|media| ListObject {
                key: media.object_key().to_owned(),
                last_modified: media.created_at(),
                etag: media.etag().to_owned(),
                size: media.size(),
            })
            .collect(),
        common_prefixes: page.common_prefixes,
        next_cursor: page.next_cursor,
    };
    let body = result
        .to_xml(&codec)
        .map_err(|error| S3ApiError::from_list(error, uri.path(), &request_id.0.0))?;
    Ok(s3_xml_response(StatusCode::OK, body, &request_id.0.0))
}

fn s3_list_token_codec(state: &AppState) -> ContinuationTokenCodec {
    let mut digest = Sha256::new();
    digest.update(b"mediahub:s3:list-token:v1");
    digest.update(&state.media_url_signer.key);
    ContinuationTokenCodec::new(digest.finalize().into())
}

pub(super) async fn s3_bucket_post(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    content: Bytes,
) -> Result<Response, S3ApiError> {
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &content, &request_id.0.0)
            .await?;
    reject_s3_versioning(&uri, &request_id.0.0)?;
    if !s3_query_flag(&uri, "delete", &request_id.0.0)? {
        return Err(S3ApiError::not_implemented(
            "Only DeleteObjects is supported on the Bucket endpoint.",
            uri.path(),
            &request_id.0.0,
        ));
    }
    auth.authorize("media:delete")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    validate_content_md5(
        headers.get("content-md5").map(HeaderValue::as_bytes),
        &content,
    )
    .map_err(|error| {
        S3ApiError::new(
            StatusCode::BAD_REQUEST,
            error.s3_code(),
            error.to_string(),
            uri.path(),
            &request_id.0.0,
        )
    })?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let request = parse_delete_objects_xml(&content)
        .map_err(|error| S3ApiError::from_xml(error, uri.path(), &request_id.0.0))?;
    let mut result = DeleteResult::default();
    for object in request.objects {
        match schedule_s3_delete(&state, &auth, bucket.id(), &object.key, &request_id.0.0).await {
            Ok(()) if !request.quiet => result.deleted.push(DeletedObject { key: object.key }),
            Ok(()) => {}
            Err(error) => {
                let (code, message) = s3_batch_delete_error(&error);
                result.errors.push(DeleteObjectError {
                    key: object.key,
                    code: code.to_owned(),
                    message,
                });
            }
        }
    }
    let body = delete_result_xml(&result)
        .map_err(|error| S3ApiError::from_xml(error, uri.path(), &request_id.0.0))?;
    Ok(s3_xml_response(StatusCode::OK, body, &request_id.0.0))
}

async fn schedule_s3_delete(
    state: &AppState,
    auth: &ApplicationAuth,
    bucket_id: BucketId,
    object_key: &str,
    request_id: &str,
) -> Result<(), ApiError> {
    let Some(media) = state
        .repository
        .find_by_object_key(auth.application.id, bucket_id, object_key)
        .await
        .map_err(ApiError::from_repository)?
    else {
        return Ok(());
    };
    match media.state() {
        MediaState::DeletePending | MediaState::Deleted => return Ok(()),
        MediaState::Active => {}
        _ => {
            return Err(ApiError::conflict(
                "object cannot be deleted in its current state",
            ));
        }
    }
    let now = OffsetDateTime::now_utc();
    let event = OutboxEvent::media_delete_scheduled(&media, now, "s3");
    let media = state
        .repository
        .schedule_delete(media.id(), now, event)
        .await
        .map_err(ApiError::from_repository)?;
    record_audit(
        state,
        auth,
        request_id,
        "media.delete_scheduled",
        "media",
        media.id().to_string(),
        serde_json::json!({
            "reason": "s3",
            "protocol": "s3",
            "object_key": object_key,
        }),
    )
    .await;
    Ok(())
}

fn s3_batch_delete_error(error: &ApiError) -> (&'static str, String) {
    let code = match error.code {
        "forbidden" | "unauthorized" => "AccessDenied",
        "conflict" => "OperationAborted",
        "unavailable" => "ServiceUnavailable",
        _ => "InternalError",
    };
    (code, error.message.clone())
}

pub(super) async fn s3_put_object(
    State(state): State<Arc<AppState>>,
    Path((bucket_name, object_key)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    content: Body,
) -> Result<Response, S3ApiError> {
    let (auth, signature) = authenticate_s3_streaming_application(
        &state,
        &method,
        &uri,
        &headers,
        &request_id.0.0,
    )
    .await?;
    reject_s3_versioning(&uri, &request_id.0.0)?;
    if s3_query_flag(&uri, "acl", &request_id.0.0)? {
        let content = to_bytes(content, MAX_ERROR_RESPONSE_BYTES)
            .await
            .map_err(|_| S3ApiError::entity_too_large(uri.path(), &request_id.0.0))?;
        signature
            .verify_payload_sha256(&hex::encode(Sha256::digest(&content)))
            .map_err(|error| S3ApiError::from_sigv4(error, uri.path(), &request_id.0.0))?;
        return s3_put_object_acl(
            S3ObjectOperation {
                state: &state,
                auth: &auth,
                bucket_name: &bucket_name,
                object_key: &object_key,
                uri: &uri,
                request_id: &request_id.0.0,
            },
            &headers,
            &content,
        )
        .await;
    }
    let upload_id = s3_query_value(&uri, "uploadId", &request_id.0.0)?;
    let part_number = s3_query_value(&uri, "partNumber", &request_id.0.0)?;
    if upload_id.is_some() || part_number.is_some() {
        let upload_id = upload_id.ok_or_else(|| {
            S3ApiError::invalid_argument("uploadId is required.", uri.path(), &request_id.0.0)
        })?;
        let part_number = part_number
            .as_deref()
            .ok_or_else(|| {
                S3ApiError::invalid_argument("partNumber is required.", uri.path(), &request_id.0.0)
            })?
            .parse::<u16>()
            .ok()
            .filter(|value| (1..=10_000).contains(value))
            .ok_or_else(|| {
                S3ApiError::invalid_argument(
                    "partNumber must be between 1 and 10000.",
                    uri.path(),
                    &request_id.0.0,
                )
            })?;
        return s3_upload_part(
            S3ObjectOperation {
                state: &state,
                auth: &auth,
                bucket_name: &bucket_name,
                object_key: &object_key,
                uri: &uri,
                request_id: &request_id.0.0,
            },
            &upload_id,
            part_number,
            signature,
            &headers,
            content,
        )
        .await;
    }
    let visibility_override = s3_canned_acl(&headers, uri.path(), &request_id.0.0)?;
    if visibility_override.is_some() {
        auth.authorize("media:update")
            .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    }
    let media = s3_upload_object_stream(
        &state,
        &auth,
        &signature,
        &bucket_name,
        &object_key,
        &headers,
        content,
        visibility_override,
        uri.path(),
        &request_id.0.0,
    )
    .await?;
    let mut response = StatusCode::OK.into_response();
    response
        .headers_mut()
        .insert(ETAG, entity_tag_header_value(media.etag()));
    response.headers_mut().insert(
        HeaderName::from_static("x-amz-request-id"),
        HeaderValue::from_str(&request_id.0.0)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid-request-id")),
    );
    Ok(response)
}

#[allow(clippy::too_many_arguments)]
async fn s3_upload_object_stream(
    state: &AppState,
    auth: &ApplicationAuth,
    signature: &ParsedSigV4,
    bucket_name: &str,
    object_key: &str,
    headers: &HeaderMap,
    content: Body,
    visibility_override: Option<Visibility>,
    resource: &str,
    request_id: &str,
) -> Result<Media, S3ApiError> {
    auth.authorize("media:upload")
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    let expected_size = s3_content_length(headers, resource, request_id)?;
    validate_upload_expected_size(expected_size)
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 PutObject Bucket lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(resource, request_id))?;
    let (display_name, extension) = s3_object_names(object_key, resource, request_id)?;
    let mime = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            mime_guess::from_path(object_key)
                .first_raw()
                .unwrap_or("application/octet-stream")
                .to_owned()
        });
    let mime = normalized_mime(&mime)
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    let service = upload_session_service(state);
    let receipt = service
        .create(&CreateUploadSessionRequest {
            application_id: auth.application.id,
            bucket_id: bucket.id(),
            object_key: object_key.to_owned(),
            original_name: Some(display_name.clone()),
            display_name,
            extension,
            expected_size,
            expected_mime: mime.clone(),
            visibility_override,
            media_expires_at: None,
            metadata: ClientMetadata::default(),
        })
        .await
        .map_err(ApiError::from_application)
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    let session = receipt.session;
    let streamed = match state
        .object_store
        .put_temporary_stream(
            session.storage_key(),
            content.into_data_stream(),
            expected_size,
            &mime,
        )
        .await
    {
        Ok(streamed) => streamed,
        Err(error) => {
            discard_s3_gateway_session(state, auth.application.id, session.id()).await;
            return Err(s3_streaming_upload_error(
                error,
                expected_size,
                resource,
                request_id,
            ));
        }
    };
    if let Err(error) = signature.verify_payload_sha256(&streamed.sha256) {
        discard_s3_gateway_session(state, auth.application.id, session.id()).await;
        return Err(S3ApiError::from_sigv4(error, resource, request_id));
    }
    let completed = service
        .complete(&CompleteUploadSessionRequest {
            application_id: auth.application.id,
            upload_session_id: session.id(),
            sha256: streamed.sha256,
        })
        .await
        .map_err(ApiError::from_application)
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    state
        .http_metrics
        .uploaded_bytes
        .fetch_add(completed.media.size(), Ordering::Relaxed);
    record_audit(
        state,
        auth,
        request_id,
        "media.uploaded",
        "media",
        completed.media.id().to_string(),
        serde_json::json!({
            "bucket": bucket.name(),
            "object_key": completed.media.object_key(),
            "size": completed.media.size(),
            "protocol": "s3",
        }),
    )
    .await;
    Ok(completed.media)
}

fn s3_streaming_upload_error(
    error: StreamingUploadError,
    expected_size: u64,
    resource: &str,
    request_id: &str,
) -> S3ApiError {
    match error {
        StreamingUploadError::SizeMismatch { actual, .. } if actual < expected_size => {
            S3ApiError::incomplete_body(resource, request_id)
        }
        StreamingUploadError::SizeMismatch { .. } => {
            S3ApiError::invalid_argument("The request body exceeds Content-Length.", resource, request_id)
        }
        StreamingUploadError::Stream(_) => {
            S3ApiError::invalid_argument("The request body stream failed.", resource, request_id)
        }
        StreamingUploadError::Storage(ObjectStoreError::AlreadyExists) => {
            S3ApiError::operation_aborted("The upload target already exists.", resource, request_id)
        }
        StreamingUploadError::Storage(error) => {
            warn!(error = %error, "S3 streaming object storage failed");
            S3ApiError::service_unavailable(resource, request_id)
        }
    }
}

async fn discard_s3_gateway_session(
    state: &AppState,
    application_id: ApplicationId,
    upload_session_id: UploadSessionId,
) {
    let service = upload_session_service(state);
    let Ok(receipt) = service
        .cancel(&CancelUploadSessionRequest {
            application_id,
            upload_session_id,
        })
        .await
    else {
        warn!(%upload_session_id, "failed to cancel rejected S3 gateway upload session");
        return;
    };
    if let Err(error) = state.object_store.abort_upload(&receipt.session).await {
        warn!(%upload_session_id, error = %error, "failed to clean rejected S3 gateway upload storage");
        return;
    }
    if let Err(error) = state
        .repository
        .complete_upload_session_cleanup(upload_session_id)
        .await
    {
        warn!(%upload_session_id, error = %error, "failed to record rejected S3 gateway upload cleanup");
    }
}

pub(super) async fn s3_get_object(
    State(state): State<Arc<AppState>>,
    Path((bucket_name, object_key)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    reject_s3_versioning(&uri, &request_id.0.0)?;
    if s3_query_flag(&uri, "acl", &request_id.0.0)? {
        if method != Method::GET {
            return Err(S3ApiError::invalid_argument(
                "GetObjectAcl requires GET.",
                uri.path(),
                &request_id.0.0,
            ));
        }
        return s3_get_object_acl(
            &state,
            &auth,
            &bucket_name,
            &object_key,
            &uri,
            &request_id.0.0,
        )
        .await;
    }
    if let Some(upload_id) = s3_query_value(&uri, "uploadId", &request_id.0.0)? {
        if method != Method::GET {
            return Err(S3ApiError::invalid_argument(
                "ListParts requires GET.",
                uri.path(),
                &request_id.0.0,
            ));
        }
        return s3_list_parts(
            &state,
            &auth,
            &bucket_name,
            &object_key,
            &upload_id,
            &uri,
            &request_id.0.0,
        )
        .await;
    }
    auth.authorize("media:read")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let media = state
        .repository
        .find_by_object_key(auth.application.id, bucket.id(), &object_key)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 object lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .filter(|media| media.state().is_readable())
        .ok_or_else(|| S3ApiError::no_such_key(uri.path(), &request_id.0.0))?;
    let visibility = media.effective_visibility(bucket.policy().visibility());
    let mut response = read_media_bytes(
        &state,
        &media,
        visibility,
        method,
        ReadMediaQuery::default(),
        headers,
    )
    .await
    .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    response.headers_mut().insert(
        HeaderName::from_static("x-amz-request-id"),
        HeaderValue::from_str(&request_id.0.0)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid-request-id")),
    );
    Ok(response)
}

