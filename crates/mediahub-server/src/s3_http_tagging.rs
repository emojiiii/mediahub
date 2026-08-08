// Object-version-scoped S3 Object Tagging protocol handling.

fn classify_s3_object_tagging(uri: &Uri, request_id: &str) -> Result<bool, S3ApiError> {
    let mut tagging_seen = false;
    let mut version_seen = false;
    for (name, value) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
        match name.as_ref() {
            "tagging" if !tagging_seen && value.is_empty() => tagging_seen = true,
            "tagging" => {
                return Err(S3ApiError::invalid_argument(
                    "The tagging subresource must occur once with an empty value.",
                    uri.path(),
                    request_id,
                ));
            }
            "versionId" if !version_seen => version_seen = true,
            "versionId" => {
                return Err(S3ApiError::invalid_argument(
                    "Query parameter versionId must not occur more than once.",
                    uri.path(),
                    request_id,
                ));
            }
            name if name.starts_with("X-Amz-") || name == "x-id" => {}
            _ if !tagging_seen => {}
            _ => {
                return Err(S3ApiError::invalid_argument(
                    "Object Tagging cannot be combined with another S3 subresource.",
                    uri.path(),
                    request_id,
                ));
            }
        }
    }
    if tagging_seen {
        // Re-scan so parameters appearing before `tagging` are also rejected.
        for (name, _) in url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()) {
            if name != "tagging"
                && name != "versionId"
                && !name.starts_with("X-Amz-")
                && name != "x-id"
            {
                return Err(S3ApiError::invalid_argument(
                    "Object Tagging cannot be combined with another S3 subresource.",
                    uri.path(),
                    request_id,
                ));
            }
        }
    }
    Ok(tagging_seen)
}

async fn s3_get_object_tagging(
    operation: S3ObjectOperation<'_>,
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
    let service = runtime_s3_object_service(state, resource, request_id)?;
    let receipt = service
        .get_object_tags(&S3ObjectRequest {
            application_id: auth.application.id,
            bucket_name: bucket_name.to_owned(),
            object_key: object_key.to_owned(),
            version_id: parse_s3_version_id(uri, request_id)?,
        })
        .await
        .map_err(|error| map_s3_object_service_error(error, resource, request_id))?;
    let body = object_tagging_xml(&receipt.tags)
        .map_err(|error| S3ApiError::from_xml(error, resource, request_id))?;
    let mut response = s3_xml_response(StatusCode::OK, body, request_id);
    insert_s3_version_id(
        response.headers_mut(),
        receipt.version.external_version_id(),
    )?;
    Ok(response)
}

async fn s3_put_object_tagging(
    operation: S3ObjectOperation<'_>,
    headers: &HeaderMap,
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
    let resource = uri.path();
    auth.authorize("media:upload")
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    validate_s3_object_key(object_key, resource, request_id)?;
    validate_content_md5(headers.get("content-md5").map(HeaderValue::as_bytes), content)
        .map_err(|error| {
            S3ApiError::new(
                StatusCode::BAD_REQUEST,
                error.s3_code(),
                error.to_string(),
                resource,
                request_id,
            )
        })?;
    let tags = parse_object_tagging_xml(content)
        .map_err(|error| map_s3_tagging_xml_error(error, resource, request_id))?;
    let service = runtime_s3_object_service(state, resource, request_id)?;
    let receipt = service
        .put_object_tags(&PutObjectTaggingRequest {
            object: S3ObjectRequest {
                application_id: auth.application.id,
                bucket_name: bucket_name.to_owned(),
                object_key: object_key.to_owned(),
                version_id: parse_s3_version_id(uri, request_id)?,
            },
            tags,
        })
        .await
        .map_err(|error| map_s3_object_service_error(error, resource, request_id))?;
    let mut response = s3_empty_response(StatusCode::OK, request_id);
    insert_s3_version_id(
        response.headers_mut(),
        receipt.version.external_version_id(),
    )?;
    Ok(response)
}

