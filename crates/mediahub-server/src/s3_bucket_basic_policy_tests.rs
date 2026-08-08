mod bucket_basic_policy_http_contract {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use mediahub_adapter_local::LocalObjectStore;
    use mediahub_adapter_postgres::PostgresRepository;
    use mediahub_app::{
        AccessKeyRepository, ApplicationRepository, AuditRepository, AuthRepository,
        NewAccessKey, PutS3IdentityPolicy, S3BucketPolicyDocument, S3BucketPolicyRepository,
        S3IdentityPolicyDocument, S3IdentityPolicyRepository,
    };
    use mediahub_core::{ApplicationId, BucketId, OffsetDateTime, S3Bucket, UserId};

    use super::*;
    use crate::server_config::SystemUpdateConfig;
    use crate::{
        AppState, AuthRateLimiter, CookieConfig, HttpMetrics, MediaUrlSigner, RuntimeObjectStore,
        SystemUpdateService, webdav,
    };
    use mediahub_server::access_key::AccessKeyCipher;

    const OWNER_KEY: &str = "mh_ak_bucket_policy_owner";
    const OWNER_SECRET: &str = "bucket-policy-owner-secret";
    const CALLER_KEY: &str = "mh_ak_bucket_policy_caller";
    const CALLER_SECRET: &str = "bucket-policy-caller-secret";

    struct TestRuntime {
        state: Arc<AppState>,
        storage_root: std::path::PathBuf,
        server: tokio::task::JoinHandle<()>,
        client: reqwest::Client,
        address: SocketAddr,
    }

    impl TestRuntime {
        async fn start(pool: sqlx::PgPool) -> Self {
            let repository = PostgresRepository::new(pool);
            let storage_root = std::env::temp_dir().join(format!(
                "prismark-s3-bucket-basic-policy-test-{}",
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
                    .expect("S3 bucket policy server");
                }
            });
            let client = reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()
                .expect("HTTP client");
            Self {
                state,
                storage_root,
                server,
                client,
                address,
            }
        }

        fn url(&self, path_and_query: &str) -> String {
            format!("http://{}{}", self.address, path_and_query)
        }

        async fn send(
            &self,
            method: Method,
            path_and_query: &str,
            body: Vec<u8>,
            access_key_id: &str,
            secret: &str,
        ) -> reqwest::Response {
            let mut request = http::Request::builder()
                .method(method)
                .uri(self.url(path_and_query))
                .header("host", self.address.to_string())
                .header(CONTENT_LENGTH, body.len().to_string())
                .body(body)
                .expect("S3 request");
            sign_request(&mut request, access_key_id, secret);
            let (parts, body) = request.into_parts();
            self.client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .expect("S3 HTTP response")
        }

        async fn send_anonymous(
            &self,
            method: Method,
            path_and_query: &str,
        ) -> reqwest::Response {
            self.client
                .request(method, self.url(path_and_query))
                .header("host", self.address.to_string())
                .send()
                .await
                .expect("anonymous S3 HTTP response")
        }

        async fn stop(self) {
            self.server.abort();
            let _ = self.server.await;
            let _ = std::fs::remove_dir_all(self.storage_root);
        }
    }

    async fn create_application(
        state: &AppState,
        application_id: ApplicationId,
        email: &str,
        name: &str,
    ) {
        let now = OffsetDateTime::now_utc();
        let user_id = UserId::new();
        state
            .repository
            .create_user(user_id, email, "hashed", now)
            .await
            .expect("create user");
        state
            .repository
            .create_application(
                application_id,
                user_id,
                name,
                &format!("app_{}", application_id.as_uuid().simple()),
                64 * 1024 * 1024,
                now,
            )
            .await
            .expect("create application");
    }

    async fn create_access_key(
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
                secret_last_four: secret
                    .chars()
                    .rev()
                    .take(4)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect(),
                name: "S3 bucket basic policy contract".to_owned(),
                permissions: vec![
                    "bucket:manage".to_owned(),
                    "bucket:list".to_owned(),
                    "media:upload".to_owned(),
                    "media:list".to_owned(),
                ],
                expires_at: None,
                created_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("create access key");
    }

    async fn create_bucket(state: &AppState, application_id: ApplicationId, bucket_name: &str) {
        state
            .repository
            .create_s3_bucket(
                &S3Bucket::new(
                    BucketId::new(),
                    application_id,
                    bucket_name,
                    DEFAULT_S3_REGION,
                    false,
                    None,
                    OffsetDateTime::now_utc(),
                )
                .expect("S3 bucket"),
            )
            .await
            .expect("create S3 bucket");
    }

    async fn put_identity_policy(
        state: &AppState,
        application_id: ApplicationId,
        access_key_id: &str,
        document: serde_json::Value,
    ) {
        state
            .repository
            .put_s3_identity_policy(&PutS3IdentityPolicy {
                application_id,
                access_key_id: access_key_id.to_owned(),
                policy: S3IdentityPolicyDocument::parse(
                    &serde_json::to_vec(&document).expect("serialize identity policy"),
                )
                .expect("identity policy"),
                updated_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("put identity policy")
            .expect("access key identity");
    }

    async fn put_bucket_policy(
        state: &AppState,
        owner_application_id: ApplicationId,
        bucket_name: &str,
        document: serde_json::Value,
    ) {
        state
            .repository
            .put_s3_bucket_policy(
                owner_application_id,
                bucket_name,
                S3BucketPolicyDocument::new(document).expect("bucket policy document"),
                OffsetDateTime::now_utc(),
            )
            .await
            .expect("put bucket policy")
            .expect("bucket identity");
    }

    fn sign_request(request: &mut http::Request<Vec<u8>>, access_key_id: &str, secret: &str) {
        request.headers_mut().insert(
            HeaderName::from_static("x-amz-content-sha256"),
            HeaderValue::from_static("UNSIGNED-PAYLOAD"),
        );
        let identity = aws_credential_types::Credentials::new(
            access_key_id,
            secret,
            None,
            None,
            "prismark-s3-bucket-basic-policy-test",
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
            .region(DEFAULT_S3_REGION)
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

    async fn assert_s3_error(response: reqwest::Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        let body = response.text().await.expect("S3 error XML");
        assert!(body.contains(&format!("<Code>{code}</Code>")), "{body}");
    }

    fn caller_bucket_policy(
        bucket_name: &str,
        caller_arn: &str,
        allow_put_and_list_versions: bool,
    ) -> serde_json::Value {
        let mut statements = vec![serde_json::json!({
            "Effect": "Allow",
            "Principal": {"AWS": caller_arn},
            "Action": ["s3:GetBucketLocation", "s3:ListBucket", "s3:DeleteBucket"],
            "Resource": format!("arn:aws:s3:::{bucket_name}")
        })];
        if allow_put_and_list_versions {
            statements.extend([
                serde_json::json!({
                    "Effect": "Allow",
                    "Principal": {"AWS": caller_arn},
                    "Action": "s3:ListBucketVersions",
                    "Resource": format!("arn:aws:s3:::{bucket_name}"),
                    "Condition": {
                        "StringLike": {"s3:prefix": "allowed/*"},
                        "NumericLessThanEquals": {"s3:max-keys": 5}
                    }
                }),
                serde_json::json!({
                    "Effect": "Allow",
                    "Principal": {"AWS": caller_arn},
                    "Action": "s3:PutObject",
                    "Resource": format!("arn:aws:s3:::{bucket_name}/allowed/*")
                }),
            ]);
        }
        serde_json::json!({
            "Version": "2012-10-17",
            "Statement": statements
        })
    }

    #[test]
    fn bucket_handlers_use_exact_policy_actions_without_legacy_fallback() {
        let source = include_str!("s3_http_core.rs");
        let cases = [
            (
                "pub(super) async fn s3_list_object_versions",
                "pub(super) async fn s3_list_multipart_uploads",
                "S3PolicyAction::ListBucketVersions",
            ),
            (
                "pub(super) async fn s3_get_bucket_location",
                "pub(super) async fn s3_head_bucket",
                "S3PolicyAction::GetBucketLocation",
            ),
            (
                "pub(super) async fn s3_head_bucket",
                "pub(super) async fn s3_delete_bucket",
                "S3PolicyAction::ListBucket",
            ),
            (
                "pub(super) async fn s3_delete_bucket",
                "fn s3_list_token_codec",
                "S3PolicyAction::DeleteBucket",
            ),
        ];
        for (start, end, action) in cases {
            let start = source.find(start).expect("handler start");
            let end = source[start..].find(end).expect("handler end") + start;
            let handler = &source[start..end];
            let expected_helper = if action == "S3PolicyAction::DeleteBucket" {
                "authorize_s3_signed_data_request"
            } else {
                "authorize_s3_data_request"
            };
            assert!(handler.contains(expected_helper));
            if action != "S3PolicyAction::DeleteBucket" {
                assert!(!handler.contains("authorize_s3_signed_data_request"));
            }
            assert!(handler.contains(action), "missing {action}");
            assert!(!handler.contains("auth.authorize("));
        }
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn bucket_basics_enforce_policy_target_tenant_empty_check_and_audit(
        pool: sqlx::PgPool,
    ) {
        let runtime = TestRuntime::start(pool).await;
        let owner_application_id = ApplicationId::new();
        let caller_application_id = ApplicationId::new();
        create_application(
            &runtime.state,
            owner_application_id,
            "bucket-policy-owner@example.com",
            "Bucket Policy Owner",
        )
        .await;
        create_application(
            &runtime.state,
            caller_application_id,
            "bucket-policy-caller@example.com",
            "Bucket Policy Caller",
        )
        .await;
        create_access_key(
            &runtime.state,
            owner_application_id,
            OWNER_KEY,
            OWNER_SECRET,
        )
        .await;
        create_access_key(
            &runtime.state,
            caller_application_id,
            CALLER_KEY,
            CALLER_SECRET,
        )
        .await;

        let list_bucket = "bucket-policy-versions";
        let empty_bucket = "bucket-policy-empty";
        create_bucket(&runtime.state, owner_application_id, list_bucket).await;
        create_bucket(&runtime.state, owner_application_id, empty_bucket).await;

        for (method, path) in [
            (Method::GET, format!("/{list_bucket}?versions")),
            (Method::GET, format!("/{list_bucket}?location")),
            (Method::HEAD, format!("/{list_bucket}")),
            (Method::DELETE, format!("/{empty_bucket}")),
        ] {
            let response = runtime
                .send(method.clone(), &path, Vec::new(), OWNER_KEY, OWNER_SECRET)
                .await;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
            if method != Method::HEAD {
                let body = response.text().await.expect("legacy denial XML");
                assert!(body.contains("<Code>AccessDenied</Code>"), "{body}");
            }
        }
        assert!(
            runtime
                .state
                .repository
                .find_s3_bucket(owner_application_id, empty_bucket)
                .await
                .expect("empty bucket lookup after denial")
                .is_some()
        );

        let bucket_resources = [list_bucket, empty_bucket]
            .map(|bucket| format!("arn:aws:s3:::{bucket}"));
        put_identity_policy(
            &runtime.state,
            caller_application_id,
            CALLER_KEY,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": [
                    {
                        "Effect": "Allow",
                        "Action": [
                            "s3:ListBucketVersions",
                            "s3:GetBucketLocation",
                            "s3:ListBucket",
                            "s3:DeleteBucket"
                        ],
                        "Resource": bucket_resources
                    },
                    {
                        "Effect": "Allow",
                        "Action": "s3:PutObject",
                        "Resource": format!("arn:aws:s3:::{list_bucket}/allowed/*")
                    }
                ]
            }),
        )
        .await;
        let caller_identity = runtime
            .state
            .repository
            .get_s3_identity_policy(CALLER_KEY)
            .await
            .expect("caller identity lookup")
            .expect("caller identity");
        let caller_arn = format!(
            "arn:aws:iam::{}:user/{CALLER_KEY}",
            caller_identity.identity.account_id.as_str()
        );
        let mut public_list_bucket_policy = caller_bucket_policy(list_bucket, &caller_arn, true);
        public_list_bucket_policy["Statement"]
            .as_array_mut()
            .expect("bucket policy statements")
            .push(serde_json::json!({
                "Effect": "Allow",
                "Principal": "*",
                "Action": "s3:GetBucketLocation",
                "Resource": format!("arn:aws:s3:::{list_bucket}")
            }));
        put_bucket_policy(
            &runtime.state,
            owner_application_id,
            list_bucket,
            public_list_bucket_policy,
        )
        .await;
        put_bucket_policy(
            &runtime.state,
            owner_application_id,
            empty_bucket,
            caller_bucket_policy(empty_bucket, &caller_arn, false),
        )
        .await;

        assert_eq!(
            runtime
                .send_anonymous(Method::GET, &format!("/{list_bucket}?location"))
                .await
                .status(),
            StatusCode::OK
        );

        assert_s3_error(
            runtime
                .send(
                    Method::GET,
                    &format!("/{list_bucket}?location"),
                    Vec::new(),
                    CALLER_KEY,
                    "wrong-secret",
                )
                .await,
            StatusCode::FORBIDDEN,
            "SignatureDoesNotMatch",
        )
        .await;

        let put_response = runtime
            .send(
                Method::PUT,
                &format!("/{list_bucket}/allowed/file.txt"),
                b"bucket policy version".to_vec(),
                CALLER_KEY,
                CALLER_SECRET,
            )
            .await;
        assert_eq!(put_response.status(), StatusCode::OK);

        let list_response = runtime
            .send(
                Method::GET,
                &format!("/{list_bucket}?versions&prefix=allowed%2F&max-keys=5"),
                Vec::new(),
                CALLER_KEY,
                CALLER_SECRET,
            )
            .await;
        assert_eq!(list_response.status(), StatusCode::OK);
        let list_xml = list_response.text().await.expect("ListObjectVersions XML");
        let owner_identity = runtime
            .state
            .repository
            .resolve_s3_bucket_identity(list_bucket)
            .await
            .expect("owner bucket identity lookup")
            .expect("owner bucket identity");
        assert!(list_xml.contains("<Key>allowed/file.txt</Key>"), "{list_xml}");
        assert!(
            list_xml.contains(&format!(
                "<Owner><ID>{}</ID><DisplayName>PrismArk Account</DisplayName></Owner>",
                owner_identity.owner_account_id.as_str()
            )),
            "{list_xml}"
        );
        assert_s3_error(
            runtime
                .send(
                    Method::GET,
                    &format!("/{list_bucket}?versions&prefix=denied%2F&max-keys=5"),
                    Vec::new(),
                    CALLER_KEY,
                    CALLER_SECRET,
                )
                .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        assert_eq!(
            runtime
                .send(
                    Method::GET,
                    &format!("/{list_bucket}?location"),
                    Vec::new(),
                    CALLER_KEY,
                    CALLER_SECRET,
                )
                .await
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            runtime
                .send(
                    Method::HEAD,
                    &format!("/{list_bucket}"),
                    Vec::new(),
                    CALLER_KEY,
                    CALLER_SECRET,
                )
                .await
                .status(),
            StatusCode::OK
        );
        assert_s3_error(
            runtime
                .send(
                    Method::DELETE,
                    &format!("/{list_bucket}"),
                    Vec::new(),
                    CALLER_KEY,
                    CALLER_SECRET,
                )
                .await,
            StatusCode::CONFLICT,
            "BucketNotEmpty",
        )
        .await;

        let empty_bucket_id = runtime
            .state
            .repository
            .find_s3_bucket(owner_application_id, empty_bucket)
            .await
            .expect("empty bucket lookup")
            .expect("empty bucket")
            .id();
        assert_eq!(
            runtime
                .send(
                    Method::DELETE,
                    &format!("/{empty_bucket}"),
                    Vec::new(),
                    CALLER_KEY,
                    CALLER_SECRET,
                )
                .await
                .status(),
            StatusCode::NO_CONTENT
        );
        assert!(
            runtime
                .state
                .repository
                .find_s3_bucket(owner_application_id, empty_bucket)
                .await
                .expect("deleted bucket lookup")
                .is_none()
        );
        let owner_audit = runtime
            .state
            .repository
            .list_audit(owner_application_id, 100)
            .await
            .expect("owner audit list");
        assert!(owner_audit.iter().any(|event| {
            event.action == "bucket.deleted"
                && event.actor_id == CALLER_KEY
                && event.target_id == empty_bucket_id.to_string()
        }));
        let caller_audit = runtime
            .state
            .repository
            .list_audit(caller_application_id, 100)
            .await
            .expect("caller audit list");
        assert!(!caller_audit.iter().any(|event| {
            event.action == "bucket.deleted" && event.target_id == empty_bucket_id.to_string()
        }));

        runtime.stop().await;
    }
}
