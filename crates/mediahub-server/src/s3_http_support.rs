// S3 ACL, response, query, and error helpers.

fn s3_complete_multipart_response(
    location: &str,
    bucket_name: &str,
    object_key: &str,
    etag: &str,
    request_id: &str,
) -> Result<Response, S3ApiError> {
    let quoted_etag = if etag.starts_with('"') && etag.ends_with('"') {
        etag.to_owned()
    } else {
        format!("\"{etag}\"")
    };
    let body =
        complete_multipart_upload_result_xml(location, bucket_name, object_key, &quoted_etag)
            .map_err(|error| S3ApiError::from_xml(error, location, request_id))?;
    Ok(s3_xml_response(StatusCode::OK, body, request_id))
}

async fn s3_get_object_acl(
    state: &AppState,
    auth: &ApplicationAuth,
    bucket_name: &str,
    object_key: &str,
    uri: &Uri,
    request_id: &str,
) -> Result<Response, S3ApiError> {
    auth.authorize("media:read")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), request_id))?;
    let media = state
        .repository
        .find_by_object_key(auth.application.id, bucket.id(), object_key)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 object ACL lookup failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?
        .filter(|media| media.state().is_readable())
        .ok_or_else(|| S3ApiError::no_such_key(uri.path(), request_id))?;
    let acl = match media.effective_visibility(bucket.policy().visibility()) {
        Visibility::Private => ObjectAcl::Private,
        Visibility::Public => ObjectAcl::PublicRead,
    };
    let body = get_object_acl_xml(&auth.application.app_id, "MediaHub Application", acl)
        .map_err(|error| S3ApiError::from_xml(error, uri.path(), request_id))?;
    Ok(s3_xml_response(StatusCode::OK, body, request_id))
}

async fn s3_put_object_acl(
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
    auth.authorize("media:update")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
    if !content.is_empty() {
        return Err(S3ApiError::acl_not_supported(uri.path(), request_id));
    }
    let visibility = s3_canned_acl(headers, uri.path(), request_id)?.ok_or_else(|| {
        S3ApiError::invalid_argument(
            "x-amz-acl must be private or public-read.",
            uri.path(),
            request_id,
        )
    })?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 Bucket lookup failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), request_id))?;
    let mut media = state
        .repository
        .find_by_object_key(auth.application.id, bucket.id(), object_key)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 object ACL lookup failed");
            S3ApiError::service_unavailable(uri.path(), request_id)
        })?
        .filter(|media| media.state().is_readable())
        .ok_or_else(|| S3ApiError::no_such_key(uri.path(), request_id))?;
    if media.visibility_override() != Some(visibility) {
        let expected_revision = media.revision();
        let now = OffsetDateTime::now_utc();
        media
            .set_visibility_override(Some(visibility), expected_revision, now)
            .map_err(|error| {
                S3ApiError::invalid_argument(error.to_string(), uri.path(), request_id)
            })?;
        let event = OutboxEvent::media_metadata_updated(&media, now);
        state
            .repository
            .update_media(media.clone(), expected_revision, event)
            .await
            .map_err(ApiError::from_repository)
            .map_err(|error| S3ApiError::from_api(error, uri.path(), request_id))?;
        record_audit(
            state,
            auth,
            request_id,
            "media.updated",
            "media",
            media.id().to_string(),
            serde_json::json!({
                "protocol": "s3",
                "acl": if visibility == Visibility::Public { "public-read" } else { "private" },
                "object_key": object_key,
            }),
        )
        .await;
    }
    Ok(s3_empty_response(StatusCode::OK, request_id))
}

fn s3_canned_acl(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<Option<Visibility>, S3ApiError> {
    if headers
        .keys()
        .any(|name| name.as_str().starts_with("x-amz-grant-"))
    {
        return Err(S3ApiError::acl_not_supported(resource, request_id));
    }
    let Some(value) = headers.get("x-amz-acl") else {
        return Ok(None);
    };
    match value.to_str().ok() {
        Some("private") => Ok(Some(Visibility::Private)),
        Some("public-read") => Ok(Some(Visibility::Public)),
        _ => Err(S3ApiError::acl_not_supported(resource, request_id)),
    }
}

fn validate_s3_object_key(
    object_key: &str,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    if object_key.is_empty() || object_key.len() > 1024 || object_key.as_bytes().contains(&0) {
        Err(S3ApiError::invalid_argument(
            "The object key is invalid.",
            resource,
            request_id,
        ))
    } else {
        Ok(())
    }
}

pub(super) fn s3_object_names(
    object_key: &str,
    resource: &str,
    request_id: &str,
) -> Result<(String, Option<String>), S3ApiError> {
    validate_s3_object_key(object_key, resource, request_id)?;
    let display_name = object_key
        .rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            S3ApiError::invalid_argument("The object key is invalid.", resource, request_id)
        })?
        .to_owned();
    let extension = display_name
        .rsplit_once('.')
        .and_then(|(_, extension)| (!extension.is_empty()).then(|| extension.to_owned()));
    Ok((display_name, extension))
}

