// ObjectVersion retention and legal-hold protocol handling.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum S3ObjectVersionLockOperation {
    Retention,
    LegalHold,
}

fn classify_s3_object_version_lock(
    uri: &Uri,
    request_id: &str,
) -> Result<Option<S3ObjectVersionLockOperation>, S3ApiError> {
    let retention = s3_query_value(uri, "retention", request_id)?;
    let legal_hold = s3_query_value(uri, "legal-hold", request_id)?;
    for (name, value) in [
        ("retention", retention.as_deref()),
        ("legal-hold", legal_hold.as_deref()),
    ] {
        if value.is_some_and(|value| !value.is_empty()) {
            return Err(S3ApiError::invalid_argument(
                format!("{name} must be an empty S3 subresource parameter."),
                uri.path(),
                request_id,
            ));
        }
    }
    if retention.is_some() && legal_hold.is_some() {
        return Err(S3ApiError::invalid_request(
            "retention and legal-hold cannot be combined.",
            uri.path(),
            request_id,
        ));
    }
    let operation = if retention.is_some() {
        Some(S3ObjectVersionLockOperation::Retention)
    } else if legal_hold.is_some() {
        Some(S3ObjectVersionLockOperation::LegalHold)
    } else {
        None
    };
    if operation.is_some() {
        for name in ["acl", "uploadId", "partNumber", "uploads"] {
            if s3_query_value(uri, name, request_id)?.is_some() {
                return Err(S3ApiError::invalid_request(
                    "Object Lock subresources cannot be combined with another object operation.",
                    uri.path(),
                    request_id,
                ));
            }
        }
    }
    Ok(operation)
}

fn parse_put_object_lock_headers(
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &str,
    now: OffsetDateTime,
) -> Result<NewS3ObjectLock, S3ApiError> {
    let mode = single_s3_object_lock_header(headers, "x-amz-object-lock-mode", uri, request_id)?;
    let retain_until = single_s3_object_lock_header(
        headers,
        "x-amz-object-lock-retain-until-date",
        uri,
        request_id,
    )?;
    let retention = match (mode, retain_until) {
        (None, None) => None,
        (Some(mode), Some(retain_until)) => {
            let mode = parse_s3_retention_mode(mode, uri, request_id)?;
            let retain_until =
                OffsetDateTime::parse(retain_until, &time::format_description::well_known::Rfc3339)
                    .map_err(|_| {
                        S3ApiError::invalid_argument(
                            "x-amz-object-lock-retain-until-date must be an RFC 3339 timestamp.",
                            uri.path(),
                            request_id,
                        )
                    })?
                    .to_offset(time::UtcOffset::UTC);
            if retain_until <= now {
                return Err(S3ApiError::invalid_argument(
                    "x-amz-object-lock-retain-until-date must be in the future.",
                    uri.path(),
                    request_id,
                ));
            }
            Some(ObjectRetention::new(mode, retain_until))
        }
        _ => {
            return Err(S3ApiError::invalid_request(
                "x-amz-object-lock-mode and x-amz-object-lock-retain-until-date must both be supplied.",
                uri.path(),
                request_id,
            ));
        }
    };
    let legal_hold =
        single_s3_object_lock_header(headers, "x-amz-object-lock-legal-hold", uri, request_id)?
            .map(|value| match value {
                "ON" => Ok(true),
                "OFF" => Ok(false),
                _ => Err(S3ApiError::invalid_argument(
                    "x-amz-object-lock-legal-hold must be ON or OFF.",
                    uri.path(),
                    request_id,
                )),
            })
            .transpose()?;
    Ok(NewS3ObjectLock {
        retention,
        legal_hold,
    })
}

fn single_s3_object_lock_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
    uri: &Uri,
    request_id: &str,
) -> Result<Option<&'a str>, S3ApiError> {
    let values = headers.get_all(name).iter().collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(None),
        [value] => value.to_str().map(Some).map_err(|_| {
            S3ApiError::invalid_argument(format!("{name} is invalid."), uri.path(), request_id)
        }),
        _ => Err(S3ApiError::invalid_request(
            format!("{name} must occur exactly once."),
            uri.path(),
            request_id,
        )),
    }
}

