// Unified S3 data-plane authentication and policy authorization.

const S3_PRESIGN_AUTH_QUERY_PARAMETERS: [&str; 7] = [
    "X-Amz-Algorithm",
    "X-Amz-Credential",
    "X-Amz-Date",
    "X-Amz-Expires",
    "X-Amz-SignedHeaders",
    "X-Amz-Signature",
    "X-Amz-Security-Token",
];

#[derive(Clone, Copy, Debug)]
struct S3DataAuthorizationInput<'a> {
    action: S3PolicyAction,
    bucket_name: &'a str,
    object_key: Option<&'a str>,
    version_id: Option<&'a str>,
    prefix: Option<&'a str>,
    delimiter: Option<&'a str>,
    max_keys: Option<u64>,
    secure_transport: bool,
    source_ip: Option<std::net::IpAddr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct S3AuthorizedDataRequest {
    bucket: S3BucketIdentity,
}

impl S3AuthorizedDataRequest {
    const fn application_id(&self) -> ApplicationId {
        self.bucket.application_id
    }
}

#[derive(Clone, Debug)]
enum S3SignedPrincipalResolution {
    Principal(S3SignedPrincipal),
    FailClosed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum S3ObjectGetOperation {
    PlainRead,
    Tagging,
    VersionLock(S3ObjectVersionLockOperation),
    Acl,
    ListParts(String),
    UnsupportedSubresource(String),
}

fn classify_s3_object_get(
    uri: &Uri,
    request_id: &str,
) -> Result<S3ObjectGetOperation, S3ApiError> {
    let tagging = classify_s3_object_tagging(uri, request_id)?;
    let version_lock = classify_s3_object_version_lock(uri, request_id)?;
    let acl = s3_query_flag(uri, "acl", request_id)?;
    let upload_id = s3_query_value(uri, "uploadId", request_id)?;
    let mut unsupported = None;
    for name in ["torrent", "select", "select-type", "restore", "attributes", "uploads"] {
        if s3_query_value(uri, name, request_id)?.is_some() {
            if unsupported.is_some() {
                return Err(S3ApiError::invalid_request(
                    "Object subresources cannot be combined.",
                    uri.path(),
                    request_id,
                ));
            }
            unsupported = Some(name.to_owned());
        }
    }
    let operation_count = usize::from(tagging)
        + usize::from(version_lock.is_some())
        + usize::from(acl)
        + usize::from(upload_id.is_some())
        + usize::from(unsupported.is_some());
    if operation_count > 1 {
        return Err(S3ApiError::invalid_request(
            "Object subresources cannot be combined.",
            uri.path(),
            request_id,
        ));
    }
    Ok(if tagging {
        S3ObjectGetOperation::Tagging
    } else if let Some(operation) = version_lock {
        S3ObjectGetOperation::VersionLock(operation)
    } else if acl {
        S3ObjectGetOperation::Acl
    } else if let Some(upload_id) = upload_id {
        S3ObjectGetOperation::ListParts(upload_id)
    } else if let Some(name) = unsupported {
        S3ObjectGetOperation::UnsupportedSubresource(name)
    } else {
        S3ObjectGetOperation::PlainRead
    })
}

/// Anonymous means exactly that the request carried no SigV4 material. A
/// partial header or query credential set is a signed attempt and is always
/// handed to `ParsedSigV4`, which returns the protocol authentication error.
fn s3_has_authentication_material(headers: &HeaderMap, uri: &Uri) -> bool {
    headers.contains_key(axum::http::header::AUTHORIZATION)
        || headers.contains_key("x-amz-date")
        || headers.contains_key("x-amz-security-token")
        || url::form_urlencoded::parse(uri.query().unwrap_or_default().as_bytes()).any(
            |(name, _)| {
                S3_PRESIGN_AUTH_QUERY_PARAMETERS
                    .iter()
                    .any(|parameter| name.eq_ignore_ascii_case(parameter))
            },
        )
}

fn s3_data_secure_transport(uri: &Uri) -> bool {
    uri.scheme_str()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("https"))
}