fn s3_query_value(
    uri: &Uri,
    name: &'static str,
    request_id: &str,
) -> Result<Option<String>, S3ApiError> {
    let mut value = None;
    for (candidate, candidate_value) in
        url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes())
    {
        if candidate == name && value.replace(candidate_value.into_owned()).is_some() {
            return Err(S3ApiError::invalid_argument(
                format!("Query parameter {name} must not occur more than once."),
                uri.path(),
                request_id,
            ));
        }
    }
    Ok(value)
}

fn s3_query_flag(uri: &Uri, name: &'static str, request_id: &str) -> Result<bool, S3ApiError> {
    s3_query_value(uri, name, request_id).map(|value| value.is_some())
}

fn reject_s3_versioning(uri: &Uri, request_id: &str) -> Result<(), S3ApiError> {
    if s3_query_value(uri, "versionId", request_id)?.is_some() {
        Err(S3ApiError::not_implemented(
            "Object Versioning is not supported.",
            uri.path(),
            request_id,
        ))
    } else {
        Ok(())
    }
}

fn s3_xml_response(status: StatusCode, body: String, request_id: &str) -> Response {
    let mut response = (status, body).into_response();
    response
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));
    insert_s3_request_id(&mut response, request_id);
    response
}

fn s3_empty_response(status: StatusCode, request_id: &str) -> Response {
    let mut response = status.into_response();
    insert_s3_request_id(&mut response, request_id);
    response
}

fn insert_s3_request_id(response: &mut Response, request_id: &str) {
    response.headers_mut().insert(
        HeaderName::from_static("x-amz-request-id"),
        HeaderValue::from_str(request_id)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid-request-id")),
    );
}

pub(super) async fn s3_post_object(
    State(state): State<Arc<AppState>>,
    Path((bucket_name, object_key)): Path<(String, String)>,
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
    if s3_query_flag(&uri, "uploads", &request_id.0.0)? {
        return s3_create_multipart_upload(
            &state,
            &auth,
            &bucket_name,
            &object_key,
            &headers,
            &uri,
            &request_id.0.0,
        )
        .await;
    }
    if let Some(upload_id) = s3_query_value(&uri, "uploadId", &request_id.0.0)? {
        return s3_complete_multipart_upload(
            S3ObjectOperation {
                state: &state,
                auth: &auth,
                bucket_name: &bucket_name,
                object_key: &object_key,
                uri: &uri,
                request_id: &request_id.0.0,
            },
            &upload_id,
            &content,
        )
        .await;
    }
    Err(S3ApiError::not_implemented(
        "Only CreateMultipartUpload and CompleteMultipartUpload are supported for POST.",
        uri.path(),
        &request_id.0.0,
    ))
}

