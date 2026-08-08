#[test]
fn anonymous_classification_requires_complete_absence_of_sigv4_material() {
    let anonymous_headers = HeaderMap::new();
    for uri in [
        "/assets/key",
        "/assets/key?x-id=GetObject&versionId=null",
        "/assets?list-type=2&prefix=public%2F&max-keys=10",
    ] {
        assert!(!s3_has_authentication_material(
            &anonymous_headers,
            &uri.parse().expect("URI"),
        ));
    }

    for query_name in S3_PRESIGN_AUTH_QUERY_PARAMETERS {
        let uri = format!("/assets/key?{query_name}=partial")
            .parse()
            .expect("URI");
        assert!(s3_has_authentication_material(&anonymous_headers, &uri));
        let lowercase_uri = format!("/assets/key?{}=partial", query_name.to_ascii_lowercase())
            .parse()
            .expect("lowercase URI");
        assert!(s3_has_authentication_material(
            &anonymous_headers,
            &lowercase_uri,
        ));
    }
    for header_name in ["authorization", "x-amz-date", "x-amz-security-token"] {
        let mut headers = HeaderMap::new();
        headers.insert(
            HeaderName::from_bytes(header_name.as_bytes()).expect("header name"),
            HeaderValue::from_static("partial"),
        );
        assert!(s3_has_authentication_material(
            &headers,
            &Uri::from_static("/assets/key"),
        ));
    }
}

#[test]
fn lowercase_or_partial_presign_material_is_rejected_instead_of_downgraded() {
    for uri in [
        "/assets/key?x-amz-signature=partial",
        "/assets/key?X-Amz-Credential=partial",
        "/assets/key?X-Amz-Algorithm=AWS4-HMAC-SHA256",
    ] {
        let uri: Uri = uri.parse().expect("URI");
        assert!(s3_has_authentication_material(&HeaderMap::new(), &uri));
        let error = ParsedSigV4::parse(
            &Method::GET,
            &uri,
            &HeaderMap::new(),
            std::time::SystemTime::now(),
        )
        .expect_err("partial SigV4 material must be rejected");
        assert!(matches!(
            error,
            SigV4Error::MissingAuthentication
                | SigV4Error::UnsupportedAlgorithm
                | SigV4Error::InvalidCredentialScope
        ));
    }
}

#[test]
fn object_get_classifier_keeps_subresources_out_of_plain_read() {
    let request_id = "request-id";
    for uri in [
        "/assets/key",
        "/assets/key?versionId=null",
        "/assets/key?response-content-type=image%2Fpng&x-id=GetObject",
    ] {
        assert_eq!(
            classify_s3_object_get(&uri.parse().expect("URI"), request_id)
                .expect("plain GetObject"),
            S3ObjectGetOperation::PlainRead,
        );
    }
    assert_eq!(
        classify_s3_object_get(&Uri::from_static("/assets/key?tagging"), request_id)
            .expect("tagging"),
        S3ObjectGetOperation::Tagging,
    );
    assert_eq!(
        classify_s3_object_get(&Uri::from_static("/assets/key?acl"), request_id)
            .expect("ACL"),
        S3ObjectGetOperation::Acl,
    );
    assert!(matches!(
        classify_s3_object_get(&Uri::from_static("/assets/key?retention"), request_id)
            .expect("retention"),
        S3ObjectGetOperation::VersionLock(S3ObjectVersionLockOperation::Retention),
    ));
    assert!(matches!(
        classify_s3_object_get(&Uri::from_static("/assets/key?uploadId=upload-1"), request_id)
            .expect("ListParts"),
        S3ObjectGetOperation::ListParts(upload_id) if upload_id == "upload-1",
    ));
    assert!(matches!(
        classify_s3_object_get(&Uri::from_static("/assets/key?torrent"), request_id)
            .expect("unsupported signed subresource"),
        S3ObjectGetOperation::UnsupportedSubresource(name) if name == "torrent",
    ));
    let mixed = classify_s3_object_get(
        &Uri::from_static("/assets/key?acl&uploadId=upload-1"),
        request_id,
    )
    .expect_err("mixed subresources");
    assert_eq!(mixed.code, "InvalidRequest");
}