fn s3_data_source_ip(
    connect_info: Option<&Extension<ConnectInfo<SocketAddr>>>,
) -> Option<std::net::IpAddr> {
    connect_info.map(|Extension(ConnectInfo(address))| address.ip())
}

async fn authorize_s3_data_request(
    state: &AppState,
    method: &Method,
    uri: &Uri,
    headers: &HeaderMap,
    input: S3DataAuthorizationInput<'_>,
    request_id: &str,
) -> Result<S3AuthorizedDataRequest, S3ApiError> {
    let principal = if s3_has_authentication_material(headers, uri) {
        let auth = authenticate_s3_application(state, method, uri, headers, &[], request_id).await?;
        return authorize_s3_signed_data_request(
            state,
            &auth,
            input,
            uri.path(),
            request_id,
        )
        .await;
    } else {
        S3AuthorizationPrincipal::Anonymous
    };

    authorize_s3_policy_principal_request(state, input, principal, uri.path(), request_id).await
}

/// Evaluates a request whose SigV4 signature has already been verified.
///
/// Buffered XML handlers and streaming upload handlers must authenticate the
/// exact body shape first, then call this helper. It deliberately receives an
/// `ApplicationAuth` instead of headers so a caller cannot accidentally
/// verify a payload twice or replace Policy with legacy permissions.
async fn authorize_s3_signed_data_request(
    state: &AppState,
    auth: &ApplicationAuth,
    input: S3DataAuthorizationInput<'_>,
    resource: &str,
    request_id: &str,
) -> Result<S3AuthorizedDataRequest, S3ApiError> {
    let principal = match load_s3_signed_policy_principal(state, auth, input, resource, request_id)
        .await?
    {
        S3SignedPrincipalResolution::Principal(principal) => {
            S3AuthorizationPrincipal::Signed(principal)
        }
        S3SignedPrincipalResolution::FailClosed => {
            return s3_fail_closed_for_existing_bucket(
                state,
                input.bucket_name,
                resource,
                request_id,
            )
            .await;
        }
    };
    authorize_s3_policy_principal_request(state, input, principal, resource, request_id).await
}

async fn authorize_s3_policy_principal_request(
    state: &AppState,
    input: S3DataAuthorizationInput<'_>,
    principal: S3AuthorizationPrincipal,
    resource: &str,
    request_id: &str,
) -> Result<S3AuthorizedDataRequest, S3ApiError> {

    let outcome = S3AuthorizationService::new(state.repository.clone())
        .authorize(&S3AuthorizationRequest {
            action: input.action,
            bucket_name: input.bucket_name.to_owned(),
            object_key: input.object_key.map(str::to_owned),
            version_id: input.version_id.map(str::to_owned),
            principal,
            secure_transport: input.secure_transport,
            source_ip: input.source_ip,
            prefix: input.prefix.map(str::to_owned),
            delimiter: input.delimiter.map(str::to_owned),
            max_keys: input.max_keys,
        })
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 data policy authorization failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?;

    match outcome {
        S3AuthorizationOutcome::BucketNotFound => {
            Err(S3ApiError::no_such_bucket(resource, request_id))
        }
        S3AuthorizationOutcome::Allowed { bucket } => Ok(S3AuthorizedDataRequest { bucket }),
        S3AuthorizationOutcome::ExplicitDeny { .. }
        | S3AuthorizationOutcome::ImplicitDeny { .. } => Err(S3ApiError::access_denied(
            "Access Denied.",
            resource,
            request_id,
        )),
    }
}

