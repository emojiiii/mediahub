// S3 Bucket Object Lock protocol and persistence boundary.

fn parse_s3_create_bucket_object_lock_header(
    headers: &HeaderMap,
    uri: &Uri,
    request_id: &str,
) -> Result<bool, S3ApiError> {
    const HEADER: &str = "x-amz-bucket-object-lock-enabled";
    let values = headers.get_all(HEADER).iter().collect::<Vec<_>>();
    match values.as_slice() {
        [] => Ok(false),
        [value] if value.as_bytes() == b"true" => Ok(true),
        [_] => Err(S3ApiError::invalid_argument(
            "x-amz-bucket-object-lock-enabled must be true when present.",
            uri.path(),
            request_id,
        )),
        _ => Err(S3ApiError::invalid_request(
            "x-amz-bucket-object-lock-enabled must occur exactly once.",
            uri.path(),
            request_id,
        )),
    }
}

async fn s3_get_bucket_object_lock(
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
            warn!(error = %error, "S3 GetObjectLockConfiguration lookup failed");
            S3ApiError::service_unavailable(uri.path(), &request_id.0.0)
        })?
        .ok_or_else(|| S3ApiError::no_such_bucket(uri.path(), &request_id.0.0))?;
    if !configuration.object_lock_enabled() {
        return Err(S3ApiError::object_lock_configuration_not_found(
            uri.path(),
            &request_id.0.0,
        ));
    }
    Ok(s3_xml_response(
        StatusCode::OK,
        s3_bucket_object_lock_xml(configuration.default_retention()),
        &request_id.0.0,
    ))
}

async fn s3_put_bucket_object_lock(
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
    let default_retention = parse_s3_bucket_object_lock_xml(&content).map_err(|error| match error {
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
        .replace_s3_bucket_object_lock(
            auth.application.id,
            &bucket_name,
            default_retention,
            OffsetDateTime::now_utc(),
        )
        .await
        .map_err(|error| {
            map_s3_configuration_repository_error(error, uri.path(), &request_id.0.0)
        })?;
    Ok(s3_empty_response(StatusCode::OK, &request_id.0.0))
}

fn parse_s3_bucket_object_lock_xml(
    input: &[u8],
) -> Result<Option<DefaultRetention>, S3BucketConfigurationXmlError> {
    let root = parse_s3_configuration_xml(input)?;
    validate_s3_configuration_root(&root, "ObjectLockConfiguration")?;
    let (enabled, rule) = match root.children.as_slice() {
        [enabled] => (enabled, None),
        [enabled, rule] => (enabled, Some(rule)),
        _ => return Err(S3BucketConfigurationXmlError::MalformedXml),
    };
    if enabled.name != "ObjectLockEnabled" {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    }
    if required_s3_leaf_text(enabled)? != "Enabled" {
        return Err(S3BucketConfigurationXmlError::Invalid(
            "ObjectLockEnabled must be Enabled.".to_owned(),
        ));
    }
    rule.map(parse_s3_object_lock_rule).transpose()
}

fn parse_s3_object_lock_rule(
    rule: &S3ConfigurationXmlElement,
) -> Result<DefaultRetention, S3BucketConfigurationXmlError> {
    if rule.name != "Rule" {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    }
    validate_s3_container(rule)?;
    let [retention] = rule.children.as_slice() else {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    };
    if retention.name != "DefaultRetention" {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    }
    validate_s3_container(retention)?;
    let [mode, period] = retention.children.as_slice() else {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    };
    if mode.name != "Mode" {
        return Err(S3BucketConfigurationXmlError::MalformedXml);
    }
    let mode = match required_s3_leaf_text(mode)? {
        "GOVERNANCE" => RetentionMode::Governance,
        "COMPLIANCE" => RetentionMode::Compliance,
        _ => {
            return Err(S3BucketConfigurationXmlError::Invalid(
                "DefaultRetention Mode must be GOVERNANCE or COMPLIANCE.".to_owned(),
            ));
        }
    };
    let period = match period.name.as_str() {
        "Days" => DefaultRetentionPeriod::Days(parse_s3_positive_u32(
            period,
            "DefaultRetention Days",
        )?),
        "Years" => DefaultRetentionPeriod::Years(parse_s3_positive_u32(
            period,
            "DefaultRetention Years",
        )?),
        _ => return Err(S3BucketConfigurationXmlError::MalformedXml),
    };
    DefaultRetention::new(mode, period).map_err(|error| {
        S3BucketConfigurationXmlError::Invalid(format!(
            "DefaultRetention violates domain invariants: {error}"
        ))
    })
}

fn s3_bucket_object_lock_xml(default_retention: Option<DefaultRetention>) -> String {
    let mut xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><ObjectLockConfiguration xmlns=\"{S3_XML_NAMESPACE}\"><ObjectLockEnabled>Enabled</ObjectLockEnabled>"
    );
    if let Some(retention) = default_retention {
        xml.push_str("<Rule><DefaultRetention><Mode>");
        xml.push_str(match retention.mode() {
            RetentionMode::Governance => "GOVERNANCE",
            RetentionMode::Compliance => "COMPLIANCE",
        });
        xml.push_str("</Mode>");
        match retention.period() {
            DefaultRetentionPeriod::Days(days) => {
                xml.push_str("<Days>");
                xml.push_str(&days.to_string());
                xml.push_str("</Days>");
            }
            DefaultRetentionPeriod::Years(years) => {
                xml.push_str("<Years>");
                xml.push_str(&years.to_string());
                xml.push_str("</Years>");
            }
        }
        xml.push_str("</DefaultRetention></Rule>");
    }
    xml.push_str("</ObjectLockConfiguration>");
    xml
}
