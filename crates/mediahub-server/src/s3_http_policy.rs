// S3 Bucket Policy HTTP protocol, ownership fences, and response codecs.

const S3_EXPECTED_BUCKET_OWNER_HEADER: &str = "x-amz-expected-bucket-owner";

async fn s3_get_bucket_policy(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    authorize_s3_bucket_policy_permission(
        &auth,
        "bucket:list",
        uri.path(),
        &request_id.0.0,
    )?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    let snapshot = state
        .repository
        .get_s3_bucket_policy(&bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 GetBucketPolicy lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    validate_s3_bucket_policy_owner(
        &auth,
        &snapshot.identity,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;
    let document = snapshot
        .policy
        .as_ref()
        .ok_or_else(|| S3ApiError::no_such_bucket_policy(uri.path(), &request_id.0.0))?;
    let policy = parse_persisted_s3_bucket_policy(document, &bucket_name).map_err(|error| {
        warn!(error = %error, "persisted S3 Bucket Policy is invalid");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    Ok(s3_policy_json_response(
        policy.to_stable_json(),
        &request_id.0.0,
    ))
}

async fn s3_get_bucket_policy_status(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    authorize_s3_bucket_policy_permission(
        &auth,
        "bucket:list",
        uri.path(),
        &request_id.0.0,
    )?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    let snapshot = state
        .repository
        .get_s3_bucket_policy(&bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 GetBucketPolicyStatus lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    validate_s3_bucket_policy_owner(
        &auth,
        &snapshot.identity,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;

    let is_public = snapshot.policy.as_ref().map_or(Ok(false), |document| {
        parse_persisted_s3_bucket_policy(document, &bucket_name).map(|policy| {
            policy
                .policy_status(snapshot.identity.owner_account_id.as_str())
                .is_public()
        })
    });
    let is_public = is_public.map_err(|error| {
        warn!(error = %error, "persisted S3 Bucket Policy is invalid");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    Ok(s3_xml_response(
        StatusCode::OK,
        s3_bucket_policy_status_xml(is_public),
        &request_id.0.0,
    ))
}

async fn s3_put_bucket_policy(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    content: Body,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let (auth, signature) = authenticate_s3_streaming_application(
        &state,
        &method,
        &uri,
        &headers,
        &request_id.0.0,
    )
    .await?;
    authorize_s3_bucket_policy_permission(
        &auth,
        "bucket:manage",
        uri.path(),
        &request_id.0.0,
    )?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    let identity = state
        .repository
        .resolve_s3_bucket_identity(&bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 PutBucketPolicy owner lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    validate_s3_bucket_policy_owner(
        &auth,
        &identity,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;

    let declared_length = s3_content_length(&headers, uri.path(), &request_id.0.0)?;
    if declared_length > MAX_S3_POLICY_BYTES as u64 {
        return Err(S3ApiError::policy_too_large(
            uri.path(),
            &request_id.0.0,
        ));
    }
    let content = to_bytes(content, MAX_S3_POLICY_BYTES + 1)
        .await
        .map_err(|_| S3ApiError::policy_too_large(uri.path(), &request_id.0.0))?;
    validate_s3_policy_body_length(
        declared_length,
        content.len(),
        uri.path(),
        &request_id.0.0,
    )?;
    let payload_sha256 = hex::encode(Sha256::digest(&content));
    signature
        .verify_payload_sha256(&payload_sha256)
        .map_err(|error| S3ApiError::from_sigv4(error, uri.path(), &request_id.0.0))?;
    validate_s3_optional_policy_content_md5(&headers, &content, uri.path(), &request_id.0.0)?;

    let policy = S3BucketPolicy::parse(&content, &bucket_name)
        .map_err(|_| S3ApiError::malformed_policy(uri.path(), &request_id.0.0))?;
    let stable_json = policy.to_stable_json();
    let stable_value = serde_json::from_str(&stable_json).map_err(|error| {
        warn!(error = %error, "stable S3 Bucket Policy JSON failed to parse");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    let document = S3BucketPolicyDocument::new(stable_value).map_err(|error| {
        warn!(error = %error, "stable S3 Bucket Policy document failed validation");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    let snapshot = state
        .repository
        .put_s3_bucket_policy(
            auth.application.id,
            &bucket_name,
            document,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 PutBucketPolicy persistence failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;

    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "bucket.policy_updated",
        "bucket",
        snapshot.identity.bucket_id.to_string(),
        serde_json::json!({
            "name": bucket_name,
            "revision": snapshot.revision,
            "protocol": "s3",
        }),
    )
    .await;
    Ok(s3_empty_response(StatusCode::OK, &request_id.0.0))
}

async fn s3_delete_bucket_policy(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    authorize_s3_bucket_policy_permission(
        &auth,
        "bucket:manage",
        uri.path(),
        &request_id.0.0,
    )?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    let identity = state
        .repository
        .resolve_s3_bucket_identity(&bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 DeleteBucketPolicy owner lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    validate_s3_bucket_policy_owner(
        &auth,
        &identity,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;
    let snapshot = state
        .repository
        .delete_s3_bucket_policy(
            auth.application.id,
            &bucket_name,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 DeleteBucketPolicy persistence failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;

    record_audit(
        &state,
        &auth,
        &request_id.0.0,
        "bucket.policy_deleted",
        "bucket",
        snapshot.identity.bucket_id.to_string(),
        serde_json::json!({
            "name": bucket_name,
            "revision": snapshot.revision,
            "protocol": "s3",
        }),
    )
    .await;
    Ok(s3_empty_response(StatusCode::NO_CONTENT, &request_id.0.0))
}

fn authorize_s3_bucket_policy_permission(
    auth: &ApplicationAuth,
    permission: &str,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    auth.authorize(permission)
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))
}

fn parse_s3_expected_bucket_owner(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<Option<String>, S3ApiError> {
    let values = headers
        .get_all(S3_EXPECTED_BUCKET_OWNER_HEADER)
        .iter()
        .collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [value] => {
            let value = value.to_str().ok().filter(|value| {
                value.len() == mediahub_app::S3_ACCOUNT_ID_DIGITS
                    && value.bytes().all(|byte| byte.is_ascii_digit())
            });
            value.map(|value| Some(value.to_owned())).ok_or_else(|| {
                S3ApiError::invalid_argument(
                    "x-amz-expected-bucket-owner must contain exactly 12 decimal digits.",
                    resource,
                    request_id,
                )
            })
        }
        _ => Err(S3ApiError::invalid_argument(
            "x-amz-expected-bucket-owner must occur at most once.",
            resource,
            request_id,
        )),
    }
}

fn validate_s3_bucket_policy_owner(
    auth: &ApplicationAuth,
    identity: &S3BucketIdentity,
    expected_owner: Option<&str>,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    if expected_owner.is_some_and(|expected| expected != identity.owner_account_id.as_str()) {
        return Err(S3ApiError::access_denied(
            "The bucket is owned by a different account than the expected bucket owner.",
            resource,
            request_id,
        ));
    }
    if identity.application_id != auth.application.id {
        return Err(S3ApiError::method_not_allowed(
            "The specified method is not allowed against this resource.",
            resource,
            request_id,
        ));
    }
    Ok(())
}

fn validate_s3_policy_body_length(
    declared_length: u64,
    actual_length: usize,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    let actual_length = u64::try_from(actual_length)
        .map_err(|_| S3ApiError::policy_too_large(resource, request_id))?;
    if actual_length < declared_length {
        Err(S3ApiError::incomplete_body(resource, request_id))
    } else if actual_length > declared_length {
        Err(S3ApiError::invalid_request(
            "The request body exceeds Content-Length.",
            resource,
            request_id,
        ))
    } else {
        Ok(())
    }
}

fn validate_s3_optional_policy_content_md5(
    headers: &HeaderMap,
    content: &[u8],
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    let values = headers.get_all("content-md5").iter().collect::<Vec<_>>();
    let Some(value) = values.first() else {
        return Ok(());
    };
    if values.len() != 1 {
        return Err(S3ApiError::invalid_digest(resource, request_id));
    }
    validate_content_md5(Some(value.as_bytes()), content).map_err(|error| {
        S3ApiError::new(
            StatusCode::BAD_REQUEST,
            error.s3_code(),
            error.to_string(),
            resource,
            request_id,
        )
    })
}

fn parse_persisted_s3_bucket_policy(
    document: &S3BucketPolicyDocument,
    bucket_name: &str,
) -> Result<S3BucketPolicy, mediahub_core::S3PolicyError> {
    let encoded = serde_json::to_vec(document.document())
        .expect("a validated JSON value always serializes");
    S3BucketPolicy::parse(&encoded, bucket_name)
}

fn s3_policy_json_response(body: String, request_id: &str) -> Response {
    let mut response = (StatusCode::OK, body).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    insert_s3_request_id(&mut response, request_id);
    response
}

fn s3_bucket_policy_status_xml(is_public: bool) -> String {
    let is_public = if is_public { "true" } else { "false" };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><PolicyStatus xmlns=\"{S3_XML_NAMESPACE}\"><IsPublic>{is_public}</IsPublic></PolicyStatus>"
    )
}