fn parse_s3_retention_mode(
    value: &str,
    uri: &Uri,
    request_id: &str,
) -> Result<RetentionMode, S3ApiError> {
    match value {
        "GOVERNANCE" => Ok(RetentionMode::Governance),
        "COMPLIANCE" => Ok(RetentionMode::Compliance),
        _ => Err(S3ApiError::invalid_argument(
            "Object Lock mode must be GOVERNANCE or COMPLIANCE.",
            uri.path(),
            request_id,
        )),
    }
}

async fn ensure_put_object_lock_bucket(
    state: &AppState,
    application_id: ApplicationId,
    bucket_name: &str,
    object_lock: NewS3ObjectLock,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    if !object_lock.requested() {
        return Ok(());
    }
    let bucket = state
        .repository
        .find_s3_bucket(application_id, bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 PutObject Object Lock bucket lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(resource, request_id))?;
    if !bucket.configuration().object_lock_enabled() {
        return Err(S3ApiError::invalid_bucket_object_lock_configuration(
            resource, request_id,
        ));
    }
    Ok(())
}

fn reject_multipart_object_lock_headers(
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &str,
) -> Result<(), S3ApiError> {
    if has_s3_object_lock_headers(headers) {
        Err(S3ApiError::invalid_request(
            "This multipart operation does not accept Object Lock headers; configure them on PutObject or use the object retention/legal-hold subresources.",
            uri.path(),
            request_id,
        ))
    } else {
        Ok(())
    }
}

fn has_s3_object_lock_headers(headers: &HeaderMap) -> bool {
    const HEADERS: [&str; 4] = [
        "x-amz-object-lock-mode",
        "x-amz-object-lock-retain-until-date",
        "x-amz-object-lock-legal-hold",
        "x-amz-bypass-governance-retention",
    ];
    HEADERS.iter().any(|name| headers.contains_key(*name))
}

fn reject_copy_object_lock_headers(
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &str,
) -> Result<(), S3ApiError> {
    if has_s3_object_lock_headers(headers) {
        Err(S3ApiError::invalid_request(
            "CopyObject Object Lock destination headers are not supported yet.",
            uri.path(),
            request_id,
        ))
    } else {
        Ok(())
    }
}

async fn s3_get_object_version_lock(
    operation: S3ObjectOperation<'_>,
    lock_operation: S3ObjectVersionLockOperation,
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
    validate_s3_object_key(object_key, uri.path(), request_id)?;
    let version_id = parse_s3_version_id(uri, request_id)?;
    let policy_operation = match lock_operation {
        S3ObjectVersionLockOperation::Retention => S3ObjectSubresourcePolicyOperation::GetRetention,
        S3ObjectVersionLockOperation::LegalHold => S3ObjectSubresourcePolicyOperation::GetLegalHold,
    };
    let authorization = authorize_s3_object_subresource(
        state,
        auth,
        S3ObjectSubresourceAuthorizationRequest {
            operation: policy_operation,
            bucket_name,
            object_key,
            version_id: version_id.as_ref(),
            uri,
            source_ip,
            request_id,
        },
    )
    .await?;
    let request = S3ObjectRequest {
        application_id: authorization.application_id(),
        bucket_name: bucket_name.to_owned(),
        object_key: object_key.to_owned(),
        version_id,
    };
    let service = runtime_s3_object_service(state, uri.path(), request_id)?;
    let head = service
        .get_object_lock(&request)
        .await
        .map_err(|error| map_s3_object_service_error(error, uri.path(), request_id))?;
    let body = match lock_operation {
        S3ObjectVersionLockOperation::Retention => {
            let retention = head.version.retention().ok_or_else(|| {
                S3ApiError::no_such_object_lock_configuration(uri.path(), request_id)
            })?;
            s3_object_retention_xml(retention)?
        }
        S3ObjectVersionLockOperation::LegalHold => {
            s3_object_legal_hold_xml(head.version.legal_hold())
        }
    };
    let mut response = s3_xml_response(StatusCode::OK, body, request_id);
    insert_s3_version_id(response.headers_mut(), head.version.external_version_id())?;
    Ok(response)
}

