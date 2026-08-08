// S3 Bucket CORS and Bucket Tagging control-plane handlers.

async fn s3_get_bucket_cors(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let authorization = authorize_s3_data_request(
        &state,
        &method,
        &uri,
        &headers,
        S3DataAuthorizationInput {
            action: S3PolicyAction::GetBucketCors,
            bucket_name: &bucket_name,
            object_key: None,
            version_id: None,
            prefix: None,
            delimiter: None,
            max_keys: None,
            secure_transport: s3_data_secure_transport(&uri),
            source_ip: s3_data_source_ip(connect_info.as_ref()),
        },
        &request_id.0.0,
    )
    .await?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    validate_s3_expected_bucket_owner(
        &authorization.bucket,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;
    let snapshot = mediahub_app::S3BucketCorsRepository::get_s3_bucket_cors(
        &state.repository,
        authorization.application_id(),
        &bucket_name,
    )
    .await
    .map_err(|error| {
        warn!(error = %error, "S3 GetBucketCors lookup failed");
        S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
    })?
    .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let configuration = snapshot.configuration.as_ref().ok_or_else(|| {
        S3ApiError::new(
            StatusCode::NOT_FOUND,
            "NoSuchCORSConfiguration",
            "The CORS configuration does not exist.",
            uri.path(),
            &request_id.0.0,
        )
    })?;
    let body = s3_cors_protocol::render_s3_cors_configuration_xml(configuration).map_err(|error| {
        warn!(error = %error, "persisted S3 Bucket CORS configuration is invalid");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    Ok(s3_xml_response(StatusCode::OK, body, &request_id.0.0))
}

async fn s3_put_bucket_cors(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    request_context: (
        HeaderMap,
        Option<Extension<ConnectInfo<SocketAddr>>>,
        Extension<RequestId>,
    ),
    content: Bytes,
) -> Result<Response, S3ApiError> {
    let (headers, connect_info, request_id) = request_context;
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &content, &request_id.0.0)
            .await?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    let authorization = authorize_s3_signed_bucket_configuration(
        &state,
        &auth,
        S3BucketConfigurationAuthorization {
            action: S3PolicyAction::PutBucketCors,
            bucket_name: &bucket_name,
            uri: &uri,
            connect_info: connect_info.as_ref(),
            request_id: &request_id.0.0,
        },
    )
    .await?;
    validate_s3_expected_bucket_owner(
        &authorization.bucket,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;
    validate_s3_sdk_checksum_pairing(&headers, uri.path(), &request_id.0.0)?;
    let configuration = s3_cors_protocol::parse_s3_cors_put_xml(
        single_s3_content_md5(&headers, uri.path(), &request_id.0.0)?,
        &content,
    )
    .map_err(|error| map_s3_cors_protocol_error(error, uri.path(), &request_id.0.0))?;
    let snapshot = mediahub_app::S3BucketCorsRepository::put_s3_bucket_cors(
        &state.repository,
        authorization.application_id(),
        &bucket_name,
        configuration,
        OffsetDateTime::now_utc(),
    )
    .await
    .map_err(|error| {
        warn!(error = %error, "S3 PutBucketCors persistence failed");
        S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
    })?
    .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    record_s3_resource_audit(
        &state,
        &auth,
        authorization.application_id(),
        &request_id.0.0,
        "s3.bucket.cors_updated",
        ("bucket", authorization.bucket.bucket_id.to_string()),
        serde_json::json!({ "name": bucket_name, "revision": snapshot.revision }),
    )
    .await;
    Ok(s3_empty_response(StatusCode::OK, &request_id.0.0))
}

async fn s3_delete_bucket_cors(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    let authorization = authorize_s3_signed_bucket_configuration(
        &state,
        &auth,
        S3BucketConfigurationAuthorization {
            action: S3PolicyAction::PutBucketCors,
            bucket_name: &bucket_name,
            uri: &uri,
            connect_info: connect_info.as_ref(),
            request_id: &request_id.0.0,
        },
    )
    .await?;
    validate_s3_expected_bucket_owner(
        &authorization.bucket,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;
    let snapshot = mediahub_app::S3BucketCorsRepository::delete_s3_bucket_cors(
        &state.repository,
        authorization.application_id(),
        &bucket_name,
        OffsetDateTime::now_utc(),
    )
    .await
    .map_err(|error| {
        warn!(error = %error, "S3 DeleteBucketCors persistence failed");
        S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
    })?
    .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    record_s3_resource_audit(
        &state,
        &auth,
        authorization.application_id(),
        &request_id.0.0,
        "s3.bucket.cors_deleted",
        ("bucket", authorization.bucket.bucket_id.to_string()),
        serde_json::json!({ "name": bucket_name, "revision": snapshot.revision }),
    )
    .await;
    Ok(s3_empty_response(StatusCode::NO_CONTENT, &request_id.0.0))
}

async fn s3_get_bucket_tagging(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let authorization = authorize_s3_data_request(
        &state,
        &method,
        &uri,
        &headers,
        S3DataAuthorizationInput {
            action: S3PolicyAction::GetBucketTagging,
            bucket_name: &bucket_name,
            object_key: None,
            version_id: None,
            prefix: None,
            delimiter: None,
            max_keys: None,
            secure_transport: s3_data_secure_transport(&uri),
            source_ip: s3_data_source_ip(connect_info.as_ref()),
        },
        &request_id.0.0,
    )
    .await?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    validate_s3_expected_bucket_owner(
        &authorization.bucket,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;
    let snapshot = mediahub_app::S3BucketTaggingRepository::get_s3_bucket_tagging(
        &state.repository,
        authorization.application_id(),
        &bucket_name,
    )
    .await
    .map_err(|error| {
        warn!(error = %error, "S3 GetBucketTagging lookup failed");
        S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
    })?
    .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let tags = snapshot
        .tags
        .as_ref()
        .ok_or_else(|| s3_bucket_tag_set_not_found(uri.path(), &request_id.0.0))?;
    let body = render_s3_bucket_tagging_xml(tags).map_err(|error| {
        warn!(error = %error, "persisted S3 Bucket Tagging document is invalid");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    Ok(s3_xml_response(StatusCode::OK, body, &request_id.0.0))
}

async fn s3_put_bucket_tagging(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    request_context: (
        HeaderMap,
        Option<Extension<ConnectInfo<SocketAddr>>>,
        Extension<RequestId>,
    ),
    content: Bytes,
) -> Result<Response, S3ApiError> {
    let (headers, connect_info, request_id) = request_context;
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &content, &request_id.0.0)
            .await?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    let authorization = authorize_s3_signed_bucket_configuration(
        &state,
        &auth,
        S3BucketConfigurationAuthorization {
            action: S3PolicyAction::PutBucketTagging,
            bucket_name: &bucket_name,
            uri: &uri,
            connect_info: connect_info.as_ref(),
            request_id: &request_id.0.0,
        },
    )
    .await?;
    validate_s3_expected_bucket_owner(
        &authorization.bucket,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;
    validate_s3_sdk_checksum_pairing(&headers, uri.path(), &request_id.0.0)?;
    let tags = parse_s3_bucket_tagging_put(
        &headers,
        &content,
        uri.path(),
        &request_id.0.0,
    )?;
    let snapshot = mediahub_app::S3BucketTaggingRepository::put_s3_bucket_tagging(
        &state.repository,
        authorization.application_id(),
        &bucket_name,
        tags,
        OffsetDateTime::now_utc(),
    )
    .await
    .map_err(|error| {
        warn!(error = %error, "S3 PutBucketTagging persistence failed");
        S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
    })?
    .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    record_s3_resource_audit(
        &state,
        &auth,
        authorization.application_id(),
        &request_id.0.0,
        "s3.bucket.tagging_updated",
        ("bucket", authorization.bucket.bucket_id.to_string()),
        serde_json::json!({ "name": bucket_name, "revision": snapshot.revision }),
    )
    .await;
    Ok(s3_empty_response(StatusCode::OK, &request_id.0.0))
}

async fn s3_delete_bucket_tagging(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &[], &request_id.0.0).await?;
    let expected_owner =
        parse_s3_expected_bucket_owner(&headers, uri.path(), &request_id.0.0)?;
    let authorization = authorize_s3_signed_bucket_configuration(
        &state,
        &auth,
        S3BucketConfigurationAuthorization {
            action: S3PolicyAction::PutBucketTagging,
            bucket_name: &bucket_name,
            uri: &uri,
            connect_info: connect_info.as_ref(),
            request_id: &request_id.0.0,
        },
    )
    .await?;
    validate_s3_expected_bucket_owner(
        &authorization.bucket,
        expected_owner.as_deref(),
        uri.path(),
        &request_id.0.0,
    )?;
    let snapshot = mediahub_app::S3BucketTaggingRepository::delete_s3_bucket_tagging(
        &state.repository,
        authorization.application_id(),
        &bucket_name,
        OffsetDateTime::now_utc(),
    )
    .await
    .map_err(|error| {
        warn!(error = %error, "S3 DeleteBucketTagging persistence failed");
        S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
    })?
    .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    record_s3_resource_audit(
        &state,
        &auth,
        authorization.application_id(),
        &request_id.0.0,
        "s3.bucket.tagging_deleted",
        ("bucket", authorization.bucket.bucket_id.to_string()),
        serde_json::json!({ "name": bucket_name, "revision": snapshot.revision }),
    )
    .await;
    Ok(s3_empty_response(StatusCode::NO_CONTENT, &request_id.0.0))
}

fn validate_s3_expected_bucket_owner(
    bucket: &S3BucketIdentity,
    expected_owner: Option<&str>,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    if expected_owner.is_some_and(|owner| owner != bucket.owner_account_id.as_str()) {
        Err(S3ApiError::access_denied(
            "The bucket is owned by a different account than the expected bucket owner.",
            resource,
            request_id,
        ))
    } else {
        Ok(())
    }
}

fn map_s3_cors_protocol_error(
    error: s3_cors_protocol::S3CorsProtocolError,
    resource: &str,
    request_id: &str,
) -> S3ApiError {
    use s3_cors_protocol::S3CorsProtocolError;

    match error {
        S3CorsProtocolError::InputTooLarge => S3ApiError::entity_too_large(resource, request_id),
        S3CorsProtocolError::MissingContentMd5 | S3CorsProtocolError::InvalidDigest => {
            S3ApiError::invalid_digest(resource, request_id)
        }
        S3CorsProtocolError::BadDigest => S3ApiError::bad_digest(resource, request_id),
        S3CorsProtocolError::MalformedXml | S3CorsProtocolError::InvalidConfiguration(_) => {
            S3ApiError::malformed_xml(resource, request_id)
        }
    }
}

fn validate_s3_sdk_checksum_pairing(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    let declarations = headers
        .get_all("x-amz-sdk-checksum-algorithm")
        .iter()
        .collect::<Vec<_>>();
    let Some(declaration) = declarations.first() else {
        return Ok(());
    };
    if declarations.len() != 1 {
        return Err(S3ApiError::invalid_argument(
            "x-amz-sdk-checksum-algorithm must occur exactly once.",
            resource,
            request_id,
        ));
    }
    let (checksum_header, decoded_bytes) = match declaration.to_str().ok() {
        Some("CRC32") => ("x-amz-checksum-crc32", 4),
        Some("CRC32C") => ("x-amz-checksum-crc32c", 4),
        Some("CRC64NVME") => ("x-amz-checksum-crc64nvme", 8),
        Some("SHA1") => ("x-amz-checksum-sha1", 20),
        Some("SHA256") => ("x-amz-checksum-sha256", 32),
        _ => {
            return Err(S3ApiError::invalid_argument(
                "x-amz-sdk-checksum-algorithm is invalid.",
                resource,
                request_id,
            ));
        }
    };
    let checksums = headers.get_all(checksum_header).iter().collect::<Vec<_>>();
    let paired = match checksums.as_slice() {
        [checksum] => STANDARD
            .decode(checksum.as_bytes())
            .is_ok_and(|value| value.len() == decoded_bytes),
        _ => false,
    };
    if !paired {
        return Err(S3ApiError::invalid_request(
            format!(
                "x-amz-sdk-checksum-algorithm requires exactly one valid {checksum_header} header."
            ),
            resource,
            request_id,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod bucket_metadata_checksum_tests {
    use super::*;

    #[test]
    fn sdk_checksum_declaration_requires_one_matching_well_formed_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-sdk-checksum-algorithm",
            HeaderValue::from_static("SHA256"),
        );
        assert_eq!(
            validate_s3_sdk_checksum_pairing(&headers, "/assets", "request-id")
                .expect_err("missing paired checksum")
                .code,
            "InvalidRequest",
        );

        headers.insert(
            "x-amz-checksum-sha256",
            HeaderValue::from_str(&STANDARD.encode([0_u8; 32])).expect("checksum header"),
        );
        validate_s3_sdk_checksum_pairing(&headers, "/assets", "request-id")
            .expect("matching SHA256 checksum header");

        headers.append(
            "x-amz-checksum-sha256",
            HeaderValue::from_str(&STANDARD.encode([1_u8; 32])).expect("checksum header"),
        );
        assert_eq!(
            validate_s3_sdk_checksum_pairing(&headers, "/assets", "request-id")
                .expect_err("duplicate paired checksum")
                .code,
            "InvalidRequest",
        );
    }

    #[test]
    fn sdk_checksum_declaration_is_optional_but_strict_when_present() {
        validate_s3_sdk_checksum_pairing(&HeaderMap::new(), "/assets", "request-id")
            .expect("optional declaration");

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-sdk-checksum-algorithm",
            HeaderValue::from_static("sha256"),
        );
        assert_eq!(
            validate_s3_sdk_checksum_pairing(&headers, "/assets", "request-id")
                .expect_err("algorithm names are exact")
                .code,
            "InvalidArgument",
        );
    }
}