pub(super) async fn s3_delete_object(
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
    if let Some(upload_id) = s3_query_value(&uri, "uploadId", &request_id.0.0)? {
        return s3_abort_multipart_upload(
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
    auth.authorize("media:delete")
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
    schedule_s3_delete(&state, &auth, bucket.id(), &object_key, &request_id.0.0)
        .await
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    Ok(s3_empty_response(StatusCode::NO_CONTENT, &request_id.0.0))
}

#[derive(Debug)]
pub(super) struct S3ApiError {
    pub(super) status: StatusCode,
    pub(super) code: &'static str,
    message: String,
    resource: String,
    request_id: String,
}

impl S3ApiError {
    fn new(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
        resource: &str,
        request_id: &str,
    ) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            resource: resource.to_owned(),
            request_id: request_id.to_owned(),
        }
    }

    fn from_sigv4(error: SigV4Error, resource: &str, request_id: &str) -> Self {
        let status = match error {
            SigV4Error::UnsupportedAlgorithm
            | SigV4Error::InvalidCredentialScope
            | SigV4Error::InvalidDate
            | SigV4Error::InvalidSignedHeaders
            | SigV4Error::InvalidSignature
            | SigV4Error::InvalidExpiry
            | SigV4Error::SessionCredentialsUnsupported
            | SigV4Error::DuplicateQueryParameter
            | SigV4Error::InvalidPayloadHash
            | SigV4Error::InvalidRequest => StatusCode::BAD_REQUEST,
            SigV4Error::MissingAuthentication
            | SigV4Error::SignatureMismatch
            | SigV4Error::Expired
            | SigV4Error::PayloadHashMismatch => StatusCode::FORBIDDEN,
        };
        Self::new(
            status,
            error.s3_code(),
            error.to_string(),
            resource,
            request_id,
        )
    }

    fn invalid_access_key(resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "InvalidAccessKeyId",
            "The AWS Access Key Id you provided does not exist in our records.",
            resource,
            request_id,
        )
    }

    fn no_such_bucket(resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NoSuchBucket",
            "The specified bucket does not exist.",
            resource,
            request_id,
        )
    }

    fn no_such_key(resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NoSuchKey",
            "The specified key does not exist.",
            resource,
            request_id,
        )
    }

    fn no_such_upload(resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "NoSuchUpload",
            "The specified multipart upload does not exist.",
            resource,
            request_id,
        )
    }

    fn invalid_argument(message: impl Into<String>, resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "InvalidArgument",
            message,
            resource,
            request_id,
        )
    }

    fn operation_aborted(message: impl Into<String>, resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::CONFLICT,
            "OperationAborted",
            message,
            resource,
            request_id,
        )
    }

    fn entity_too_small(resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "EntityTooSmall",
            "Your proposed upload is smaller than the minimum allowed object size.",
            resource,
            request_id,
        )
    }

    fn not_implemented(message: impl Into<String>, resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::NOT_IMPLEMENTED,
            "NotImplemented",
            message,
            resource,
            request_id,
        )
    }

    fn acl_not_supported(resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "AccessControlListNotSupported",
            "Only the private and public-read canned ACLs are supported.",
            resource,
            request_id,
        )
    }

    fn from_list(error: S3ListError, resource: &str, request_id: &str) -> Self {
        match error {
            S3ListError::TokenEncodingFailed
            | S3ListError::PageExceedsMaxKeys
            | S3ListError::InvalidLastModified
            | S3ListError::InvalidXmlCharacter
            | S3ListError::InvalidInternalCursor
            | S3ListError::InvalidBucketContext => Self::service_unavailable(resource, request_id),
            error => Self::invalid_argument(error.to_string(), resource, request_id),
        }
    }

    fn from_xml(error: S3XmlError, resource: &str, request_id: &str) -> Self {
        let status = if error == S3XmlError::InputTooLarge {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::BAD_REQUEST
        };
        Self::new(
            status,
            error.s3_code(),
            error.to_string(),
            resource,
            request_id,
        )
    }

    fn from_multipart_manifest(
        error: S3MultipartManifestError,
        resource: &str,
        request_id: &str,
    ) -> Self {
        let (code, message) = match error {
            S3MultipartManifestError::InvalidPartOrder => (
                "InvalidPartOrder",
                "The list of parts was not in ascending order.",
            ),
            S3MultipartManifestError::Empty => {
                ("InvalidPart", "The multipart upload contains no parts.")
            }
            S3MultipartManifestError::InvalidPartNumber(_)
            | S3MultipartManifestError::MissingPart(_)
            | S3MultipartManifestError::EtagMismatch(_) => (
                "InvalidPart",
                "One or more of the specified parts could not be found or did not match its ETag.",
            ),
        };
        Self::new(StatusCode::BAD_REQUEST, code, message, resource, request_id)
    }

    fn service_unavailable(resource: &str, request_id: &str) -> Self {
        Self::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "ServiceUnavailable",
            "Please reduce your request rate.",
            resource,
            request_id,
        )
    }

    pub(super) fn from_api(error: ApiError, resource: &str, request_id: &str) -> Self {
        let (status, code) = match error.code {
            "forbidden" | "unauthorized" => (StatusCode::FORBIDDEN, "AccessDenied"),
            "not_found" if error.message == "bucket not found" => {
                (StatusCode::NOT_FOUND, "NoSuchBucket")
            }
            "not_found" => (StatusCode::NOT_FOUND, "NoSuchKey"),
            "object_exists" | "conflict" => (StatusCode::CONFLICT, "OperationAborted"),
            "payload_too_large" | "quota_exceeded" => (StatusCode::BAD_REQUEST, "EntityTooLarge"),
            "invalid_range" => (StatusCode::RANGE_NOT_SATISFIABLE, "InvalidRange"),
            "unavailable" => (StatusCode::SERVICE_UNAVAILABLE, "ServiceUnavailable"),
            _ => (StatusCode::BAD_REQUEST, "InvalidRequest"),
        };
        Self::new(status, code, error.message, resource, request_id)
    }
}

impl IntoResponse for S3ApiError {
    fn into_response(self) -> Response {
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{}</Code><Message>{}</Message><Resource>{}</Resource><RequestId>{}</RequestId></Error>",
            escape_s3_xml(self.code),
            escape_s3_xml(&self.message),
            escape_s3_xml(&self.resource),
            escape_s3_xml(&self.request_id),
        );
        let mut response = (self.status, body).into_response();
        response
            .headers_mut()
            .insert(CONTENT_TYPE, HeaderValue::from_static("application/xml"));
        response.headers_mut().insert(
            HeaderName::from_static("x-amz-request-id"),
            HeaderValue::from_str(&self.request_id)
                .unwrap_or_else(|_| HeaderValue::from_static("invalid-request-id")),
        );
        response
    }
}

fn escape_s3_xml(value: &str) -> Cow<'_, str> {
    if !value
        .bytes()
        .any(|byte| matches!(byte, b'&' | b'<' | b'>' | b'\"' | b'\''))
    {
        return Cow::Borrowed(value);
    }
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            character => escaped.push(character),
        }
    }
    Cow::Owned(escaped)
}
