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
    let secret = std::str::from_utf8(&secret)
        .map_err(|_| {
            warn!("S3 access key secret is not valid UTF-8");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .to_owned();
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum S3BucketGetOperation {
    ListObjects,
    ListObjectVersions,
    ListMultipartUploads,
    GetLocation,
    GetVersioning,
    GetLifecycle,
    GetObjectLock,
}

fn classify_s3_bucket_get(uri: &Uri, request_id: &str) -> Result<S3BucketGetOperation, S3ApiError> {
    let location = s3_query_flag(uri, "location", request_id)?;
    let versioning = s3_query_flag(uri, "versioning", request_id)?;
    let lifecycle = s3_query_flag(uri, "lifecycle", request_id)?;
    let object_lock = s3_query_flag(uri, "object-lock", request_id)?;
    let versions = s3_query_flag(uri, "versions", request_id)?;
    let uploads = s3_query_flag(uri, "uploads", request_id)?;
    if usize::from(location)
        + usize::from(versioning)
        + usize::from(lifecycle)
        + usize::from(object_lock)
        + usize::from(versions)
        + usize::from(uploads)
        > 1
    {
        return Err(S3ApiError::invalid_request(
            "Bucket subresources cannot be combined.",
            uri.path(),
            request_id,
        ));
    }

    let operation = if location {
        S3BucketGetOperation::GetLocation
    } else if versioning {
        S3BucketGetOperation::GetVersioning
    } else if lifecycle {
        S3BucketGetOperation::GetLifecycle
    } else if object_lock {
        S3BucketGetOperation::GetObjectLock
    } else if versions {
        S3BucketGetOperation::ListObjectVersions
    } else if uploads {
        S3BucketGetOperation::ListMultipartUploads
    } else {
        S3BucketGetOperation::ListObjects
    };

    const ALL_LIST_QUERY_PARAMETERS: [&str; 13] = [
        "list-type",
        "prefix",
        "delimiter",
        "marker",
        "continuation-token",
        "start-after",
        "encoding-type",
        "max-keys",
        "key-marker",
        "version-id-marker",
        "max-uploads",
        "upload-id-marker",
        "uploads",
    ];

    match operation {
        S3BucketGetOperation::GetLocation
        | S3BucketGetOperation::GetVersioning
        | S3BucketGetOperation::GetLifecycle
        | S3BucketGetOperation::GetObjectLock => {
            for name in ALL_LIST_QUERY_PARAMETERS {
                if s3_query_value(uri, name, request_id)?.is_some() {
                    return Err(S3ApiError::invalid_request(
                        "Bucket configuration parameters cannot be combined with listing parameters.",
                        uri.path(),
                        request_id,
                    ));
                }
            }
        }
        S3BucketGetOperation::ListObjects => {
            reject_s3_listing_parameters(
                uri,
                request_id,
                &["key-marker", "version-id-marker", "max-uploads", "upload-id-marker"],
            )?;
        }
        S3BucketGetOperation::ListObjectVersions => {
            reject_s3_listing_parameters(
                uri,
                request_id,
                &[
                    "list-type",
                    "marker",
                    "continuation-token",
                    "start-after",
                    "max-uploads",
                    "upload-id-marker",
                ],
            )?;
        }
        S3BucketGetOperation::ListMultipartUploads => {
            reject_s3_listing_parameters(
                uri,
                request_id,
                &[
                    "list-type",
                    "marker",
                    "continuation-token",
                    "start-after",
                    "max-keys",
                    "version-id-marker",
                ],
            )?;
        }
    }
    Ok(operation)
}

fn reject_s3_listing_parameters(
    uri: &Uri,
    request_id: &str,
    names: &[&'static str],
) -> Result<(), S3ApiError> {
    for &name in names {
        if s3_query_value(uri, name, request_id)?.is_some() {
            return Err(S3ApiError::invalid_request(
                format!("Query parameter {name} is not valid for this bucket operation."),
                uri.path(),
                request_id,
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ListObjectVersionsQuery {
    prefix: String,
    delimiter: Option<String>,
    key_marker: Option<String>,
    version_id_marker: Option<S3VersionId>,
    max_keys: usize,
    encoding_url: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ListMultipartUploadsQuery {
    prefix: String,
    delimiter: Option<String>,
    key_marker: Option<String>,
    upload_id_marker: Option<String>,
    max_uploads: usize,
    encoding_url: bool,
}

fn parse_list_object_versions_query(
    uri: &Uri,
    request_id: &str,
) -> Result<ListObjectVersionsQuery, S3ApiError> {
    let prefix = s3_query_value(uri, "prefix", request_id)?.unwrap_or_default();
    validate_s3_listing_key_like(&prefix, "prefix", uri, request_id)?;
    let key_marker = s3_query_value(uri, "key-marker", request_id)?;
    if let Some(marker) = key_marker.as_deref() {
        validate_s3_listing_key_like(marker, "key-marker", uri, request_id)?;
    }
    let version_id_marker = s3_query_value(uri, "version-id-marker", request_id)?
        .map(S3VersionId::new)
        .transpose()
        .map_err(|_| {
            S3ApiError::invalid_argument(
                "version-id-marker is invalid.",
                uri.path(),
                request_id,
            )
        })?;
    if version_id_marker.is_some() && key_marker.is_none() {
        return Err(S3ApiError::invalid_argument(
            "version-id-marker cannot be specified without key-marker.",
            uri.path(),
            request_id,
        ));
    }
    Ok(ListObjectVersionsQuery {
        prefix,
        delimiter: parse_s3_listing_delimiter(uri, request_id)?,
        key_marker,
        version_id_marker,
        max_keys: parse_s3_listing_limit(uri, "max-keys", request_id)?,
        encoding_url: parse_s3_listing_encoding(uri, request_id)?,
    })
}

fn parse_list_multipart_uploads_query(
    uri: &Uri,
    request_id: &str,
) -> Result<ListMultipartUploadsQuery, S3ApiError> {
    let prefix = s3_query_value(uri, "prefix", request_id)?.unwrap_or_default();
    validate_s3_listing_key_like(&prefix, "prefix", uri, request_id)?;
    let key_marker = s3_query_value(uri, "key-marker", request_id)?;
    if let Some(marker) = key_marker.as_deref() {
        validate_s3_listing_key_like(marker, "key-marker", uri, request_id)?;
    }
    let upload_id_marker = s3_query_value(uri, "upload-id-marker", request_id)?;
    if upload_id_marker.as_ref().is_some_and(String::is_empty) {
        return Err(S3ApiError::invalid_argument(
            "upload-id-marker must not be empty.",
            uri.path(),
            request_id,
        ));
    }
    Ok(ListMultipartUploadsQuery {
        prefix,
        delimiter: parse_s3_listing_delimiter(uri, request_id)?,
        key_marker,
        upload_id_marker,
        max_uploads: parse_s3_listing_limit(uri, "max-uploads", request_id)?,
        encoding_url: parse_s3_listing_encoding(uri, request_id)?,
    })
}

fn parse_s3_listing_limit(
    uri: &Uri,
    name: &'static str,
    request_id: &str,
) -> Result<usize, S3ApiError> {
    let Some(value) = s3_query_value(uri, name, request_id)? else {
        return Ok(1_000);
    };
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(S3ApiError::invalid_argument(
            format!("{name} must be an integer between 0 and 1000."),
            uri.path(),
            request_id,
        ));
    }
    value
        .parse::<usize>()
        .ok()
        .filter(|value| *value <= 1_000)
        .ok_or_else(|| {
            S3ApiError::invalid_argument(
                format!("{name} must be an integer between 0 and 1000."),
                uri.path(),
                request_id,
            )
        })
}

fn parse_s3_listing_delimiter(
    uri: &Uri,
    request_id: &str,
) -> Result<Option<String>, S3ApiError> {
    match s3_query_value(uri, "delimiter", request_id)? {
        None => Ok(None),
        Some(value) if value.is_empty() => Ok(None),
        Some(value) if value == "/" => Ok(Some(value)),
        Some(_) => Err(S3ApiError::invalid_argument(
            "Only delimiter=/ is supported.",
            uri.path(),
            request_id,
        )),
    }
}

fn parse_s3_listing_encoding(uri: &Uri, request_id: &str) -> Result<bool, S3ApiError> {
    match s3_query_value(uri, "encoding-type", request_id)?.as_deref() {
        None => Ok(false),
        Some("url") => Ok(true),
        Some(_) => Err(S3ApiError::invalid_argument(
            "encoding-type must be url.",
            uri.path(),
            request_id,
        )),
    }
}

fn validate_s3_listing_key_like(
    value: &str,
    name: &str,
    uri: &Uri,
    request_id: &str,
) -> Result<(), S3ApiError> {
    if value.len() <= 1_024 && !value.as_bytes().contains(&0) {
        Ok(())
    } else {
        Err(S3ApiError::invalid_argument(
            format!("{name} is invalid."),
            uri.path(),
            request_id,
        ))
    }
}
pub(super) async fn s3_list_buckets(
    State(state): State<Arc<AppState>>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    auth.authorize("bucket:list")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let buckets = state
        .repository
        .list_buckets(auth.application.id)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 ListBuckets lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?;
    let body = s3_list_buckets_xml(&auth.application.app_id, &auth.application.name, &buckets)
        .map_err(|error| {
            warn!(error = %error, "S3 ListBuckets XML encoding failed");
            S3ApiError::internal_error(uri.path(), &request_id.0.0)
        })?;
    Ok(s3_xml_response(StatusCode::OK, body, &request_id.0.0))
}

pub(super) async fn s3_create_bucket(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    content: Bytes,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|_| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &content, &request_id.0.0)
            .await?;
    auth.authorize("bucket:manage")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let object_lock_enabled =
        parse_s3_create_bucket_object_lock_header(&headers, &uri, &request_id.0.0)?;
    validate_s3_create_bucket_configuration(&content).map_err(|error| match error {
        S3CreateBucketConfigurationError::MalformedXml => {
            S3ApiError::malformed_xml(uri.path(), &request_id.0.0)
        }
        S3CreateBucketConfigurationError::InvalidLocationConstraint => {
            S3ApiError::invalid_location_constraint(uri.path(), &request_id.0.0)
        }
    })?;

    if state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 CreateBucket preflight lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .is_some()
    {
        return Err(S3ApiError::bucket_already_owned_by_you(
            uri.path(),
            &request_id.0.0,
        ));
    }

    let now = OffsetDateTime::now_utc();
    let bucket = S3Bucket::new(
        BucketId::new(),
        auth.application.id,
        bucket_name.clone(),
        DEFAULT_S3_REGION,
        object_lock_enabled,
        None,
        now,
    )
    .map_err(|_| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    match state.repository.create_s3_bucket(&bucket).await {
        Ok(()) => {}
        Err(mediahub_app::RepositoryError::Conflict) => {
            return Err(S3ApiError::bucket_already_owned_by_you(
                uri.path(),
                &request_id.0.0,
            ));
        }
        Err(error) => {
            warn!(error = %error, "S3 CreateBucket persistence failed");
            return Err(S3ApiError::service_unavailable(uri.path(), &request_id.0.0));
        }
    }

    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "bucket.created",
        "bucket",
        bucket.id().to_string(),
        serde_json::json!({
            "name": bucket.name(),
            "visibility": "private",
            "protocol": "s3",
            "region": DEFAULT_S3_REGION,
            "object_lock_enabled": object_lock_enabled,
        }),
    )
    .await;

    let mut response = s3_empty_response(StatusCode::OK, &request_id.0.0);
    response.headers_mut().insert(
        LOCATION,
        HeaderValue::from_str(&format!("/{bucket_name}"))
            .unwrap_or_else(|_| HeaderValue::from_static("/")),
    );
    Ok(response)
}

pub(super) async fn s3_list_objects(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|_| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;

    auth.authorize("media:list")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let bucket = state
        .repository
        .find_s3_bucket(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let query = ListObjectsV2Query::parse(uri.query())
        .map_err(|error| s3_list_api_error(error, uri.path(), &request_id.0.0))?;
    let codec = s3_list_token_codec(&state);
    let cursor = query
        .decode_continuation_cursor(&codec, &bucket_name)
        .map_err(|error| s3_list_api_error(error, uri.path(), &request_id.0.0))?;
    let page = state
        .repository
        .list_current_s3_objects(
            auth.application.id,
            &S3ObjectListQuery {
                bucket_id: bucket.id(),
                prefix: query.prefix.clone(),
                start_after: cursor.or_else(|| query.start_after.clone()),
                delimiter: query.delimiter.is_some(),
                limit: query.max_keys,
            },
        )
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 ObjectVersion listing failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?;
    let result = s3_list_result_from_object_page(
        auth.application.id,
        bucket.id(),
        bucket_name,
        query,
        page,
    );
    let body = result
        .to_xml(&codec)
        .map_err(|error| s3_list_api_error(error, uri.path(), &request_id.0.0))?;
    Ok(s3_xml_response(StatusCode::OK, body, &request_id.0.0))
}

fn s3_list_result_from_object_page(
    application_id: ApplicationId,
    bucket_id: BucketId,
    bucket: String,
    query: ListObjectsV2Query,
    page: S3ObjectPage,
) -> ListObjectsV2Result {
    let items = page
        .items
        .into_iter()
        .filter_map(|item| s3_list_object_from_version(application_id, bucket_id, item))
        .collect();
    ListObjectsV2Result {
        bucket,
        query,
        items,
        common_prefixes: page.common_prefixes,
        next_cursor: page.next_cursor,
    }
}

fn s3_list_object_from_version(
    application_id: ApplicationId,
    bucket_id: BucketId,
    item: S3ObjectListItem,
) -> Option<ListObject> {
    let version = item.version;
    if version.application_id() != application_id
        || version.bucket_id() != bucket_id
        || version.state() != mediahub_core::ObjectVersionState::Committed
        || version.became_noncurrent_at().is_some()
    {
        return None;
    }
    let ObjectVersionPayload::Object(payload) = version.payload() else {
        return None;
    };
    Some(ListObject {
        key: item.key,
        last_modified: version.created_at(),
        etag: payload.etag().as_str().to_owned(),
        size: payload.size_bytes(),
    })
}

fn s3_list_api_error(error: S3ListError, resource: &str, request_id: &str) -> S3ApiError {
    if error == S3ListError::InvalidContinuationToken {
        S3ApiError::new(
            StatusCode::BAD_REQUEST,
            "InvalidToken",
            error.to_string(),
            resource,
            request_id,
        )
    } else {
        S3ApiError::from_list(error, resource, request_id)
    }
}

pub(super) async fn s3_list_object_versions(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|_| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    auth.authorize("media:list")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let bucket = state
        .repository
        .find_s3_bucket(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 ListObjectVersions bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let query = parse_list_object_versions_query(&uri, &request_id.0.0)?;
    let page = state
        .repository
        .list_s3_object_versions_page(
            auth.application.id,
            &S3ObjectVersionListQuery {
                bucket_id: bucket.id(),
                prefix: query.prefix.clone(),
                key_marker: query.key_marker.clone(),
                version_id_marker: query.version_id_marker.clone(),
                delimiter: query.delimiter.is_some(),
                limit: query.max_keys,
            },
        )
        .await
        .map_err(|error| match error {
            RepositoryError::NotFound => S3ApiError::invalid_argument(
                "The version-id-marker does not identify a visible version of key-marker.",
                uri.path(),
                &request_id.0.0,
            ),
            error => {
                warn!(error = %error, "S3 ListObjectVersions lookup failed");
                S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
            }
        })?;
    let result = s3_object_versions_result_from_page(
        auth.application.id,
        bucket.id(),
        bucket_name,
        auth.application.app_id.clone(),
        auth.application.name.clone(),
        query,
        page,
    )
    .map_err(|error| {
        warn!(error = %error, "S3 ListObjectVersions repository invariant failed");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    let body = list_object_versions_result_xml(&result).map_err(|error| {
        warn!(error = %error, "S3 ListObjectVersions XML encoding failed");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    Ok(s3_xml_response(StatusCode::OK, body, &request_id.0.0))
}

fn s3_object_versions_result_from_page(
    application_id: ApplicationId,
    bucket_id: BucketId,
    bucket: String,
    owner_id: String,
    owner_display_name: String,
    query: ListObjectVersionsQuery,
    page: S3ObjectVersionPage,
) -> Result<ListObjectVersionsResult, RepositoryError> {
    let mut items = Vec::with_capacity(page.items.len());
    for item in page.items {
        let version = item.version;
        if version.application_id() != application_id
            || version.bucket_id() != bucket_id
            || version.state() != mediahub_core::ObjectVersionState::Committed
        {
            return Err(RepositoryError::Invariant(
                "ListObjectVersions returned a version outside the requested bucket".into(),
            ));
        }
        let kind = match version.payload() {
            ObjectVersionPayload::Object(payload) => ListedVersionKind::Object {
                etag: payload.etag().as_str().to_owned(),
                size: payload.size_bytes(),
            },
            ObjectVersionPayload::DeleteMarker => ListedVersionKind::DeleteMarker,
        };
        items.push(ListedObjectVersion {
            key: item.key,
            version_id: version.external_version_id().as_str().to_owned(),
            is_latest: item.is_latest,
            last_modified: version.created_at(),
            kind,
        });
    }
    Ok(ListObjectVersionsResult {
        bucket,
        owner_id,
        owner_display_name,
        prefix: query.prefix,
        delimiter: query.delimiter,
        key_marker: query.key_marker,
        version_id_marker: query
            .version_id_marker
            .map(|version_id| version_id.as_str().to_owned()),
        next_key_marker: page.next_key_marker,
        next_version_id_marker: page
            .next_version_id_marker
            .map(|version_id| version_id.as_str().to_owned()),
        max_keys: query.max_keys,
        encoding_url: query.encoding_url,
        items,
        common_prefixes: page.common_prefixes,
    })
}

pub(super) async fn s3_list_multipart_uploads(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|_| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    auth.authorize("media:list")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let bucket = state
        .repository
        .find_s3_bucket(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 ListMultipartUploads bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let query = parse_list_multipart_uploads_query(&uri, &request_id.0.0)?;
    let repository_upload_id_marker = if query.key_marker.is_some() {
        query.upload_id_marker.clone()
    } else {
        None
    };
    let page = state
        .repository
        .list_s3_multipart_uploads_page(
            auth.application.id,
            &S3MultipartUploadListQuery {
                bucket_id: bucket.id(),
                prefix: query.prefix.clone(),
                key_marker: query.key_marker.clone(),
                upload_id_marker: repository_upload_id_marker,
                delimiter: query.delimiter.is_some(),
                limit: query.max_uploads,
                as_of: OffsetDateTime::now_utc(),
            },
        )
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 ListMultipartUploads lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?;
    let result = s3_multipart_uploads_result_from_page(
        bucket_name,
        auth.application.app_id.clone(),
        auth.application.name.clone(),
        query,
        page,
    );
    let body = list_multipart_uploads_result_xml(&result).map_err(|error| {
        warn!(error = %error, "S3 ListMultipartUploads XML encoding failed");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    Ok(s3_xml_response(StatusCode::OK, body, &request_id.0.0))
}

fn s3_multipart_uploads_result_from_page(
    bucket: String,
    owner_id: String,
    owner_display_name: String,
    query: ListMultipartUploadsQuery,
    page: S3MultipartUploadPage,
) -> ListMultipartUploadsResult {
    ListMultipartUploadsResult {
        bucket,
        owner_id,
        owner_display_name,
        prefix: query.prefix,
        delimiter: query.delimiter,
        key_marker: query.key_marker,
        upload_id_marker: query.upload_id_marker,
        next_key_marker: page.next_key_marker,
        next_upload_id_marker: page.next_upload_id_marker,
        max_uploads: query.max_uploads,
        encoding_url: query.encoding_url,
        items: page
            .items
            .into_iter()
            .map(|item| ListedMultipartUpload {
                key: item.key,
                upload_id: item.upload_id,
                initiated_at: item.initiated_at,
            })
            .collect(),
        common_prefixes: page.common_prefixes,
    }
}
pub(super) async fn s3_get_bucket_location(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|_| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    auth.authorize("bucket:list")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    state
        .repository
        .get_s3_bucket_location(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 GetBucketLocation lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    Ok(s3_xml_response(
        StatusCode::OK,
        s3_bucket_location_xml(),
        &request_id.0.0,
    ))
}
pub(super) async fn s3_head_bucket(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|_| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    auth.authorize("bucket:list")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 HeadBucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    Ok(s3_bucket_region_response(StatusCode::OK, &request_id.0.0))
}

pub(super) async fn s3_delete_bucket(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|_| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    auth.authorize("bucket:manage")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 DeleteBucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;

    match state
        .repository
        .delete_empty_bucket(auth.application.id, &bucket_name)
        .await
    {
        Ok(true) => {}
        Ok(false) | Err(mediahub_app::RepositoryError::NotFound) => {
            return Err(S3ApiError::no_such_bucket(uri.path(), &request_id.0.0));
        }
        Err(mediahub_app::RepositoryError::Conflict) => {
            return Err(S3ApiError::bucket_not_empty(uri.path(), &request_id.0.0));
        }
        Err(error) => {
            warn!(error = %error, "S3 DeleteBucket persistence failed");
            return Err(S3ApiError::service_unavailable(uri.path(), &request_id.0.0));
        }
    }

    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "bucket.deleted",
        "bucket",
        bucket.id().to_string(),
        serde_json::json!({
            "name": bucket.name(),
            "protocol": "s3",
        }),
    )
    .await;
    Ok(s3_empty_response(StatusCode::NO_CONTENT, &request_id.0.0))
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
    let bypass_governance =
        parse_s3_bypass_governance(&headers, &uri, &request_id.0.0)?;
    state
        .repository
        .find_s3_bucket(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let request = parse_delete_objects_xml(&content)
        .map_err(|error| S3ApiError::from_xml(error, uri.path(), &request_id.0.0))?;
    let service = runtime_s3_object_service(&state, uri.path(), &request_id.0.0)?;
    let mut result = DeleteResult::default();
    for object in request.objects {
        let requested_version_id = object.version_id.clone();
        let version_id = match requested_version_id
            .as_deref()
            .map(S3VersionId::new)
            .transpose()
        {
            Ok(version_id) => version_id,
            Err(_) => {
                result.errors.push(DeleteObjectError {
                    key: object.key,
                    version_id: requested_version_id,
                    code: "InvalidArgument".to_owned(),
                    message: "The version ID is invalid.".to_owned(),
                });
                continue;
            }
        };
        let deletion = service
            .delete(&DeleteObjectRequest {
                application_id: auth.application.id,
                bucket_name: bucket_name.clone(),
                object_key: object.key.clone(),
                version_id,
                bypass_governance,
                deleted_by: auth.actor_id.clone(),
            })
            .await;
        record_s3_batch_delete_result(
            &mut result,
            object.key,
            requested_version_id,
            request.quiet,
            deletion,
        );
    }
    let body = delete_result_xml(&result)
        .map_err(|error| S3ApiError::from_xml(error, uri.path(), &request_id.0.0))?;
    Ok(s3_xml_response(StatusCode::OK, body, &request_id.0.0))
}

fn record_s3_batch_delete_result(
    result: &mut DeleteResult,
    key: String,
    requested_version_id: Option<String>,
    quiet: bool,
    deletion: Result<DeleteObjectReceipt, S3ObjectServiceError>,
) {
    match deletion {
        Ok(receipt) if !quiet => {
            let receipt_version_id = receipt
                .version_id
                .as_ref()
                .map(|version_id| version_id.as_str().to_owned());
            result.deleted.push(DeletedObject {
                key,
                version_id: (!receipt.delete_marker)
                    .then_some(requested_version_id)
                    .flatten(),
                delete_marker: receipt.delete_marker,
                delete_marker_version_id: receipt
                    .delete_marker
                    .then_some(receipt_version_id)
                    .flatten(),
            });
        }
        Ok(_) => {}
        Err(error) => {
            let (code, message) = s3_batch_delete_error(error);
            result.errors.push(DeleteObjectError {
                key,
                version_id: requested_version_id,
                code,
                message,
            });
        }
    }
}

fn s3_batch_delete_error(error: S3ObjectServiceError) -> (String, String) {
    let error = map_s3_object_service_error(error, "", "");
    (error.code.to_owned(), error.message)
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
    if let Some(lock_operation) =
        classify_s3_object_version_lock(&uri, &request_id.0.0)?
    {
        let content = to_bytes(content, MAX_S3_XML_BODY_BYTES)
            .await
            .map_err(|_| S3ApiError::entity_too_large(uri.path(), &request_id.0.0))?;
        let auth = authenticate_s3_application(
            &state,
            &method,
            &uri,
            &headers,
            &content,
            &request_id.0.0,
        )
        .await?;
        return s3_put_object_version_lock(
            S3ObjectOperation {
                state: &state,
                auth: &auth,
                bucket_name: &bucket_name,
                object_key: &object_key,
                uri: &uri,
                request_id: &request_id.0.0,
            },
            lock_operation,
            &headers,
            &content,
        )
        .await;
    }
    let (auth, signature) =
        authenticate_s3_streaming_application(&state, &method, &uri, &headers, &request_id.0.0)
            .await?;
    if s3_query_flag(&uri, "acl", &request_id.0.0)? {
        reject_s3_versioning(&uri, &request_id.0.0)?;
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
    let is_copy = headers.contains_key("x-amz-copy-source");
    if upload_id.is_some() || part_number.is_some() {
        reject_multipart_object_lock_headers(&headers, &uri, &request_id.0.0)?;
        reject_s3_versioning(&uri, &request_id.0.0)?;
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
        let operation = S3ObjectOperation {
            state: &state,
            auth: &auth,
            bucket_name: &bucket_name,
            object_key: &object_key,
            uri: &uri,
            request_id: &request_id.0.0,
        };
        return if is_copy {
            s3_upload_part_copy(
                operation,
                &upload_id,
                part_number,
                &signature,
                &headers,
                content,
            )
            .await
        } else {
            s3_upload_part(
                operation,
                &upload_id,
                part_number,
                signature,
                &headers,
                content,
            )
            .await
        };
    }
    if is_copy {
        reject_copy_object_lock_headers(&headers, &uri, &request_id.0.0)?;
        return s3_copy_object(
            S3ObjectOperation {
                state: &state,
                auth: &auth,
                bucket_name: &bucket_name,
                object_key: &object_key,
                uri: &uri,
                request_id: &request_id.0.0,
            },
            &signature,
            &headers,
            content,
        )
        .await;
    }
    s3_put_regular_object(
        S3ObjectOperation {
            state: &state,
            auth: &auth,
            bucket_name: &bucket_name,
            object_key: &object_key,
            uri: &uri,
            request_id: &request_id.0.0,
        },
        &signature,
        &headers,
        content,
    )
    .await
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
        StreamingUploadError::SizeMismatch { .. } => S3ApiError::invalid_argument(
            "The request body exceeds Content-Length.",
            resource,
            request_id,
        ),
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

pub(super) async fn s3_get_object(
    State(state): State<Arc<AppState>>,
    Path((bucket_name, object_key)): Path<(String, String)>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    let lock_operation = classify_s3_object_version_lock(&uri, &request_id.0.0)?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    if let Some(lock_operation) = lock_operation {
        if method != Method::GET {
            return Err(S3ApiError::method_not_allowed(
                "Object retention and legal hold subresources require GET.",
                uri.path(),
                &request_id.0.0,
            ));
        }
        return s3_get_object_version_lock(
            S3ObjectOperation {
                state: &state,
                auth: &auth,
                bucket_name: &bucket_name,
                object_key: &object_key,
                uri: &uri,
                request_id: &request_id.0.0,
            },
            lock_operation,
        )
        .await;
    }
    if s3_query_flag(&uri, "acl", &request_id.0.0)? {
        reject_s3_versioning(&uri, &request_id.0.0)?;
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
        reject_s3_versioning(&uri, &request_id.0.0)?;
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
    s3_read_regular_object(
        S3ObjectOperation {
            state: &state,
            auth: &auth,
            bucket_name: &bucket_name,
            object_key: &object_key,
            uri: &uri,
            request_id: &request_id.0.0,
        },
        method,
        headers,
    )
    .await
}
