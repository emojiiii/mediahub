use std::{sync::Arc, time::Duration};

use mediahub_app::{
    ApplicationRepository, AuthRepository, S3BucketPolicyRepository, S3BucketRepository,
};
use mediahub_core::{ApplicationId, BucketId, OffsetDateTime, S3Bucket, UserId};

use super::http_contract::{
    assert_s3_error, create_data_policy_identity, data_policy_test_state, install_bucket_policy,
    install_identity_policy,
};
use super::*;

struct PolicyTestAccount {
    application_id: ApplicationId,
    access_key_id: &'static str,
    secret: &'static str,
}

struct PolicyCall<'a> {
    method: Method,
    bucket_name: &'a str,
    subresource: &'a str,
    body: Vec<u8>,
    expected_owner: Option<&'a str>,
    content_md5: Option<String>,
}

impl<'a> PolicyCall<'a> {
    fn new(method: Method, bucket_name: &'a str, subresource: &'a str) -> Self {
        Self {
            method,
            bucket_name,
            subresource,
            body: Vec::new(),
            expected_owner: None,
            content_md5: None,
        }
    }

    fn with_body(mut self, body: Vec<u8>) -> Self {
        self.body = body;
        self
    }

    fn with_expected_owner(mut self, expected_owner: &'a str) -> Self {
        self.expected_owner = Some(expected_owner);
        self
    }

    fn with_content_md5(mut self, content_md5: String) -> Self {
        self.content_md5 = Some(content_md5);
        self
    }
}

async fn create_policy_test_account(
    state: &AppState,
    email: &str,
    access_key_id: &'static str,
    secret: &'static str,
) -> PolicyTestAccount {
    let now = OffsetDateTime::now_utc();
    let user_id = UserId::new();
    let application_id = ApplicationId::new();
    state
        .repository
        .create_user(user_id, email, "hashed", now)
        .await
        .expect("create Bucket Policy test user");
    state
        .repository
        .create_application(
            application_id,
            user_id,
            "S3 Bucket Policy Identity",
            &format!("app_{}", application_id.as_uuid().simple()),
            64 * 1024 * 1024,
            now,
        )
        .await
        .expect("create Bucket Policy test application");
    create_data_policy_identity(state, application_id, access_key_id, secret).await;
    PolicyTestAccount {
        application_id,
        access_key_id,
        secret,
    }
}

fn policy_identity_document(
    bucket_name: &str,
    allow: &[&str],
    deny: &[&str],
) -> String {
    let resource = format!("arn:aws:s3:::{bucket_name}");
    let mut statements = Vec::new();
    if !allow.is_empty() {
        statements.push(serde_json::json!({
            "Effect": "Allow",
            "Action": allow,
            "Resource": resource,
        }));
    }
    if !deny.is_empty() {
        statements.push(serde_json::json!({
            "Effect": "Deny",
            "Action": deny,
            "Resource": resource,
        }));
    }
    serde_json::json!({
        "Version": "2012-10-17",
        "Statement": statements,
    })
    .to_string()
}

fn sign_policy_request(
    request: &mut http::Request<Vec<u8>>,
    access_key_id: &str,
    secret: &str,
) {
    let identity = aws_credential_types::Credentials::new(
        access_key_id,
        secret,
        None,
        None,
        "prismark-s3-bucket-policy-identity-test",
    )
    .into();
    let mut settings = aws_sigv4::http_request::SigningSettings::default();
    settings.signature_location = aws_sigv4::http_request::SignatureLocation::Headers;
    settings.percent_encoding_mode = aws_sigv4::http_request::PercentEncodingMode::Single;
    settings.uri_path_normalization_mode =
        aws_sigv4::http_request::UriPathNormalizationMode::Disabled;
    settings.payload_checksum_kind = aws_sigv4::http_request::PayloadChecksumKind::NoHeader;
    let params = aws_sigv4::sign::v4::SigningParams::builder()
        .identity(&identity)
        .region("us-east-1")
        .name("s3")
        .time(std::time::SystemTime::now())
        .settings(settings)
        .build()
        .expect("Bucket Policy signing params")
        .into();
    let payload_sha256 = request
        .headers()
        .get("x-amz-content-sha256")
        .expect("precomputed payload hash")
        .to_str()
        .expect("payload hash header")
        .to_owned();
    let signing_uri = request
        .uri()
        .path_and_query()
        .map_or("/", http::uri::PathAndQuery::as_str);
    let signable = aws_sigv4::http_request::SignableRequest::new(
        request.method().as_str(),
        signing_uri,
        request.headers().iter().map(|(name, value)| {
            (
                name.as_str(),
                value.to_str().expect("Bucket Policy request header"),
            )
        }),
        aws_sigv4::http_request::SignableBody::Precomputed(payload_sha256),
    )
    .expect("Bucket Policy signable request");
    aws_sigv4::http_request::sign(signable, &params)
        .expect("Bucket Policy signature")
        .into_parts()
        .0
        .apply_to_request_http1x(request);
}

