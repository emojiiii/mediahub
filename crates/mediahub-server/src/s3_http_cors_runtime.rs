// Runtime S3 CORS preflight handling and response decoration.

pub(super) async fn s3_options_bucket(
    State(state): State<Arc<AppState>>,
    Path(bucket_name): Path<String>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    s3_options_for_bucket(&state, &bucket_name, &headers, &request_id.0.0).await
}

pub(super) async fn s3_options_object(
    State(state): State<Arc<AppState>>,
    Path((bucket_name, _object_key)): Path<(String, String)>,
    headers: HeaderMap,
    request_id: Extension<RequestId>,
) -> Result<Response, S3ApiError> {
    s3_options_for_bucket(&state, &bucket_name, &headers, &request_id.0.0).await
}

async fn s3_options_for_bucket(
    state: &AppState,
    bucket_name: &str,
    headers: &HeaderMap,
    request_id: &str,
) -> Result<Response, S3ApiError> {
    validate_s3_bucket_name(bucket_name)
        .map_err(|()| S3ApiError::invalid_bucket_name(bucket_name, request_id))?;
    let origin = required_single_s3_cors_header(headers, "origin", bucket_name, request_id)?;
    let requested_method = required_single_s3_cors_header(
        headers,
        "access-control-request-method",
        bucket_name,
        request_id,
    )?;
    let requested_method = parse_s3_cors_method(requested_method).ok_or_else(|| {
        S3ApiError::invalid_argument(
            "Access-Control-Request-Method is not a supported S3 method.",
            bucket_name,
            request_id,
        )
    })?;
    let requested_headers = parse_s3_cors_requested_headers(headers, bucket_name, request_id)?;
    let snapshot = mediahub_app::S3BucketCorsRepository::get_s3_bucket_cors(
        &state.repository,
        resolve_s3_cors_bucket_application(state, bucket_name, request_id).await?,
        bucket_name,
    )
    .await
    .map_err(|error| {
        warn!(error = %error, bucket_name, "S3 CORS preflight lookup failed");
        S3ApiError::service_unavailable(bucket_name, request_id)
    })?
    .ok_or_else(|| S3ApiError::no_such_bucket(bucket_name, request_id))?;
    let configuration = snapshot.configuration.as_ref().ok_or_else(|| {
        s3_cors_access_forbidden(
            "CORS is not enabled for this bucket.",
            bucket_name,
            request_id,
        )
    })?;
    let requested_header_refs = requested_headers
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    let decision = crate::s3_cors_runtime::evaluate_s3_cors_preflight(
        configuration,
        origin,
        requested_method,
        &requested_header_refs,
    )
    .ok_or_else(|| {
        s3_cors_access_forbidden(
            "This CORS request is not allowed.",
            bucket_name,
            request_id,
        )
    })?;
    let cors_headers = build_s3_cors_decision_headers(&decision, true, bucket_name, request_id)?;
    let mut response = s3_empty_response(StatusCode::OK, request_id);
    response.headers_mut().extend(cors_headers);
    merge_s3_cors_vary(response.headers_mut(), true);
    Ok(response)
}

pub(super) async fn s3_cors_response_middleware(
    State(state): State<Arc<AppState>>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    let Some(origin) = single_s3_cors_header(request.headers(), "origin") else {
        return next.run(request).await;
    };
    let Some(method) = parse_s3_cors_method(request.method().as_str()) else {
        return next.run(request).await;
    };
    let Some(bucket_name) = s3_cors_bucket_from_path(request.uri().path()).map(str::to_owned) else {
        return next.run(request).await;
    };
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .map(|value| value.0.clone())
        .unwrap_or_else(|| "unknown-request-id".to_owned());
    let decision = match load_s3_cors_decision(
        &state,
        &bucket_name,
        origin,
        method,
        &request_id,
    )
    .await
    {
        Ok(decision) => decision,
        Err(error) => return error.into_response(),
    };
    let cors_headers = match decision {
        Some(decision) => {
            match build_s3_cors_decision_headers(&decision, false, &bucket_name, &request_id) {
                Ok(headers) => Some(headers),
                Err(error) => return error.into_response(),
            }
        }
        None => None,
    };
    let mut response = next.run(request).await;
    if let Some(cors_headers) = cors_headers {
        response.headers_mut().extend(cors_headers);
        merge_s3_cors_vary(response.headers_mut(), false);
    }
    response
}