async fn load_s3_signed_policy_principal(
    state: &AppState,
    auth: &ApplicationAuth,
    input: S3DataAuthorizationInput<'_>,
    resource: &str,
    request_id: &str,
) -> Result<S3SignedPrincipalResolution, S3ApiError> {
    // `authenticate_s3_application` creates this identity only after a valid
    // SigV4 signature. Its legacy permissions are deliberately never read.
    let Some(hmac_identity) = auth.hmac_identity.as_ref() else {
        warn!(actor_id = %auth.actor_id, "verified S3 request has no access-key identity");
        return Ok(S3SignedPrincipalResolution::FailClosed);
    };
    let access_key_id = hmac_identity.access_key_id.as_str();
    let snapshot = match state.repository.get_s3_identity_policy(access_key_id).await {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => {
            warn!(access_key_id, "verified S3 access key has no identity snapshot");
            return Ok(S3SignedPrincipalResolution::FailClosed);
        }
        Err(RepositoryError::Invariant(error)) => {
            warn!(access_key_id, %error, "invalid persisted S3 identity policy state");
            return Ok(S3SignedPrincipalResolution::FailClosed);
        }
        Err(error) => {
            warn!(access_key_id, %error, "S3 identity policy lookup failed");
            return Err(S3ApiError::service_unavailable(resource, request_id));
        }
    };
    if snapshot.identity.application_id != auth.application.id
        || snapshot.identity.application_id != hmac_identity.application_id
        || snapshot.identity.access_key_id != access_key_id
    {
        warn!(access_key_id, "S3 identity policy lookup returned a different caller identity");
        return Ok(S3SignedPrincipalResolution::FailClosed);
    }

    let caller_account_id = snapshot.identity.account_id;
    let principal_arn = format!(
        "arn:aws:iam::{}:user/{access_key_id}",
        caller_account_id.as_str()
    );
    let identity_decision = snapshot.policy.as_ref().map_or(
        S3PolicyDecision::ImplicitDeny,
        |document| {
            let Ok(policy) = S3IdentityPolicy::parse(document.stable_json().as_bytes()) else {
                warn!(access_key_id, "persisted S3 identity policy failed strict runtime parsing");
                return S3PolicyDecision::ImplicitDeny;
            };
            let Some(request) = s3_identity_policy_request(
                input,
                &principal_arn,
                caller_account_id.as_str(),
                access_key_id,
            ) else {
                return S3PolicyDecision::ImplicitDeny;
            };
            policy.evaluate(&request)
        },
    );
    let principal = match S3SignedPrincipal::new(
        caller_account_id,
        principal_arn,
        access_key_id,
        identity_decision,
    ) {
        Ok(principal) => principal,
        Err(error) => {
            warn!(access_key_id, %error, "S3 signed principal validation failed");
            return Ok(S3SignedPrincipalResolution::FailClosed);
        }
    };
    Ok(S3SignedPrincipalResolution::Principal(principal))
}

fn s3_identity_policy_request<'a>(
    input: S3DataAuthorizationInput<'a>,
    principal_arn: &'a str,
    principal_account: &'a str,
    userid: &'a str,
) -> Option<S3IdentityPolicyRequest<'a>> {
    let mut request = match input.action.scope() {
        S3PolicyResourceScope::Bucket if input.object_key.is_none() => {
            S3IdentityPolicyRequest::bucket(input.action, input.bucket_name)
        }
        S3PolicyResourceScope::Object => S3IdentityPolicyRequest::object(
            input.action,
            input.bucket_name,
            input.object_key?,
        ),
        S3PolicyResourceScope::Account | S3PolicyResourceScope::Bucket => return None,
    };
    request.version_id = input.version_id;
    request.principal_arn = Some(principal_arn);
    request.principal_account = Some(principal_account);
    request.userid = Some(userid);
    request.secure_transport = input.secure_transport;
    request.source_ip = input.source_ip;
    request.query = S3PolicyQuery {
        prefix: input.prefix,
        delimiter: input.delimiter,
        max_keys: input.max_keys,
    };
    Some(request)
}

async fn s3_fail_closed_for_existing_bucket(
    state: &AppState,
    bucket_name: &str,
    resource: &str,
    request_id: &str,
) -> Result<S3AuthorizedDataRequest, S3ApiError> {
    let bucket = state
        .repository
        .resolve_s3_bucket_identity(bucket_name)
        .await
        .map_err(|error| {
            warn!(error = %error, "S3 fail-closed bucket lookup failed");
            S3ApiError::service_unavailable(resource, request_id)
        })?;
    match bucket {
        None => Err(S3ApiError::no_such_bucket(resource, request_id)),
        Some(_) => Err(S3ApiError::access_denied(
            "Access Denied.",
            resource,
            request_id,
        )),
    }
}