async fn s3_put_object_version_lock(
    operation: S3ObjectOperation<'_>,
    lock_operation: S3ObjectVersionLockOperation,
    headers: &HeaderMap,
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
    validate_s3_object_key(object_key, uri.path(), request_id)?;
    let version_id = parse_s3_version_id(uri, request_id)?;
    let bypass_governance = match lock_operation {
        S3ObjectVersionLockOperation::Retention => {
            parse_s3_bypass_governance(headers, uri, request_id)?
        }
        S3ObjectVersionLockOperation::LegalHold => {
            if headers.contains_key("x-amz-bypass-governance-retention") {
                return Err(S3ApiError::invalid_request(
                    "x-amz-bypass-governance-retention is not valid for PutObjectLegalHold.",
                    uri.path(),
                    request_id,
                ));
            }
            false
        }
    };
    let policy_operation = match lock_operation {
        S3ObjectVersionLockOperation::Retention => S3ObjectSubresourcePolicyOperation::PutRetention,
        S3ObjectVersionLockOperation::LegalHold => S3ObjectSubresourcePolicyOperation::PutLegalHold,
    };
    let authorization = authorize_s3_object_subresource(
        state,
        auth,
        S3ObjectSubresourceAuthorizationRequest {
            operation: policy_operation,
            bucket_name,
            object_key,
            version_id: version_id.as_ref(),
            uri,
            source_ip,
            request_id,
        },
    )
    .await?;
    if bypass_governance {
        authorize_s3_object_subresource(
            state,
            auth,
            S3ObjectSubresourceAuthorizationRequest {
                operation: S3ObjectSubresourcePolicyOperation::BypassGovernanceRetention,
                bucket_name,
                object_key,
                version_id: version_id.as_ref(),
                uri,
                source_ip,
                request_id,
            },
        )
        .await?;
    }
    validate_s3_configuration_content_md5(headers, content, uri, request_id)?;
    let request = S3ObjectRequest {
        application_id: authorization.application_id(),
        bucket_name: bucket_name.to_owned(),
        object_key: object_key.to_owned(),
        version_id,
    };
    let service = runtime_s3_object_service(state, uri.path(), request_id)?;
    let (version, audit_action) = match lock_operation {
        S3ObjectVersionLockOperation::Retention => {
            let retention = parse_s3_object_retention_xml(content, uri, request_id)?;
            (
                service
                    .put_object_retention(&PutObjectRetentionRequest {
                        object: request,
                        retention,
                        bypass_governance,
                    })
                    .await,
                "s3.object_retention_updated",
            )
        }
        S3ObjectVersionLockOperation::LegalHold => {
            let legal_hold = parse_s3_object_legal_hold_xml(content, uri, request_id)?;
            (
                service
                    .put_object_legal_hold(&PutObjectLegalHoldRequest {
                        object: request,
                        legal_hold,
                    })
                    .await,
                "s3.object_legal_hold_updated",
            )
        }
    };
    let version =
        version.map_err(|error| map_s3_object_service_error(error, uri.path(), request_id))?;
    record_s3_resource_audit(
        state,
        auth,
        authorization.application_id(),
        request_id,
        audit_action,
        ("object_version", version.id().to_string()),
        serde_json::json!({
            "protocol": "s3",
            "bucket": bucket_name,
            "object_key": object_key,
            "version_id": version.external_version_id().as_str(),
        }),
    )
    .await;
    let mut response = s3_empty_response(StatusCode::OK, request_id);
    insert_s3_version_id(response.headers_mut(), version.external_version_id())?;
    Ok(response)
}