async fn load_s3_cors_decision(
    state: &AppState,
    bucket_name: &str,
    origin: &str,
    method: mediahub_core::S3CorsMethod,
    request_id: &str,
) -> Result<Option<crate::s3_cors_runtime::S3CorsDecision>, S3ApiError> {
    let Some(identity) = state
        .repository
        .resolve_s3_bucket_identity(bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, bucket_name, "S3 CORS bucket lookup failed");
            S3ApiError::service_unavailable(bucket_name, request_id)
        })?
    else {
        return Ok(None);
    };
    let Some(snapshot) = mediahub_app::S3BucketCorsRepository::get_s3_bucket_cors(
        &state.repository,
        identity.application_id,
        bucket_name,
    )
    .await
    .map_err(|error| {
        warn!(error = %error, bucket_name, "S3 CORS configuration lookup failed");
        S3ApiError::service_unavailable(bucket_name, request_id)
    })?
    else {
        return Ok(None);
    };
    Ok(snapshot.configuration.as_ref().and_then(|configuration| {
        crate::s3_cors_runtime::evaluate_s3_cors_actual_request(configuration, origin, method)
    }))
}

async fn resolve_s3_cors_bucket_application(
    state: &AppState,
    bucket_name: &str,
    request_id: &str,
) -> Result<ApplicationId, S3ApiError> {
    state
        .repository
        .resolve_s3_bucket_identity(bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, bucket_name, "S3 CORS bucket lookup failed");
            S3ApiError::service_unavailable(bucket_name, request_id)
        })?
        .map(|identity| identity.application_id)
        .ok_or_else(|| S3ApiError::no_such_bucket(bucket_name, request_id))
}

fn s3_cors_bucket_from_path(path: &str) -> Option<&str> {
    path.strip_prefix('/')?
        .split('/')
        .next()
        .filter(|bucket| validate_s3_bucket_name(bucket).is_ok())
}

fn parse_s3_cors_method(value: &str) -> Option<mediahub_core::S3CorsMethod> {
    match value {
        "GET" => Some(mediahub_core::S3CorsMethod::Get),
        "PUT" => Some(mediahub_core::S3CorsMethod::Put),
        "HEAD" => Some(mediahub_core::S3CorsMethod::Head),
        "POST" => Some(mediahub_core::S3CorsMethod::Post),
        "DELETE" => Some(mediahub_core::S3CorsMethod::Delete),
        _ => None,
    }
}

fn required_single_s3_cors_header<'a>(
    headers: &'a HeaderMap,
    name: &'static str,
    resource: &str,
    request_id: &str,
) -> Result<&'a str, S3ApiError> {
    single_s3_cors_header(headers, name).ok_or_else(|| {
        S3ApiError::invalid_argument(
            format!("{name} must occur exactly once and contain a valid value."),
            resource,
            request_id,
        )
    })
}

fn single_s3_cors_header<'a>(headers: &'a HeaderMap, name: &'static str) -> Option<&'a str> {
    let values = headers.get_all(name).iter().collect::<Vec<_>>();
    match values.as_slice() {
        [value] => value.to_str().ok().filter(|value| !value.is_empty()),
        _ => None,
    }
}

fn parse_s3_cors_requested_headers(
    headers: &HeaderMap,
    resource: &str,
    request_id: &str,
) -> Result<Vec<String>, S3ApiError> {
    let mut requested = Vec::new();
    for value in headers.get_all("access-control-request-headers") {
        let value = value.to_str().map_err(|_| {
            S3ApiError::invalid_argument(
                "Access-Control-Request-Headers is invalid.",
                resource,
                request_id,
            )
        })?;
        for name in value.split(',').map(str::trim) {
            if name.is_empty() || HeaderName::from_bytes(name.as_bytes()).is_err() {
                return Err(S3ApiError::invalid_argument(
                    "Access-Control-Request-Headers contains an invalid header name.",
                    resource,
                    request_id,
                ));
            }
            requested.push(name.to_owned());
        }
    }
    Ok(requested)
}