async fn s3_delete_object_tagging(
    operation: S3ObjectOperation<'_>,
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
    auth.authorize("media:delete")
        .map_err(|error| S3ApiError::from_api(error, resource, request_id))?;
    validate_s3_object_key(object_key, resource, request_id)?;
    let service = runtime_s3_object_service(state, resource, request_id)?;
    let receipt = service
        .delete_object_tags(&S3ObjectRequest {
            application_id: auth.application.id,
            bucket_name: bucket_name.to_owned(),
            object_key: object_key.to_owned(),
            version_id: parse_s3_version_id(uri, request_id)?,
        })
        .await
        .map_err(|error| map_s3_object_service_error(error, resource, request_id))?;
    let mut response = s3_empty_response(StatusCode::NO_CONTENT, request_id);
    insert_s3_version_id(
        response.headers_mut(),
        receipt.version.external_version_id(),
    )?;
    Ok(response)
}

fn map_s3_tagging_xml_error(
    error: S3TaggingXmlError,
    resource: &str,
    request_id: &str,
) -> S3ApiError {
    match error {
        S3TaggingXmlError::InputTooLarge => S3ApiError::entity_too_large(resource, request_id),
        S3TaggingXmlError::MalformedXml => S3ApiError::malformed_xml(resource, request_id),
        S3TaggingXmlError::InvalidTag(error) => {
            S3ApiError::invalid_tag(error.to_string(), resource, request_id)
        }
    }
}

fn parse_s3_tagging_header(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<Option<S3ObjectTagSet>, S3ApiError> {
    let Some(value) = optional_single_s3_header(headers, "x-amz-tagging", resource, request_id)?
    else {
        return Ok(None);
    };
    if value.is_empty() {
        return Ok(Some(S3ObjectTagSet::empty()));
    }
    let mut tags = Vec::new();
    for pair in value.split('&') {
        let (key, value) = pair.split_once('=').ok_or_else(|| {
            S3ApiError::invalid_tag(
                "x-amz-tagging must contain URL-encoded key=value pairs.",
                resource,
                request_id,
            )
        })?;
        if value.contains('=') {
            return Err(S3ApiError::invalid_tag(
                "Reserved characters in x-amz-tagging must be percent-encoded.",
                resource,
                request_id,
            ));
        }
        let key = strict_form_component_decode(key, resource, request_id)?;
        let value = strict_form_component_decode(value, resource, request_id)?;
        tags.push(
            S3ObjectTag::new(key, value).map_err(|error| {
                S3ApiError::invalid_tag(error.to_string(), resource, request_id)
            })?,
        );
    }
    S3ObjectTagSet::new(tags)
        .map(Some)
        .map_err(|error| S3ApiError::invalid_tag(error.to_string(), resource, request_id))
}

fn strict_form_component_decode(
    value: &str,
    resource: &str,
    request_id: &str,
) -> Result<String, S3ApiError> {
    let input = value.as_bytes();
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            b'%' if index + 2 < input.len() => {
                let high = hex_nibble(input[index + 1]);
                let low = hex_nibble(input[index + 2]);
                let (Some(high), Some(low)) = (high, low) else {
                    return Err(S3ApiError::invalid_tag(
                        "x-amz-tagging contains invalid percent encoding.",
                        resource,
                        request_id,
                    ));
                };
                output.push((high << 4) | low);
                index += 3;
            }
            b'%' => {
                return Err(S3ApiError::invalid_tag(
                    "x-amz-tagging contains truncated percent encoding.",
                    resource,
                    request_id,
                ));
            }
            b'+' => {
                output.push(b' ');
                index += 1;
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(output).map_err(|_| {
        S3ApiError::invalid_tag(
            "x-amz-tagging must decode to valid UTF-8.",
            resource,
            request_id,
        )
    })
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