#[test]
fn identity_request_distinguishes_current_and_version_reads() {
    let current = S3DataAuthorizationInput {
        action: S3PolicyAction::GetObject,
        bucket_name: "assets",
        object_key: Some("images/a.png"),
        version_id: None,
        prefix: None,
        delimiter: None,
        max_keys: None,
        secure_transport: false,
        source_ip: Some("127.0.0.1".parse().expect("IP")),
    };
    let current = s3_identity_policy_request(
        current,
        "arn:aws:iam::123456789012:user/mh_ak_test",
        "123456789012",
        "mh_ak_test",
    )
    .expect("identity request");
    assert_eq!(current.action, S3PolicyAction::GetObject);
    assert_eq!(current.version_id, None);
    assert_eq!(current.source_ip, Some("127.0.0.1".parse().expect("IP")));

    let version = S3DataAuthorizationInput {
        action: S3PolicyAction::GetObjectVersion,
        version_id: Some("version-1"),
        ..S3DataAuthorizationInput {
            action: S3PolicyAction::GetObject,
            bucket_name: "assets",
            object_key: Some("images/a.png"),
            version_id: None,
            prefix: None,
            delimiter: None,
            max_keys: None,
            secure_transport: true,
            source_ip: None,
        }
    };
    let version = s3_identity_policy_request(
        version,
        "arn:aws:iam::123456789012:user/mh_ak_test",
        "123456789012",
        "mh_ak_test",
    )
    .expect("version identity request");
    assert_eq!(version.action, S3PolicyAction::GetObjectVersion);
    assert_eq!(version.version_id, Some("version-1"));
    assert!(version.secure_transport);
}

mod http_contract {
    use std::{sync::Arc, time::Duration};

    use mediahub_adapter_local::LocalObjectStore;
    use mediahub_adapter_postgres::PostgresRepository;
    use mediahub_app::{
        AccessKeyRepository, ApplicationRepository, AuthRepository, DeleteS3IdentityPolicy,
        NewAccessKey, PutS3IdentityPolicy, S3BucketPolicyDocument, S3BucketPolicyRepository,
        S3IdentityPolicyDocument, S3IdentityPolicyRepository,
    };
    use mediahub_core::{ApplicationId, OffsetDateTime, UserId};

    use super::*;
    use crate::server_config::SystemUpdateConfig;
    use crate::{
        AppState, AuthRateLimiter, CookieConfig, HttpMetrics, MediaUrlSigner, RuntimeObjectStore,
        SystemUpdateService, webdav,
    };
    use mediahub_server::access_key::AccessKeyCipher;

    pub(super) async fn data_policy_test_state(
        pool: sqlx::PgPool,
    ) -> (Arc<AppState>, std::path::PathBuf) {
        let repository = PostgresRepository::new(pool);
        let storage_root = std::env::temp_dir().join(format!(
            "prismark-s3-data-policy-test-{}",
            uuid::Uuid::now_v7().simple()
        ));
        let object_store = RuntimeObjectStore::local(
            LocalObjectStore::new(&storage_root).expect("local object store"),
        );
        let access_key_cipher = Arc::new(
            AccessKeyCipher::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", 1)
                .expect("access key cipher"),
        );
        let webdav = webdav::WebDavService::new(
            repository.clone(),
            object_store.clone(),
            time::Duration::hours(24),
            Arc::clone(&access_key_cipher),
        );
        let state = Arc::new(AppState {
            repository,
            object_store,
            s3_gc_grace: time::Duration::hours(24),
            webdav,
            access_key_cipher,
            media_url_signer: Arc::new(MediaUrlSigner::new(vec![7; 32])),
            cookie_config: CookieConfig {
                secure: false,
                same_site: "Lax",
            },
            cors_allowed_origins: Vec::new(),
            registration_enabled: true,
            expose_auth_tokens: true,
            email_provider: None,
            auth_rate_limiter: AuthRateLimiter::default(),
            variant_slots: Arc::new(tokio::sync::Semaphore::new(1)),
            http_metrics: HttpMetrics::default(),
            metrics_bearer_token: None,
            system_update: SystemUpdateService::new(SystemUpdateConfig {
                updater_url: None,
                updater_token: None,
                github_token: None,
            })
            .expect("system update service"),
        });
        (state, storage_root)
    }