fn build_s3_cors_decision_headers(
    decision: &crate::s3_cors_runtime::S3CorsDecision,
    preflight: bool,
    resource: &str,
    request_id: &str,
) -> Result<HeaderMap, S3ApiError> {
    let mut headers = HeaderMap::new();
    insert_s3_cors_header(
        &mut headers,
        "access-control-allow-origin",
        &decision.allow_origin,
        resource,
        request_id,
    )?;
    if preflight {
        let methods = decision
            .allow_methods
            .iter()
            .map(|method| method.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        insert_s3_cors_header(
            &mut headers,
            "access-control-allow-methods",
            &methods,
            resource,
            request_id,
        )?;
        if !decision.allow_headers.is_empty() {
            insert_s3_cors_header(
                &mut headers,
                "access-control-allow-headers",
                &decision.allow_headers.join(", "),
                resource,
                request_id,
            )?;
        }
        if let Some(max_age) = decision.max_age_seconds {
            insert_s3_cors_header(
                &mut headers,
                "access-control-max-age",
                &max_age.to_string(),
                resource,
                request_id,
            )?;
        }
    }
    if !decision.expose_headers.is_empty() {
        insert_s3_cors_header(
            &mut headers,
            "access-control-expose-headers",
            &decision.expose_headers.join(", "),
            resource,
            request_id,
        )?;
    }
    Ok(headers)
}

fn merge_s3_cors_vary(headers: &mut HeaderMap, preflight: bool) {
    let required: &[&str] = if preflight {
        &[
            "Origin",
            "Access-Control-Request-Method",
            "Access-Control-Request-Headers",
        ]
    } else {
        &["Origin"]
    };
    let existing = headers
        .get_all("vary")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
        .collect::<std::collections::HashSet<_>>();
    let missing = required
        .iter()
        .copied()
        .filter(|name| !existing.contains(&name.to_ascii_lowercase()))
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        headers.append(
            HeaderName::from_static("vary"),
            HeaderValue::from_str(&missing.join(", "))
                .expect("static S3 CORS Vary names are valid header values"),
        );
    }
}

fn insert_s3_cors_header(
    headers: &mut HeaderMap,
    name: &'static str,
    value: &str,
    resource: &str,
    request_id: &str,
) -> Result<(), S3ApiError> {
    let value = HeaderValue::from_str(value)
        .map_err(|_| S3ApiError::internal_error(resource, request_id))?;
    headers.insert(HeaderName::from_static(name), value);
    Ok(())
}

fn s3_cors_access_forbidden(
    message: &str,
    resource: &str,
    request_id: &str,
) -> S3ApiError {
    S3ApiError::new(
        StatusCode::FORBIDDEN,
        "AccessForbidden",
        message,
        resource,
        request_id,
    )
}

#[cfg(test)]
mod s3_cors_http_tests {
    use super::*;

    #[test]
    fn parses_methods_and_requested_headers_strictly() {
        assert_eq!(parse_s3_cors_method("GET"), Some(mediahub_core::S3CorsMethod::Get));
        assert_eq!(parse_s3_cors_method("get"), None);
        let mut headers = HeaderMap::new();
        headers.append(
            "access-control-request-headers",
            HeaderValue::from_static("Content-Type, X-Amz-Date"),
        );
        headers.append(
            "access-control-request-headers",
            HeaderValue::from_static("X-Custom"),
        );
        assert_eq!(
            parse_s3_cors_requested_headers(&headers, "/bucket", "request").expect("headers"),
            ["Content-Type", "X-Amz-Date", "X-Custom"]
        );
    }

    #[test]
    fn extracts_only_valid_path_style_bucket_names() {
        assert_eq!(s3_cors_bucket_from_path("/assets/key"), Some("assets"));
        assert_eq!(s3_cors_bucket_from_path("/"), None);
        assert_eq!(s3_cors_bucket_from_path("/BadBucket/key"), None);
    }

    #[test]
    fn vary_headers_are_preserved_and_extended_by_request_shape() {
        let mut actual = HeaderMap::new();
        actual.insert("vary", HeaderValue::from_static("Accept-Encoding"));
        merge_s3_cors_vary(&mut actual, false);
        let actual = actual
            .get_all("vary")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>()
            .join(", ");
        assert!(actual.contains("Accept-Encoding"));
        assert!(actual.contains("Origin"));
        assert!(!actual.contains("Access-Control-Request-Method"));

        let mut preflight = HeaderMap::new();
        merge_s3_cors_vary(&mut preflight, true);
        merge_s3_cors_vary(&mut preflight, true);
        let preflight = preflight
            .get_all("vary")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .collect::<Vec<_>>()
            .join(", ");
        assert_eq!(preflight.matches("Origin").count(), 1);
        assert_eq!(preflight.matches("Access-Control-Request-Method").count(), 1);
        assert_eq!(preflight.matches("Access-Control-Request-Headers").count(), 1);
    }
}