async fn send_policy_call(
    client: &reqwest::Client,
    address: std::net::SocketAddr,
    account: &PolicyTestAccount,
    call: PolicyCall<'_>,
) -> reqwest::Response {
    let uri = format!(
        "http://{address}/{}?{}",
        call.bucket_name, call.subresource
    );
    let payload_sha256 = hex::encode(Sha256::digest(&call.body));
    let mut builder = http::Request::builder()
        .method(call.method)
        .uri(uri)
        .header("host", address.to_string())
        .header("x-amz-content-sha256", payload_sha256);
    if let Some(expected_owner) = call.expected_owner {
        builder = builder.header(S3_EXPECTED_BUCKET_OWNER_HEADER, expected_owner);
    }
    if let Some(content_md5) = call.content_md5 {
        builder = builder.header("content-md5", content_md5);
    }
    if !call.body.is_empty() {
        builder = builder.header(CONTENT_LENGTH, call.body.len());
    }
    let mut request = builder
        .body(call.body)
        .expect("Bucket Policy HTTP request");
    sign_policy_request(&mut request, account.access_key_id, account.secret);
    let (parts, body) = request.into_parts();
    client
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .expect("send Bucket Policy HTTP request")
}

async fn assert_policy_document_unchanged(
    state: &AppState,
    bucket_name: &str,
    expected_sha256: &str,
) {
    let snapshot = state
        .repository
        .get_s3_bucket_policy(bucket_name)
        .await
        .expect("read Bucket Policy snapshot")
        .expect("Bucket Policy bucket");
    assert_eq!(
        snapshot
            .policy
            .as_ref()
            .expect("persisted Bucket Policy")
            .sha256(),
        expected_sha256
    );
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn bucket_policy_management_requires_owner_identity_actions_and_cannot_self_authorize(
    pool: sqlx::PgPool,
) {
    const ALL_POLICY_ACTIONS: [&str; 4] = [
        "s3:GetBucketPolicy",
        "s3:GetBucketPolicyStatus",
        "s3:PutBucketPolicy",
        "s3:DeleteBucketPolicy",
    ];

    let (state, storage_root) = data_policy_test_state(pool).await;
    let owner = create_policy_test_account(
        &state,
        "bucket-policy-owner@example.com",
        "mh_ak_bucket_policy_owner",
        "bucket-policy-owner-secret",
    )
    .await;
    let outsider = create_policy_test_account(
        &state,
        "bucket-policy-outsider@example.com",
        "mh_ak_bucket_policy_outsider",
        "bucket-policy-outsider-secret",
    )
    .await;
    let bucket_name = "identity-policy-control";
    state
        .repository
        .create_s3_bucket(
            &S3Bucket::new(
                BucketId::new(),
                owner.application_id,
                bucket_name,
                "us-east-1",
                false,
                None,
                OffsetDateTime::now_utc(),
            )
            .expect("Bucket Policy test bucket"),
        )
        .await
        .expect("persist Bucket Policy test bucket");
    let bucket_identity = state
        .repository
        .resolve_s3_bucket_identity(bucket_name)
        .await
        .expect("resolve Bucket Policy test bucket")
        .expect("Bucket Policy test bucket identity");
    let owner_account_id = bucket_identity.owner_account_id.as_str().to_owned();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Bucket Policy listener");
    let address = listener.local_addr().expect("Bucket Policy address");
    let server = tokio::spawn({
        let application = crate::s3_router::router(Arc::clone(&state));
        async move {
            axum::serve(
                listener,
                application.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .expect("Bucket Policy identity test server");
        }
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("Bucket Policy HTTP client");

    let self_authorizing_policy = serde_json::json!({
        "Version": "2012-10-17",
        "Statement": {
            "Effect": "Allow",
            "Principal": "*",
            "Action": ALL_POLICY_ACTIONS,
            "Resource": format!("arn:aws:s3:::{bucket_name}"),
        }
    });
    let self_authorizing_body = serde_json::to_vec(&self_authorizing_policy)
        .expect("serialize self-authorizing Bucket Policy");

    // Wire validation and strict parsing happen before owner identity
    // authorization, but a valid incoming policy is never used to authorize
    // its own PutBucketPolicy request.
    let malformed = br#"{"Version":"2012-10-17","Statement":[]}"#.to_vec();
    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &owner,
            PolicyCall::new(Method::PUT, bucket_name, "policy").with_body(malformed),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "MalformedPolicy",
    )
    .await;
    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &owner,
            PolicyCall::new(Method::PUT, bucket_name, "policy")
                .with_body(self_authorizing_body.clone())
                .with_content_md5("AAAAAAAAAAAAAAAAAAAAAA==".to_owned()),
        )
        .await,
        StatusCode::BAD_REQUEST,
        "BadDigest",
    )
    .await;
    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &owner,
            PolicyCall::new(Method::PUT, bucket_name, "policy")
                .with_body(self_authorizing_body.clone()),
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;
    assert!(
        state
            .repository
            .get_s3_bucket_policy(bucket_name)
            .await
            .expect("read empty Bucket Policy state")
            .expect("Bucket Policy bucket")
            .policy
            .is_none(),
        "the incoming policy must not persist before identity authorization"
    );
    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &owner,
            PolicyCall::new(Method::GET, "missing-policy-control", "policy"),
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;

    // A previously persisted Bucket Policy cannot authorize any management
    // operation either, even when it grants every policy action to everyone.
    install_bucket_policy(
        &state,
        owner.application_id,
        bucket_name,
        self_authorizing_policy,
    )
    .await;
    let existing_policy_sha256 = state
        .repository
        .get_s3_bucket_policy(bucket_name)
        .await
        .expect("read self-authorizing Bucket Policy")
        .expect("Bucket Policy bucket")
        .policy
        .expect("self-authorizing Bucket Policy")
        .sha256()
        .to_owned();
    for (method, subresource) in [
        (Method::GET, "policy"),
        (Method::GET, "policyStatus"),
        (Method::DELETE, "policy"),
    ] {
        assert_s3_error(
            send_policy_call(
                &client,
                address,
                &owner,
                PolicyCall::new(method, bucket_name, subresource),
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
    }
    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &owner,
            PolicyCall::new(Method::PUT, bucket_name, "policy")
                .with_body(self_authorizing_body.clone()),
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;
    assert_policy_document_unchanged(&state, bucket_name, &existing_policy_sha256).await;

    // Every endpoint asks for its own S3 action; legacy grants and a nearby
    // policy action never substitute for the requested action.
    install_identity_policy(
        &state,
        owner.application_id,
        owner.access_key_id,
        &policy_identity_document(bucket_name, &["s3:GetBucketPolicyStatus"], &[]),
    )
    .await;
    let status = send_policy_call(
        &client,
        address,
        &owner,
        PolicyCall::new(Method::GET, bucket_name, "policyStatus"),
    )
    .await;
    assert_eq!(status.status(), StatusCode::OK);
    for method in [Method::GET, Method::DELETE] {
        assert_s3_error(
            send_policy_call(
                &client,
                address,
                &owner,
                PolicyCall::new(method, bucket_name, "policy"),
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
    }
    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &owner,
            PolicyCall::new(Method::PUT, bucket_name, "policy")
                .with_body(self_authorizing_body.clone()),
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;

    install_identity_policy(
        &state,
        owner.application_id,
        owner.access_key_id,
        &policy_identity_document(bucket_name, &ALL_POLICY_ACTIONS, &[]),
    )
    .await;
    let get = send_policy_call(
        &client,
        address,
        &owner,
        PolicyCall::new(Method::GET, bucket_name, "policy"),
    )
    .await;
    assert_eq!(get.status(), StatusCode::OK);

    let replacement_policy = serde_json::json!({
        "Version": "2012-10-17",
        "Statement": {
            "Effect": "Allow",
            "Principal": "*",
            "Action": "s3:ListBucket",
            "Resource": format!("arn:aws:s3:::{bucket_name}"),
        }
    });
    let replacement_body = serde_json::to_vec(&replacement_policy)
        .expect("serialize replacement Bucket Policy");
    let replacement_md5 = STANDARD.encode(<md5::Md5 as md5::Digest>::digest(&replacement_body));
    let put = send_policy_call(
        &client,
        address,
        &owner,
        PolicyCall::new(Method::PUT, bucket_name, "policy")
            .with_body(replacement_body)
            .with_content_md5(replacement_md5),
    )
    .await;
    assert_eq!(put.status(), StatusCode::OK);

    install_identity_policy(
        &state,
        owner.application_id,
        owner.access_key_id,
        &policy_identity_document(
            bucket_name,
            &ALL_POLICY_ACTIONS,
            &["s3:DeleteBucketPolicy"],
        ),
    )
    .await;
    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &owner,
            PolicyCall::new(Method::DELETE, bucket_name, "policy"),
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;

    install_identity_policy(
        &state,
        owner.application_id,
        owner.access_key_id,
        &policy_identity_document(bucket_name, &ALL_POLICY_ACTIONS, &[]),
    )
    .await;
    let wrong_owner = if owner_account_id == "999999999999" {
        "888888888888"
    } else {
        "999999999999"
    };
    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &owner,
            PolicyCall::new(Method::GET, bucket_name, "policy")
                .with_expected_owner(wrong_owner),
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;

    // Cross-account callers receive the S3 management special-case 405 even
    // if both their Identity Policy and the Bucket Policy explicitly allow.
    install_identity_policy(
        &state,
        outsider.application_id,
        outsider.access_key_id,
        &policy_identity_document(bucket_name, &ALL_POLICY_ACTIONS, &[]),
    )
    .await;
    install_bucket_policy(
        &state,
        owner.application_id,
        bucket_name,
        serde_json::json!({
            "Version": "2012-10-17",
            "Statement": {
                "Effect": "Allow",
                "Principal": "*",
                "Action": ALL_POLICY_ACTIONS,
                "Resource": format!("arn:aws:s3:::{bucket_name}"),
            }
        }),
    )
    .await;
    for (method, subresource, body) in [
        (Method::GET, "policy", Vec::new()),
        (Method::GET, "policyStatus", Vec::new()),
        (Method::PUT, "policy", self_authorizing_body.clone()),
        (Method::DELETE, "policy", Vec::new()),
    ] {
        assert_s3_error(
            send_policy_call(
                &client,
                address,
                &outsider,
                PolicyCall::new(method, bucket_name, subresource).with_body(body),
            )
            .await,
            StatusCode::METHOD_NOT_ALLOWED,
            "MethodNotAllowed",
        )
        .await;
    }

    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &outsider,
            PolicyCall::new(Method::GET, bucket_name, "policy")
                .with_expected_owner(wrong_owner),
        )
        .await,
        StatusCode::METHOD_NOT_ALLOWED,
        "MethodNotAllowed",
    )
    .await;

    // Once Identity Policy explicitly allows the owner action, the owner-only
    // helper proceeds to bucket lookup and returns standard NoSuchBucket.
    install_identity_policy(
        &state,
        owner.application_id,
        owner.access_key_id,
        &policy_identity_document("*", &ALL_POLICY_ACTIONS, &[]),
    )
    .await;
    assert_s3_error(
        send_policy_call(
            &client,
            address,
            &owner,
            PolicyCall::new(Method::GET, "missing-policy-control", "policy"),
        )
        .await,
        StatusCode::NOT_FOUND,
        "NoSuchBucket",
    )
    .await;

    let delete = send_policy_call(
        &client,
        address,
        &owner,
        PolicyCall::new(Method::DELETE, bucket_name, "policy")
            .with_expected_owner(&owner_account_id),
    )
    .await;
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    server.abort();
    let _ = tokio::fs::remove_dir_all(storage_root).await;
}
