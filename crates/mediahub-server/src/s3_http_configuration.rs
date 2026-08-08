// S3 Bucket configuration operation dispatch, XML codecs, and handlers.

const MAX_S3_BUCKET_CONFIGURATION_BYTES: usize = 1024 * 1024;
const MAX_S3_LIFECYCLE_RULES: usize = 1_000;
const MAX_S3_LIFECYCLE_ID_BYTES: usize = 255;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum S3BucketPutOperation {
    CreateBucket,
    PutPolicy,
    PutVersioning,
    PutLifecycle,
    PutObjectLock,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum S3BucketDeleteOperation {
    Bucket,
    Policy,
    Lifecycle,
}

fn classify_s3_bucket_put(uri: &Uri, request_id: &str) -> Result<S3BucketPutOperation, S3ApiError> {
    let versioning = s3_query_flag(uri, "versioning", request_id)?;
    let lifecycle = s3_query_flag(uri, "lifecycle", request_id)?;
    let object_lock = s3_query_flag(uri, "object-lock", request_id)?;
    let policy = s3_query_flag(uri, "policy", request_id)?;
    if usize::from(versioning)
        + usize::from(lifecycle)
        + usize::from(object_lock)
        + usize::from(policy)
        > 1
    {
        return Err(S3ApiError::invalid_request(
            "Bucket configuration subresources cannot be combined.",
            uri.path(),
            request_id,
        ));
    }
    Ok(if policy {
        S3BucketPutOperation::PutPolicy
    } else if versioning {
        S3BucketPutOperation::PutVersioning
    } else if lifecycle {
        S3BucketPutOperation::PutLifecycle
    } else if object_lock {
        S3BucketPutOperation::PutObjectLock
    } else {
        S3BucketPutOperation::CreateBucket
    })
}

fn classify_s3_bucket_delete(
    uri: &Uri,
    request_id: &str,
) -> Result<S3BucketDeleteOperation, S3ApiError> {
    let versioning = s3_query_flag(uri, "versioning", request_id)?;
    let lifecycle = s3_query_flag(uri, "lifecycle", request_id)?;
    let object_lock = s3_query_flag(uri, "object-lock", request_id)?;
    let policy = s3_query_flag(uri, "policy", request_id)?;
    if usize::from(versioning)
        + usize::from(lifecycle)
        + usize::from(object_lock)
        + usize::from(policy)
        > 1
    {
        return Err(S3ApiError::invalid_request(
            "Bucket configuration subresources cannot be combined.",
            uri.path(),
            request_id,
        ));
    }
    if versioning {
        return Err(S3ApiError::method_not_allowed(
            "DeleteBucketVersioning is not an S3 operation.",
            uri.path(),
            request_id,
        ));
    }
    if object_lock {
        return Err(S3ApiError::method_not_allowed(
            "Object Lock cannot be disabled after it has been enabled.",
            uri.path(),
            request_id,
        ));
    }
    Ok(if policy {
        S3BucketDeleteOperation::Policy
    } else if lifecycle {
        S3BucketDeleteOperation::Lifecycle
    } else {
        S3BucketDeleteOperation::Bucket
    })
}

pub(super) async fn s3_bucket_get(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    let operation = classify_s3_bucket_get(&uri, &request_id.0.0)?;
    match operation {
        S3BucketGetOperation::ListObjects => {
            s3_list_objects(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketGetOperation::ListObjectVersions => {
            s3_list_object_versions(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketGetOperation::ListMultipartUploads => {
            s3_list_multipart_uploads(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketGetOperation::GetLocation => {
            s3_get_bucket_location(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketGetOperation::GetVersioning => {
            s3_get_bucket_versioning(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketGetOperation::GetLifecycle => {
            s3_get_bucket_lifecycle_configuration(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketGetOperation::GetObjectLock => {
            s3_get_bucket_object_lock(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketGetOperation::GetPolicy => {
            s3_get_bucket_policy(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketGetOperation::GetPolicyStatus => {
            s3_get_bucket_policy_status(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
    }
}

pub(super) async fn s3_bucket_put(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    content: Body,
) -> Result<Response, S3ApiError> {
    let operation = classify_s3_bucket_put(&uri, &request_id.0.0)?;
    if operation == S3BucketPutOperation::PutPolicy {
        return s3_put_bucket_policy(
            State(state),
            Path(bucket_name),
            OriginalUri(uri),
            method,
            headers,
            request_id,
            content,
        )
        .await;
    }
    let content = to_bytes(content, MAX_S3_BUCKET_CONFIGURATION_BYTES + 1)
        .await
        .map_err(|_| {
            S3ApiError::entity_too_large(uri.path(), &request_id.0.0)
        })?;
    match operation {
        S3BucketPutOperation::CreateBucket => {
            s3_create_bucket(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
                content,
            )
            .await
        }
        S3BucketPutOperation::PutPolicy => unreachable!("policy is dispatched before body read"),
        S3BucketPutOperation::PutVersioning => {
            s3_put_bucket_versioning(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
                content,
            )
            .await
        }
        S3BucketPutOperation::PutLifecycle => {
            s3_put_bucket_lifecycle_configuration(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
                content,
            )
            .await
        }
        S3BucketPutOperation::PutObjectLock => {
            s3_put_bucket_object_lock(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
                content,
            )
            .await
        }
    }
}

pub(super) async fn s3_bucket_delete(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    let operation = classify_s3_bucket_delete(&uri, &request_id.0.0)?;
    match operation {
        S3BucketDeleteOperation::Bucket => {
            s3_delete_bucket(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketDeleteOperation::Lifecycle => {
            s3_delete_bucket_lifecycle_configuration(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
        S3BucketDeleteOperation::Policy => {
            s3_delete_bucket_policy(
                State(state),
                Path(bucket_name),
                OriginalUri(uri),
                method,
                headers,
                request_id,
            )
            .await
        }
    }
}

async fn s3_get_bucket_versioning(
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
    auth.authorize("bucket:list")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let status = state
        .repository
        .get_s3_bucket_versioning(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 GetBucketVersioning lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    Ok(s3_xml_response(
        StatusCode::OK,
        s3_bucket_versioning_xml(status),
        &request_id.0.0,
    ))
}

async fn s3_put_bucket_versioning(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    content: Bytes,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &content, &request_id.0.0)
            .await?;
    auth.authorize("bucket:manage")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    validate_s3_configuration_content_md5(&headers, &content, &uri, &request_id.0.0)?;
    let status = parse_s3_bucket_versioning_xml(&content).map_err(|error| match error {
        S3BucketConfigurationXmlError::MalformedXml => {
            S3ApiError::malformed_xml(uri.path(), &request_id.0.0)
        }
        S3BucketConfigurationXmlError::Invalid(message) => {
            S3ApiError::invalid_request(message, uri.path(), &request_id.0.0)
        }
        S3BucketConfigurationXmlError::Unsupported(feature) => {
            S3ApiError::not_implemented(feature, uri.path(), &request_id.0.0)
        }
    })?;
    state
        .repository
        .set_s3_bucket_versioning(
            auth.application.id,
            &bucket_name,
            status,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|error| {
            map_s3_configuration_repository_error(error, uri.path(), &request_id.0.0)
        })?;
    Ok(s3_empty_response(StatusCode::OK, &request_id.0.0))
}

async fn s3_get_bucket_lifecycle_configuration(
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
    auth.authorize("bucket:list")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    let configuration = state
        .repository
        .get_s3_bucket_configuration(auth.application.id, &bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 GetBucketLifecycleConfiguration lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    let document = configuration
        .lifecycle_configuration()
        .ok_or_else(|| S3ApiError::no_such_lifecycle_configuration(uri.path(), &request_id.0.0))?;
    let lifecycle = s3_lifecycle_document_from_core(document).map_err(|error| {
        warn!(error = %error, "persisted S3 lifecycle configuration is invalid");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    lifecycle.validate_persisted().map_err(|error| {
        warn!(error = %error, "persisted S3 lifecycle configuration violates protocol invariants");
        S3ApiError::internal_error(uri.path(), &request_id.0.0)
    })?;
    Ok(s3_xml_response(
        StatusCode::OK,
        s3_bucket_lifecycle_xml(&lifecycle),
        &request_id.0.0,
    ))
}

async fn s3_put_bucket_lifecycle_configuration(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    OriginalUri(uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
    content: Bytes,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(&bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(uri.path(), &request_id.0.0))?;
    let auth =
        authenticate_s3_application(&state, &method, &uri, &headers, &content, &request_id.0.0)
            .await?;
    auth.authorize("bucket:manage")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    validate_s3_configuration_content_md5(&headers, &content, &uri, &request_id.0.0)?;
    let lifecycle = parse_s3_bucket_lifecycle_xml(&content).map_err(|error| match error {
        S3BucketConfigurationXmlError::MalformedXml => {
            S3ApiError::malformed_xml(uri.path(), &request_id.0.0)
        }
        S3BucketConfigurationXmlError::Invalid(message) => {
            S3ApiError::invalid_request(message, uri.path(), &request_id.0.0)
        }
        S3BucketConfigurationXmlError::Unsupported(feature) => {
            S3ApiError::not_implemented(feature, uri.path(), &request_id.0.0)
        }
    })?;
    let document = s3_lifecycle_document_to_core(lifecycle).map_err(|error| match error {
        S3BucketConfigurationXmlError::Invalid(message) => {
            S3ApiError::invalid_request(message, uri.path(), &request_id.0.0)
        }
        S3BucketConfigurationXmlError::Unsupported(feature) => {
            S3ApiError::not_implemented(feature, uri.path(), &request_id.0.0)
        }
        S3BucketConfigurationXmlError::MalformedXml => {
            S3ApiError::malformed_xml(uri.path(), &request_id.0.0)
        }
    })?;
    state
        .repository
        .replace_s3_bucket_lifecycle(
            auth.application.id,
            &bucket_name,
            Some(document),
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|error| {
            map_s3_configuration_repository_error(error, uri.path(), &request_id.0.0)
        })?;
    Ok(s3_empty_response(StatusCode::OK, &request_id.0.0))
}

async fn s3_delete_bucket_lifecycle_configuration(
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
    auth.authorize("bucket:manage")
        .map_err(|error| S3ApiError::from_api(error, uri.path(), &request_id.0.0))?;
    state
        .repository
        .replace_s3_bucket_lifecycle(
            auth.application.id,
            &bucket_name,
            None,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|error| {
            map_s3_configuration_repository_error(error, uri.path(), &request_id.0.0)
        })?;
    Ok(s3_empty_response(StatusCode::NO_CONTENT, &request_id.0.0))
}

fn validate_s3_configuration_content_md5(
    headers: &HeaderMap,
    content: &[u8],
    uri: &Uri,
    request_id: &str,
) -> Result<(), S3ApiError> {
    validate_content_md5(
        headers.get("content-md5").map(HeaderValue::as_bytes),
        content,
    )
    .map_err(|error| {
        S3ApiError::new(
            StatusCode::BAD_REQUEST,
            error.s3_code(),
            error.to_string(),
            uri.path(),
            request_id,
        )
    })
}

fn map_s3_configuration_repository_error(
    error: mediahub_app::RepositoryError,
    resource: &str,
    request_id: &str,
) -> S3ApiError {
    match error {
        mediahub_app::RepositoryError::NotFound => S3ApiError::no_such_bucket(resource, request_id),
        mediahub_app::RepositoryError::Conflict => S3ApiError::operation_aborted(
            "A conflicting conditional operation is currently in progress against this resource.",
            resource,
            request_id,
        ),
        mediahub_app::RepositoryError::Invariant(message) => {
            S3ApiError::invalid_bucket_state(message, resource, request_id)
        }
        error => {
            warn!(error = %error, "S3 Bucket configuration persistence failed");
            S3ApiError::service_unavailable(resource, request_id)
        }
    }
}

fn s3_bucket_versioning_xml(status: VersioningStatus) -> String {
    let status = match status {
        VersioningStatus::Unversioned => "",
        VersioningStatus::Enabled => "<Status>Enabled</Status>",
        VersioningStatus::Suspended => "<Status>Suspended</Status>",
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><VersioningConfiguration xmlns=\"{S3_XML_NAMESPACE}\">{status}</VersioningConfiguration>"
    )
}

#[derive(Debug, PartialEq, Eq)]
enum S3BucketConfigurationXmlError {
    MalformedXml,
    Invalid(String),
    Unsupported(&'static str),
}

impl std::fmt::Display for S3BucketConfigurationXmlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedXml => formatter.write_str("malformed XML"),
            Self::Invalid(message) => formatter.write_str(message),
            Self::Unsupported(feature) => formatter.write_str(feature),
        }
    }
}

fn parse_s3_bucket_versioning_xml(
    input: &[u8],
) -> Result<VersioningStatus, S3BucketConfigurationXmlError> {
    let root = parse_s3_configuration_xml(input)?;
    validate_s3_configuration_root(&root, "VersioningConfiguration")?;
    let mut status = None;
    for child in &root.children {
        match child.name.as_str() {
            "Status" if status.is_none() => {
                status = Some(match required_s3_leaf_text(child)? {
                    "Enabled" => VersioningStatus::Enabled,
                    "Suspended" => VersioningStatus::Suspended,
                    _ => {
                        return Err(S3BucketConfigurationXmlError::Invalid(
                            "Versioning Status must be Enabled or Suspended.".to_owned(),
                        ));
                    }
                });
            }
            "MFADelete" => {
                return Err(S3BucketConfigurationXmlError::Unsupported(
                    "MFA Delete is not supported.",
                ));
            }
            _ => return Err(S3BucketConfigurationXmlError::MalformedXml),
        }
    }
    status.ok_or_else(|| {
        S3BucketConfigurationXmlError::Invalid(
            "VersioningConfiguration requires exactly one Status.".to_owned(),
        )
    })
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct S3LifecycleDocument {
    schema_version: u8,
    rules: Vec<S3LifecycleRuleDocument>,
}

impl S3LifecycleDocument {
    fn validate_persisted(&self) -> Result<(), &'static str> {
        if self.schema_version != 1
            || self.rules.is_empty()
            || self.rules.len() > MAX_S3_LIFECYCLE_RULES
        {
            return Err("invalid lifecycle document envelope");
        }
        let mut ids = std::collections::HashSet::new();
        for rule in &self.rules {
            if rule
                .id
                .as_ref()
                .is_some_and(|id| id.len() > MAX_S3_LIFECYCLE_ID_BYTES || !ids.insert(id.as_str()))
            {
                return Err("invalid or duplicate lifecycle rule ID");
            }
            if rule.expiration.is_none()
                && rule.noncurrent_version_expiration.is_none()
                && rule.abort_incomplete_multipart_upload.is_none()
            {
                return Err("lifecycle rule has no supported action");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct S3LifecycleRuleDocument {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    status: S3LifecycleStatus,
    filter: S3LifecycleFilter,
    #[serde(skip_serializing_if = "Option::is_none")]
    expiration: Option<S3LifecycleExpiration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    noncurrent_version_expiration: Option<S3LifecycleNoncurrentExpiration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    abort_incomplete_multipart_upload: Option<S3LifecycleAbortMultipart>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum S3LifecycleStatus {
    Enabled,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum S3LifecycleFilter {
    Empty,
    Prefix { value: String },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum S3LifecycleExpiration {
    Days { days: u32 },
    Date { date: String },
    ExpiredObjectDeleteMarker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct S3LifecycleNoncurrentExpiration {
    noncurrent_days: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct S3LifecycleAbortMultipart {
    days_after_initiation: u32,
}

fn s3_lifecycle_document_to_core(
    document: S3LifecycleDocument,
) -> Result<mediahub_core::S3LifecycleConfiguration, S3BucketConfigurationXmlError> {
    if document.schema_version != 1 {
        return Err(S3BucketConfigurationXmlError::Invalid(
            "Lifecycle document schema version is invalid.".to_owned(),
        ));
    }
    let rules = document
        .rules
        .into_iter()
        .map(|rule| {
            let filter = match rule.filter {
                S3LifecycleFilter::Empty => mediahub_core::S3LifecycleFilter::Empty,
                S3LifecycleFilter::Prefix { value } => {
                    mediahub_core::S3LifecycleFilter::Prefix(value)
                }
            };
            let expiration = match rule.expiration {
                None => None,
                Some(S3LifecycleExpiration::Days { days }) => {
                    Some(mediahub_core::S3Expiration::Days(days))
                }
                Some(S3LifecycleExpiration::Date { date }) => Some(
                    mediahub_core::S3Expiration::Date(
                        OffsetDateTime::parse(
                            &date,
                            &time::format_description::well_known::Rfc3339,
                        )
                        .map_err(|_| {
                            S3BucketConfigurationXmlError::Invalid(
                                "Lifecycle Expiration Date is invalid.".to_owned(),
                            )
                        })?,
                    ),
                ),
                Some(S3LifecycleExpiration::ExpiredObjectDeleteMarker) => {
                    Some(mediahub_core::S3Expiration::ExpiredObjectDeleteMarker)
                }
            };
            Ok(mediahub_core::S3LifecycleRule {
                id: rule.id,
                status: match rule.status {
                    S3LifecycleStatus::Enabled => mediahub_core::S3LifecycleRuleStatus::Enabled,
                    S3LifecycleStatus::Disabled => mediahub_core::S3LifecycleRuleStatus::Disabled,
                },
                filter,
                expiration,
                noncurrent_version_expiration: rule.noncurrent_version_expiration.map(|value| {
                    mediahub_core::S3NoncurrentVersionExpiration {
                        noncurrent_days: value.noncurrent_days,
                    }
                }),
                abort_incomplete_multipart_upload: rule
                    .abort_incomplete_multipart_upload
                    .map(|value| mediahub_core::S3AbortIncompleteMultipartUpload {
                        days_after_initiation: value.days_after_initiation,
                    }),
            })
        })
        .collect::<Result<Vec<_>, S3BucketConfigurationXmlError>>()?;
    mediahub_core::S3LifecycleConfiguration::new(rules).map_err(|error| {
        S3BucketConfigurationXmlError::Invalid(format!(
            "Lifecycle configuration violates domain invariants: {error}"
        ))
    })
}

fn s3_lifecycle_document_from_core(
    configuration: &mediahub_core::S3LifecycleConfiguration,
) -> Result<S3LifecycleDocument, String> {
    configuration.validate().map_err(|error| error.to_string())?;
    let rules = configuration
        .rules
        .iter()
        .map(|rule| {
            let expiration = match rule.expiration.as_ref() {
                None => None,
                Some(mediahub_core::S3Expiration::Days(days)) => {
                    Some(S3LifecycleExpiration::Days { days: *days })
                }
                Some(mediahub_core::S3Expiration::Date(date)) => Some(
                    S3LifecycleExpiration::Date {
                        date: date
                            .format(&time::format_description::well_known::Rfc3339)
                            .map_err(|error| error.to_string())?,
                    },
                ),
                Some(mediahub_core::S3Expiration::ExpiredObjectDeleteMarker) => {
                    Some(S3LifecycleExpiration::ExpiredObjectDeleteMarker)
                }
            };
            Ok(S3LifecycleRuleDocument {
                id: rule.id.clone(),
                status: match rule.status {
                    mediahub_core::S3LifecycleRuleStatus::Enabled => S3LifecycleStatus::Enabled,
                    mediahub_core::S3LifecycleRuleStatus::Disabled => S3LifecycleStatus::Disabled,
                },
                filter: match &rule.filter {
                    mediahub_core::S3LifecycleFilter::Empty => S3LifecycleFilter::Empty,
                    mediahub_core::S3LifecycleFilter::Prefix(value) => {
                        S3LifecycleFilter::Prefix {
                            value: value.clone(),
                        }
                    }
                },
                expiration,
                noncurrent_version_expiration: rule
                    .noncurrent_version_expiration
                    .map(|value| S3LifecycleNoncurrentExpiration {
                        noncurrent_days: value.noncurrent_days,
                    }),
                abort_incomplete_multipart_upload: rule
                    .abort_incomplete_multipart_upload
                    .map(|value| S3LifecycleAbortMultipart {
                        days_after_initiation: value.days_after_initiation,
                    }),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(S3LifecycleDocument {
        schema_version: 1,
        rules,
    })
}
fn parse_s3_bucket_lifecycle_xml(
    input: &[u8],
) -> Result<S3LifecycleDocument, S3BucketConfigurationXmlError> {
    let root = parse_s3_configuration_xml(input)?;
    validate_s3_configuration_root(&root, "LifecycleConfiguration")?;
    if root.children.is_empty() || root.children.len() > MAX_S3_LIFECYCLE_RULES {
        return Err(S3BucketConfigurationXmlError::Invalid(format!(
            "LifecycleConfiguration must contain between 1 and {MAX_S3_LIFECYCLE_RULES} rules."
        )));
    }
    let mut rules = Vec::with_capacity(root.children.len());
    let mut ids = std::collections::HashSet::new();
    for child in &root.children {
        if child.name != "Rule" {
            return Err(S3BucketConfigurationXmlError::MalformedXml);
        }
        let rule = parse_s3_lifecycle_rule(child)?;
        if let Some(id) = &rule.id
            && !ids.insert(id.clone())
        {
            return Err(S3BucketConfigurationXmlError::Invalid(
                "Lifecycle rule IDs must be unique.".to_owned(),
            ));
        }
        rules.push(rule);
    }
    Ok(S3LifecycleDocument {
        schema_version: 1,
        rules,
    })
}

fn parse_s3_lifecycle_rule(
    element: &S3ConfigurationXmlElement,
) -> Result<S3LifecycleRuleDocument, S3BucketConfigurationXmlError> {
    validate_s3_container(element)?;
    let mut id = None;
    let mut status = None;
    let mut filter = None;
    let mut expiration = None;
    let mut noncurrent_version_expiration = None;
    let mut abort_incomplete_multipart_upload = None;
    for child in &element.children {
        match child.name.as_str() {
            "ID" if id.is_none() => {
                let value = required_s3_leaf_text(child)?.to_owned();
                if value.len() > MAX_S3_LIFECYCLE_ID_BYTES {
                    return Err(S3BucketConfigurationXmlError::Invalid(
                        "Lifecycle rule ID must not exceed 255 bytes.".to_owned(),
                    ));
                }
                id = Some(value);
            }
            "Status" if status.is_none() => {
                status = Some(match required_s3_leaf_text(child)? {
                    "Enabled" => S3LifecycleStatus::Enabled,
                    "Disabled" => S3LifecycleStatus::Disabled,
                    _ => {
                        return Err(S3BucketConfigurationXmlError::Invalid(
                            "Lifecycle rule Status must be Enabled or Disabled.".to_owned(),
                        ));
                    }
                });
            }
            "Filter" if filter.is_none() => filter = Some(parse_s3_lifecycle_filter(child)?),
            "Expiration" if expiration.is_none() => {
                expiration = Some(parse_s3_lifecycle_expiration(child)?);
            }
            "NoncurrentVersionExpiration" if noncurrent_version_expiration.is_none() => {
                noncurrent_version_expiration =
                    Some(parse_s3_lifecycle_noncurrent_expiration(child)?);
            }
            "AbortIncompleteMultipartUpload" if abort_incomplete_multipart_upload.is_none() => {
                abort_incomplete_multipart_upload =
                    Some(parse_s3_lifecycle_abort_multipart(child)?);
            }
            "Transition" => {
                return Err(S3BucketConfigurationXmlError::Unsupported(
                    "Lifecycle Transition is not supported.",
                ));
            }
            "NoncurrentVersionTransition" => {
                return Err(S3BucketConfigurationXmlError::Unsupported(
                    "Lifecycle NoncurrentVersionTransition is not supported.",
                ));
            }
            _ => return Err(S3BucketConfigurationXmlError::MalformedXml),
        }
    }
    let filter = filter.ok_or_else(|| {
        S3BucketConfigurationXmlError::Invalid(
            "Lifecycle rule requires an empty Filter or a Prefix Filter.".to_owned(),
        )
    })?;
    let status = status.ok_or_else(|| {
        S3BucketConfigurationXmlError::Invalid(
            "Lifecycle rule requires exactly one Status.".to_owned(),
        )
    })?;
    if expiration.is_none()
        && noncurrent_version_expiration.is_none()
        && abort_incomplete_multipart_upload.is_none()
    {
        return Err(S3BucketConfigurationXmlError::Invalid(
            "Lifecycle rule requires at least one supported action.".to_owned(),
        ));
    }
    Ok(S3LifecycleRuleDocument {
        id,
        status,
        filter,
        expiration,
        noncurrent_version_expiration,
        abort_incomplete_multipart_upload,
    })
}

fn parse_s3_lifecycle_filter(
    element: &S3ConfigurationXmlElement,
) -> Result<S3LifecycleFilter, S3BucketConfigurationXmlError> {
    validate_s3_container(element)?;
    match element.children.as_slice() {
        [] => Ok(S3LifecycleFilter::Empty),
        [prefix] if prefix.name == "Prefix" => {
            let value = s3_leaf_text(prefix)?.to_owned();
            if value.len() > 1024 || value.as_bytes().contains(&0) {
                return Err(S3BucketConfigurationXmlError::Invalid(
                    "Lifecycle Prefix is invalid.".to_owned(),
                ));
            }
            Ok(S3LifecycleFilter::Prefix { value })
        }
        children
            if children.iter().any(|child| {
                matches!(
                    child.name.as_str(),
                    "Tag" | "And" | "ObjectSizeGreaterThan" | "ObjectSizeLessThan"
                )
            }) =>
        {
            Err(S3BucketConfigurationXmlError::Unsupported(
                "Lifecycle Tag, And, and ObjectSize filters are not supported.",
            ))
        }
        _ => Err(S3BucketConfigurationXmlError::MalformedXml),
    }
}

fn parse_s3_lifecycle_expiration(
    element: &S3ConfigurationXmlElement,
) -> Result<S3LifecycleExpiration, S3BucketConfigurationXmlError> {
    validate_s3_container(element)?;
    match element.children.as_slice() {
        [child] if child.name == "Days" => Ok(S3LifecycleExpiration::Days {
            days: parse_s3_positive_u32(child, "Expiration Days")?,
        }),
        [child] if child.name == "Date" => Ok(S3LifecycleExpiration::Date {
            date: parse_s3_lifecycle_date(child)?,
        }),
        [child] if child.name == "ExpiredObjectDeleteMarker" => {
            if required_s3_leaf_text(child)? != "true" {
                return Err(S3BucketConfigurationXmlError::Invalid(
                    "ExpiredObjectDeleteMarker must be true.".to_owned(),
                ));
            }
            Ok(S3LifecycleExpiration::ExpiredObjectDeleteMarker)
        }
        _ => Err(S3BucketConfigurationXmlError::MalformedXml),
    }
}

fn parse_s3_lifecycle_noncurrent_expiration(
    element: &S3ConfigurationXmlElement,
) -> Result<S3LifecycleNoncurrentExpiration, S3BucketConfigurationXmlError> {
    validate_s3_container(element)?;
    if element
        .children
        .iter()
        .any(|child| child.name == "NewerNoncurrentVersions")
    {
        return Err(S3BucketConfigurationXmlError::Unsupported(
            "Lifecycle NewerNoncurrentVersions is not supported.",
        ));
    }
    match element.children.as_slice() {
        [child] if child.name == "NoncurrentDays" => Ok(S3LifecycleNoncurrentExpiration {
            noncurrent_days: parse_s3_positive_u32(child, "NoncurrentDays")?,
        }),
        _ => Err(S3BucketConfigurationXmlError::MalformedXml),
    }
}

fn parse_s3_lifecycle_abort_multipart(
    element: &S3ConfigurationXmlElement,
) -> Result<S3LifecycleAbortMultipart, S3BucketConfigurationXmlError> {
    validate_s3_container(element)?;
    match element.children.as_slice() {
        [child] if child.name == "DaysAfterInitiation" => Ok(S3LifecycleAbortMultipart {
            days_after_initiation: parse_s3_positive_u32(child, "DaysAfterInitiation")?,
        }),
        _ => Err(S3BucketConfigurationXmlError::MalformedXml),
    }
}

fn parse_s3_positive_u32(
    element: &S3ConfigurationXmlElement,
    field: &str,
) -> Result<u32, S3BucketConfigurationXmlError> {
    required_s3_leaf_text(element)?
        .parse::<u32>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            S3BucketConfigurationXmlError::Invalid(format!("{field} must be a positive integer."))
        })
}

fn parse_s3_lifecycle_date(
    element: &S3ConfigurationXmlElement,
) -> Result<String, S3BucketConfigurationXmlError> {
    let value = required_s3_leaf_text(element)?;
    let parsed = chrono::DateTime::parse_from_rfc3339(value).map_err(|_| {
        S3BucketConfigurationXmlError::Invalid(
            "Lifecycle Expiration Date must be an ISO-8601 timestamp.".to_owned(),
        )
    })?;
    if parsed.offset().local_minus_utc() != 0
        || parsed.time() != chrono::NaiveTime::from_hms_opt(0, 0, 0).expect("valid midnight")
    {
        return Err(S3BucketConfigurationXmlError::Invalid(
            "Lifecycle Expiration Date must be midnight UTC.".to_owned(),
        ));
    }
    Ok(parsed.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

fn s3_bucket_lifecycle_xml(document: &S3LifecycleDocument) -> String {
    let mut xml = String::with_capacity(512 + document.rules.len().saturating_mul(384));
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?><LifecycleConfiguration xmlns=\"");
    xml.push_str(S3_XML_NAMESPACE);
    xml.push_str("\">");
    for rule in &document.rules {
        xml.push_str("<Rule>");
        if let Some(id) = &rule.id {
            xml.push_str("<ID>");
            xml.push_str(&escape_s3_xml(id));
            xml.push_str("</ID>");
        }
        match &rule.filter {
            S3LifecycleFilter::Empty => xml.push_str("<Filter></Filter>"),
            S3LifecycleFilter::Prefix { value } => {
                xml.push_str("<Filter><Prefix>");
                xml.push_str(&escape_s3_xml(value));
                xml.push_str("</Prefix></Filter>");
            }
        }
        xml.push_str("<Status>");
        xml.push_str(match rule.status {
            S3LifecycleStatus::Enabled => "Enabled",
            S3LifecycleStatus::Disabled => "Disabled",
        });
        xml.push_str("</Status>");
        if let Some(expiration) = &rule.expiration {
            xml.push_str("<Expiration>");
            match expiration {
                S3LifecycleExpiration::Days { days } => {
                    xml.push_str("<Days>");
                    xml.push_str(&days.to_string());
                    xml.push_str("</Days>");
                }
                S3LifecycleExpiration::Date { date } => {
                    xml.push_str("<Date>");
                    xml.push_str(&escape_s3_xml(date));
                    xml.push_str("</Date>");
                }
                S3LifecycleExpiration::ExpiredObjectDeleteMarker => {
                    xml.push_str("<ExpiredObjectDeleteMarker>true</ExpiredObjectDeleteMarker>");
                }
            }
            xml.push_str("</Expiration>");
        }
        if let Some(expiration) = rule.noncurrent_version_expiration {
            xml.push_str("<NoncurrentVersionExpiration><NoncurrentDays>");
            xml.push_str(&expiration.noncurrent_days.to_string());
            xml.push_str("</NoncurrentDays></NoncurrentVersionExpiration>");
        }
        if let Some(abort) = rule.abort_incomplete_multipart_upload {
            xml.push_str("<AbortIncompleteMultipartUpload><DaysAfterInitiation>");
            xml.push_str(&abort.days_after_initiation.to_string());
            xml.push_str("</DaysAfterInitiation></AbortIncompleteMultipartUpload>");
        }
        xml.push_str("</Rule>");
    }
    xml.push_str("</LifecycleConfiguration>");
    xml
}

#[derive(Debug)]
struct S3ConfigurationXmlElement {
    name: String,
    attributes: Vec<(String, String)>,
    text: String,
    children: Vec<Self>,
}

fn parse_s3_configuration_xml(
    input: &[u8],
) -> Result<S3ConfigurationXmlElement, S3BucketConfigurationXmlError> {
    if input.is_empty() || input.len() > MAX_S3_BUCKET_CONFIGURATION_BYTES {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    }
    let mut reader = Reader::from_reader(input);
    reader.config_mut().check_end_names = true;
    let mut buffer = Vec::new();
    let mut stack = Vec::<S3ConfigurationXmlElement>::new();
    let mut root = None;
    let mut declaration_seen = false;
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|_| S3BucketConfigurationXmlError::MalformedXml)?;
        match event {
            Event::Start(element) => {
                stack.push(new_s3_configuration_xml_element(&reader, &element)?);
            }
            Event::Empty(element) => {
                let element = new_s3_configuration_xml_element(&reader, &element)?;
                attach_s3_configuration_xml_element(&mut stack, &mut root, element)?;
            }
            Event::End(_) => {
                let element = stack
                    .pop()
                    .ok_or(S3BucketConfigurationXmlError::MalformedXml)?;
                attach_s3_configuration_xml_element(&mut stack, &mut root, element)?;
            }
            Event::Text(text) => {
                let value = text
                    .xml10_content()
                    .map_err(|_| S3BucketConfigurationXmlError::MalformedXml)?;
                append_s3_configuration_xml_text(&mut stack, &value)?;
            }
            Event::CData(text) => {
                let value = text
                    .xml10_content()
                    .map_err(|_| S3BucketConfigurationXmlError::MalformedXml)?;
                append_s3_configuration_xml_text(&mut stack, &value)?;
            }
            Event::Decl(_) if !declaration_seen && stack.is_empty() && root.is_none() => {
                declaration_seen = true;
            }
            Event::Comment(_) => {}
            Event::GeneralRef(reference) => {
                let character = if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|_| S3BucketConfigurationXmlError::MalformedXml)?
                {
                    character
                } else {
                    match reference
                        .decode()
                        .map_err(|_| S3BucketConfigurationXmlError::MalformedXml)?
                        .as_ref()
                    {
                        "amp" => '&',
                        "lt" => '<',
                        "gt" => '>',
                        "quot" => '"',
                        "apos" => '\'',
                        _ => return Err(S3BucketConfigurationXmlError::MalformedXml),
                    }
                };
                if !is_s3_configuration_xml_character(character) {
                    return Err(S3BucketConfigurationXmlError::MalformedXml);
                }
                append_s3_configuration_xml_text(&mut stack, &character.to_string())?;
            }
            Event::DocType(_) | Event::PI(_) | Event::Decl(_) => {
                return Err(S3BucketConfigurationXmlError::MalformedXml);
            }
            Event::Eof => break,
        }
        buffer.clear();
    }
    if !stack.is_empty() {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    }
    root.ok_or(S3BucketConfigurationXmlError::MalformedXml)
}

fn is_s3_configuration_xml_character(character: char) -> bool {
    matches!(
        character,
        '\u{9}' | '\u{A}' | '\u{D}' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}'
    )
}
fn new_s3_configuration_xml_element(
    reader: &Reader<&[u8]>,
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<S3ConfigurationXmlElement, S3BucketConfigurationXmlError> {
    let name = std::str::from_utf8(element.local_name().as_ref())
        .map_err(|_| S3BucketConfigurationXmlError::MalformedXml)?
        .to_owned();
    let mut attributes = Vec::new();
    for attribute in element.attributes().with_checks(true) {
        let attribute = attribute.map_err(|_| S3BucketConfigurationXmlError::MalformedXml)?;
        let key = std::str::from_utf8(attribute.key.as_ref())
            .map_err(|_| S3BucketConfigurationXmlError::MalformedXml)?
            .to_owned();
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|_| S3BucketConfigurationXmlError::MalformedXml)?
            .into_owned();
        attributes.push((key, value));
    }
    Ok(S3ConfigurationXmlElement {
        name,
        attributes,
        text: String::new(),
        children: Vec::new(),
    })
}

fn attach_s3_configuration_xml_element(
    stack: &mut [S3ConfigurationXmlElement],
    root: &mut Option<S3ConfigurationXmlElement>,
    element: S3ConfigurationXmlElement,
) -> Result<(), S3BucketConfigurationXmlError> {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(element);
    } else if root.replace(element).is_some() {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    }
    Ok(())
}

fn append_s3_configuration_xml_text(
    stack: &mut [S3ConfigurationXmlElement],
    value: &str,
) -> Result<(), S3BucketConfigurationXmlError> {
    if let Some(element) = stack.last_mut() {
        element.text.push_str(value);
        Ok(())
    } else if value.trim().is_empty() {
        Ok(())
    } else {
        Err(S3BucketConfigurationXmlError::MalformedXml)
    }
}

fn validate_s3_configuration_root(
    element: &S3ConfigurationXmlElement,
    expected_name: &str,
) -> Result<(), S3BucketConfigurationXmlError> {
    if element.name != expected_name || !element.text.trim().is_empty() {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    }
    match element.attributes.as_slice() {
        [] => Ok(()),
        [(name, value)] if name == "xmlns" && value == S3_XML_NAMESPACE => Ok(()),
        _ => Err(S3BucketConfigurationXmlError::MalformedXml),
    }
}

fn validate_s3_container(
    element: &S3ConfigurationXmlElement,
) -> Result<(), S3BucketConfigurationXmlError> {
    if element.attributes.is_empty() && element.text.trim().is_empty() {
        Ok(())
    } else {
        Err(S3BucketConfigurationXmlError::MalformedXml)
    }
}

fn s3_leaf_text(
    element: &S3ConfigurationXmlElement,
) -> Result<&str, S3BucketConfigurationXmlError> {
    if element.attributes.is_empty() && element.children.is_empty() {
        Ok(element.text.trim())
    } else {
        Err(S3BucketConfigurationXmlError::MalformedXml)
    }
}

fn required_s3_leaf_text(
    element: &S3ConfigurationXmlElement,
) -> Result<&str, S3BucketConfigurationXmlError> {
    let value = s3_leaf_text(element)?;
    if value.is_empty() {
        Err(S3BucketConfigurationXmlError::MalformedXml)
    } else {
        Ok(value)
    }
}