    pub(super) async fn create_data_policy_identity(
        state: &AppState,
        application_id: ApplicationId,
        access_key_id: &str,
        secret: &str,
    ) {
        state
            .repository
            .create_access_key(&NewAccessKey {
                id: uuid::Uuid::now_v7().to_string(),
                application_id,
                access_key_id: access_key_id.to_owned(),
                secret_ciphertext: state
                    .access_key_cipher
                    .encrypt(secret.as_bytes())
                    .expect("encrypt access key"),
                secret_key_version: state.access_key_cipher.version(),
                secret_last_four: secret.chars().rev().take(4).collect::<String>().chars().rev().collect(),
                name: "S3 data policy contract".to_owned(),
                // These legacy grants intentionally prove that data access no
                // longer falls back when the identity policy is absent/denied.
                permissions: vec![
                    "bucket:manage".to_owned(),
                    "bucket:list".to_owned(),
                    "media:upload".to_owned(),
                    "media:read".to_owned(),
                    "media:list".to_owned(),
                ],
                expires_at: None,
                created_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("create access key");
    }

    pub(super) fn sign_data_policy_request(
        request: &mut http::Request<Vec<u8>>,
        access_key_id: &str,
        secret: &str,
    ) {
        if !request.headers().contains_key("x-amz-content-sha256") {
            request.headers_mut().insert(
                HeaderName::from_static("x-amz-content-sha256"),
                HeaderValue::from_static("UNSIGNED-PAYLOAD"),
            );
        }
        let identity = aws_credential_types::Credentials::new(
            access_key_id,
            secret,
            None,
            None,
            "prismark-s3-data-policy-test",
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
            .expect("S3 signing params")
            .into();
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
                    value.to_str().expect("S3 test request header"),
                )
            }),
            aws_sigv4::http_request::SignableBody::UnsignedPayload,
        )
        .expect("S3 signable request");
        aws_sigv4::http_request::sign(signable, &params)
            .expect("S3 signature")
            .into_parts()
            .0
            .apply_to_request_http1x(request);
    }

    pub(super) async fn send_data_policy_request(
        client: &reqwest::Client,
        request: http::Request<Vec<u8>>,
    ) -> reqwest::Response {
        let (parts, body) = request.into_parts();
        client
            .request(parts.method, parts.uri.to_string())
            .headers(parts.headers)
            .body(body)
            .send()
            .await
            .expect("S3 HTTP request")
    }

    pub(super) async fn install_identity_policy(
        state: &AppState,
        application_id: ApplicationId,
        access_key_id: &str,
        document: &str,
    ) {
        state
            .repository
            .put_s3_identity_policy(&PutS3IdentityPolicy {
                application_id,
                access_key_id: access_key_id.to_owned(),
                policy: S3IdentityPolicyDocument::parse(document.as_bytes())
                    .expect("identity policy"),
                updated_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("put identity policy")
            .expect("access key identity");
    }

    pub(super) async fn install_bucket_policy(
        state: &AppState,
        application_id: ApplicationId,
        bucket_name: &str,
        document: serde_json::Value,
    ) {
        state
            .repository
            .put_s3_bucket_policy(
                application_id,
                bucket_name,
                S3BucketPolicyDocument::new(document).expect("bucket policy document"),
                OffsetDateTime::now_utc(),
            )
            .await
            .expect("put bucket policy")
            .expect("bucket identity");
    }

    async fn delete_bucket_policy(
        state: &AppState,
        application_id: ApplicationId,
        bucket_name: &str,
    ) {
        state
            .repository
            .delete_s3_bucket_policy(
                application_id,
                bucket_name,
                OffsetDateTime::now_utc(),
            )
            .await
            .expect("delete bucket policy")
            .expect("bucket identity");
    }

    pub(super) async fn assert_s3_error(
        response: reqwest::Response,
        status: StatusCode,
        code: &str,
    ) {
        assert_eq!(response.status(), status);
        let body = response.text().await.expect("S3 error XML");
        assert!(body.contains(&format!("<Code>{code}</Code>")), "{body}");
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn data_plane_combines_anonymous_identity_and_bucket_policy_without_fallback(
        pool: sqlx::PgPool,
    ) {
        let (state, storage_root) = data_policy_test_state(pool).await;
        let now = OffsetDateTime::now_utc();
        let user_id = UserId::new();
        let application_id = ApplicationId::new();
        state
            .repository
            .create_user(user_id, "s3-data-policy@example.com", "hashed", now)
            .await
            .expect("create user");
        state
            .repository
            .create_application(
                application_id,
                user_id,
                "S3 Data Policy",
                &format!("app_{}", application_id.as_uuid().simple()),
                64 * 1024 * 1024,
                now,
            )
            .await
            .expect("create application");
        let access_key_id = "mh_ak_data_policy_test";
        let secret = "data-policy-test-secret";
        create_data_policy_identity(&state, application_id, access_key_id, secret).await;
        install_identity_policy(
            &state,
            application_id,
            access_key_id,
            r#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Action":"s3:CreateBucket","Resource":"*"}}"#,
        )
        .await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("S3 listener");
        let address = listener.local_addr().expect("S3 address");
        let server = tokio::spawn({
            let application = crate::s3_router::router(Arc::clone(&state));
            async move {
                axum::serve(
                    listener,
                    application.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .expect("S3 data policy server");
            }
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client");
        let bucket_name = "data-policy-assets";
        let bucket_url = format!("http://{address}/{bucket_name}");
        let object_url = format!("{bucket_url}/public/a.txt");

        let mut create_bucket = http::Request::builder()
            .method(Method::PUT)
            .uri(&bucket_url)
            .header("host", address.to_string())
            .body(Vec::new())
            .expect("CreateBucket request");
        sign_data_policy_request(&mut create_bucket, access_key_id, secret);
        assert_eq!(
            send_data_policy_request(&client, create_bucket).await.status(),
            StatusCode::OK,
        );
        install_identity_policy(
            &state,
            application_id,
            access_key_id,
            &format!(
                r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Action":"s3:PutObject","Resource":"arn:aws:s3:::{bucket_name}/public/*"}}}}"#
            ),
        )
        .await;
        let mut put_object = http::Request::builder()
            .method(Method::PUT)
            .uri(&object_url)
            .header("host", address.to_string())
            .header(CONTENT_TYPE, "text/plain")
            .body(b"public-data".to_vec())
            .expect("PutObject request");
        sign_data_policy_request(&mut put_object, access_key_id, secret);
        assert_eq!(
            send_data_policy_request(&client, put_object).await.status(),
            StatusCode::OK,
        );
        state
            .repository
            .delete_s3_identity_policy(&DeleteS3IdentityPolicy {
                application_id,
                access_key_id: access_key_id.to_owned(),
                updated_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("delete PutObject fixture policy")
            .expect("identity snapshot");

        let public_policy = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [
                {
                    "Effect": "Allow",
                    "Principal": "*",
                    "Action": ["s3:GetObject", "s3:GetObjectVersion"],
                    "Resource": format!("arn:aws:s3:::{bucket_name}/public/*"),
                    "Condition": {
                        "Bool": {"aws:SecureTransport": false},
                        "IpAddress": {"aws:SourceIp": "127.0.0.1/32"}
                    }
                },
                {
                    "Effect": "Allow",
                    "Principal": "*",
                    "Action": "s3:ListBucket",
                    "Resource": format!("arn:aws:s3:::{bucket_name}"),
                    "Condition": {
                        "StringEquals": {"s3:prefix": "public/", "s3:delimiter": "/"},
                        "NumericLessThanEquals": {"s3:max-keys": 10},
                        "Bool": {"aws:SecureTransport": false},
                        "IpAddress": {"aws:SourceIp": "127.0.0.1/32"}
                    }
                }
            ]
        });
        install_bucket_policy(&state, application_id, bucket_name, public_policy.clone()).await;

        let anonymous_get = client.get(&object_url).send().await.expect("anonymous GET");
        assert_eq!(anonymous_get.status(), StatusCode::OK);
        assert_eq!(anonymous_get.bytes().await.expect("GET body"), "public-data");
        let anonymous_head = client.head(&object_url).send().await.expect("anonymous HEAD");
        assert_eq!(anonymous_head.status(), StatusCode::OK);
        assert!(anonymous_head.bytes().await.expect("HEAD body").is_empty());
        assert_s3_error(
            client
                .get(format!("{object_url}?tagging"))
                .send()
                .await
                .expect("anonymous GetObjectTagging"),
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        let anonymous_list = client
            .get(format!(
                "{bucket_url}?list-type=2&prefix=public%2F&delimiter=%2F&max-keys=10"
            ))
            .send()
            .await
            .expect("anonymous ListObjectsV2");
        assert_eq!(anonymous_list.status(), StatusCode::OK);
        assert!(
            anonymous_list
                .text()
                .await
                .expect("list XML")
                .contains("<Key>public/a.txt</Key>")
        );
        assert_s3_error(
            client
                .get(format!("{bucket_url}/public/missing.txt"))
                .send()
                .await
                .expect("missing key"),
            StatusCode::NOT_FOUND,
            "NoSuchKey",
        )
        .await;

        let mut bad_signature = http::Request::builder()
            .method(Method::GET)
            .uri(&object_url)
            .header("host", address.to_string())
            .body(Vec::new())
            .expect("bad signature request");
        sign_data_policy_request(&mut bad_signature, access_key_id, "wrong-secret");
        assert_s3_error(
            send_data_policy_request(&client, bad_signature).await,
            StatusCode::FORBIDDEN,
            "SignatureDoesNotMatch",
        )
        .await;
        for attempted_auth_url in [
            format!("{object_url}?X-Amz-Signature=partial"),
            format!("{object_url}?x-amz-signature=partial"),
        ] {
            let attempted = client
                .get(attempted_auth_url)
                .send()
                .await
                .expect("partial SigV4 request");
            assert_ne!(attempted.status(), StatusCode::OK);
            assert!(
                attempted
                    .text()
                    .await
                    .expect("partial SigV4 XML")
                    .contains("<Code>AccessDenied</Code>")
            );
        }

        delete_bucket_policy(&state, application_id, bucket_name).await;
        assert_s3_error(
            client.get(&object_url).send().await.expect("private anonymous GET"),
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_s3_error(
            client
                .get(format!("{bucket_url}/private/missing.txt"))
                .send()
                .await
                .expect("private missing-key GET"),
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_s3_error(
            client
                .get(format!(
                    "{bucket_url}?list-type=2&prefix=public%2F&max-keys=10"
                ))
                .send()
                .await
                .expect("private ListObjectsV2"),
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        let mut signed_without_policy = http::Request::builder()
            .method(Method::GET)
            .uri(&object_url)
            .header("host", address.to_string())
            .body(Vec::new())
            .expect("signed request without identity policy");
        sign_data_policy_request(&mut signed_without_policy, access_key_id, secret);
        assert_s3_error(
            send_data_policy_request(&client, signed_without_policy).await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        let identity_allow = format!(
            r#"{{"Version":"2012-10-17","Statement":[{{"Effect":"Allow","Action":["s3:GetObject","s3:GetObjectVersion"],"Resource":"arn:aws:s3:::{bucket_name}/public/*"}},{{"Effect":"Allow","Action":"s3:ListBucket","Resource":"arn:aws:s3:::{bucket_name}"}}]}}"#
        );
        install_identity_policy(
            &state,
            application_id,
            access_key_id,
            &identity_allow,
        )
        .await;
        let mut signed_allow = http::Request::builder()
            .method(Method::GET)
            .uri(&object_url)
            .header("host", address.to_string())
            .body(Vec::new())
            .expect("signed identity allow");
        sign_data_policy_request(&mut signed_allow, access_key_id, secret);
        assert_eq!(
            send_data_policy_request(&client, signed_allow).await.status(),
            StatusCode::OK,
        );

        install_bucket_policy(&state, application_id, bucket_name, public_policy).await;
        let identity_deny = format!(
            r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Deny","Action":"s3:GetObject","Resource":"arn:aws:s3:::{bucket_name}/public/*"}}}}"#
        );
        install_identity_policy(&state, application_id, access_key_id, &identity_deny).await;
        let mut signed_identity_deny = http::Request::builder()
            .method(Method::GET)
            .uri(&object_url)
            .header("host", address.to_string())
            .body(Vec::new())
            .expect("signed identity deny");
        sign_data_policy_request(&mut signed_identity_deny, access_key_id, secret);
        assert_s3_error(
            send_data_policy_request(&client, signed_identity_deny).await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        install_identity_policy(
            &state,
            application_id,
            access_key_id,
            &identity_allow,
        )
        .await;
        let caller = state
            .repository
            .get_s3_identity_policy(access_key_id)
            .await
            .expect("identity lookup")
            .expect("identity snapshot");
        let caller_arn = format!(
            "arn:aws:iam::{}:user/{access_key_id}",
            caller.identity.account_id.as_str()
        );
        install_bucket_policy(
            &state,
            application_id,
            bucket_name,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Deny",
                    "Principal": {"AWS": caller_arn},
                    "Action": "s3:GetObject",
                    "Resource": format!("arn:aws:s3:::{bucket_name}/public/*")
                }
            }),
        )
        .await;
        let mut signed_bucket_deny = http::Request::builder()
            .method(Method::GET)
            .uri(&object_url)
            .header("host", address.to_string())
            .body(Vec::new())
            .expect("signed bucket deny");
        sign_data_policy_request(&mut signed_bucket_deny, access_key_id, secret);
        assert_s3_error(
            send_data_policy_request(&client, signed_bucket_deny).await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        state
            .repository
            .delete_s3_identity_policy(&DeleteS3IdentityPolicy {
                application_id,
                access_key_id: access_key_id.to_owned(),
                updated_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("delete identity policy")
            .expect("identity snapshot");
        assert_s3_error(
            client
                .get(format!("http://{address}/missing-data-bucket/public/a.txt"))
                .send()
                .await
                .expect("missing bucket"),
            StatusCode::NOT_FOUND,
            "NoSuchBucket",
        )
        .await;

        server.abort();
        let _ = server.await;
        drop(client);
        drop(state);
        std::fs::remove_dir_all(storage_root).expect("remove test object storage");
    }
}

include!("s3_put_copy_policy_tests.rs");
include!("s3_account_policy_tests.rs");