fn parse_s3_object_retention_xml(
    input: &[u8],
    uri: &Uri,
    request_id: &str,
) -> Result<ObjectRetention, S3ApiError> {
    let root = parse_s3_configuration_xml(input)
        .map_err(|_| S3ApiError::malformed_xml(uri.path(), request_id))?;
    validate_s3_configuration_root(&root, "Retention")
        .map_err(|_| S3ApiError::malformed_xml(uri.path(), request_id))?;
    let [mode, retain_until] = root.children.as_slice() else {
        return Err(S3ApiError::malformed_xml(uri.path(), request_id));
    };
    if mode.name != "Mode" || retain_until.name != "RetainUntilDate" {
        return Err(S3ApiError::malformed_xml(uri.path(), request_id));
    }
    let mode = parse_s3_retention_mode(
        required_s3_leaf_text(mode)
            .map_err(|_| S3ApiError::malformed_xml(uri.path(), request_id))?,
        uri,
        request_id,
    )?;
    let retain_until = OffsetDateTime::parse(
        required_s3_leaf_text(retain_until)
            .map_err(|_| S3ApiError::malformed_xml(uri.path(), request_id))?,
        &time::format_description::well_known::Rfc3339,
    )
    .map_err(|_| {
        S3ApiError::invalid_argument(
            "RetainUntilDate must be an RFC 3339 timestamp.",
            uri.path(),
            request_id,
        )
    })?
    .to_offset(time::UtcOffset::UTC);
    if retain_until <= OffsetDateTime::now_utc() {
        return Err(S3ApiError::invalid_argument(
            "RetainUntilDate must be in the future.",
            uri.path(),
            request_id,
        ));
    }
    Ok(ObjectRetention::new(mode, retain_until))
}

fn parse_s3_object_legal_hold_xml(
    input: &[u8],
    uri: &Uri,
    request_id: &str,
) -> Result<bool, S3ApiError> {
    let root = parse_s3_configuration_xml(input)
        .map_err(|_| S3ApiError::malformed_xml(uri.path(), request_id))?;
    validate_s3_configuration_root(&root, "LegalHold")
        .map_err(|_| S3ApiError::malformed_xml(uri.path(), request_id))?;
    let [status] = root.children.as_slice() else {
        return Err(S3ApiError::malformed_xml(uri.path(), request_id));
    };
    if status.name != "Status" {
        return Err(S3ApiError::malformed_xml(uri.path(), request_id));
    }
    match required_s3_leaf_text(status)
        .map_err(|_| S3ApiError::malformed_xml(uri.path(), request_id))?
    {
        "ON" => Ok(true),
        "OFF" => Ok(false),
        _ => Err(S3ApiError::invalid_argument(
            "LegalHold Status must be ON or OFF.",
            uri.path(),
            request_id,
        )),
    }
}

fn s3_object_retention_xml(retention: ObjectRetention) -> Result<String, S3ApiError> {
    let retain_until = retention
        .retain_until()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| S3ApiError::internal_error("retention", "response"))?;
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Retention xmlns=\"{S3_XML_NAMESPACE}\"><Mode>{}</Mode><RetainUntilDate>{retain_until}</RetainUntilDate></Retention>",
        match retention.mode() {
            RetentionMode::Governance => "GOVERNANCE",
            RetentionMode::Compliance => "COMPLIANCE",
        }
    ))
}

fn s3_object_legal_hold_xml(legal_hold: bool) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><LegalHold xmlns=\"{S3_XML_NAMESPACE}\"><Status>{}</Status></LegalHold>",
        if legal_hold { "ON" } else { "OFF" }
    )
}

fn insert_s3_object_lock_headers(
    headers: &mut HeaderMap,
    version: &ObjectVersion,
) -> Result<(), S3ApiError> {
    if let Some(retention) = version.retention() {
        headers.insert(
            HeaderName::from_static("x-amz-object-lock-mode"),
            HeaderValue::from_static(match retention.mode() {
                RetentionMode::Governance => "GOVERNANCE",
                RetentionMode::Compliance => "COMPLIANCE",
            }),
        );
        let retain_until = retention
            .retain_until()
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|_| S3ApiError::internal_error("retention", "response"))?;
        headers.insert(
            HeaderName::from_static("x-amz-object-lock-retain-until-date"),
            HeaderValue::from_str(&retain_until)
                .map_err(|_| S3ApiError::internal_error("retention", "response"))?,
        );
    }
    if version.legal_hold() {
        headers.insert(
            HeaderName::from_static("x-amz-object-lock-legal-hold"),
            HeaderValue::from_static("ON"),
        );
    }
    Ok(())
}
