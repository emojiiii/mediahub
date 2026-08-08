mod multipart_policy_http_contract {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use mediahub_adapter_local::LocalObjectStore;
    use mediahub_adapter_postgres::PostgresRepository;
    use mediahub_app::{
        AccessKeyRepository, ApplicationRepository, AuditRepository, AuthRepository,
        DeleteS3IdentityPolicy, NewAccessKey, PutS3IdentityPolicy, S3BucketPolicyDocument,
        S3BucketPolicyRepository, S3IdentityPolicyDocument, S3IdentityPolicyRepository,
    };
    use mediahub_core::{ApplicationId, OffsetDateTime, UserId};

    use super::*;
    use crate::server_config::SystemUpdateConfig;
    use crate::{
        AppState, AuthRateLimiter, CookieConfig, HttpMetrics, MediaUrlSigner, RuntimeObjectStore,
        SystemUpdateService, webdav,
    };
    use mediahub_server::access_key::AccessKeyCipher;

    const OWNER_KEY: &str = "mh_ak_multipart_policy_owner";
    const OWNER_SECRET: &str = "multipart-policy-owner-secret";
    const CALLER_KEY: &str = "mh_ak_multipart_policy_caller";
    const CALLER_SECRET: &str = "multipart-policy-caller-secret";

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
                "prismark-s3-multipart-policy-test-{}",
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
                    .expect("S3 multipart policy server");
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

        async fn stop(self) {
            self.server.abort();
            let _ = self.server.await;
            let _ = std::fs::remove_dir_all(self.storage_root);
        }

        fn url(&self, path_and_query: &str) -> String {
            format!("http://{}{}", self.address, path_and_query)
        }

        async fn send(
            &self,
            method: Method,
            path_and_query: &str,
            body: Vec<u8>,
            key: &str,
            secret: &str,
            extra_headers: &[(&str, &str)],
        ) -> reqwest::Response {
            let url = self.url(path_and_query);
            let mut request = http::Request::builder()
                .method(method)
                .uri(&url)
                .header("host", self.address.to_string())
                .header(CONTENT_LENGTH, body.len().to_string())
                .body(body)
                .expect("S3 request");
            for &(name, value) in extra_headers {
                request.headers_mut().insert(
                    HeaderName::from_bytes(name.as_bytes()).expect("header name"),
                    HeaderValue::from_str(value).expect("header value"),
                );
            }
            sign_request(&mut request, key, secret);
            let (parts, body) = request.into_parts();
            self.client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(body)
                .send()
                .await
                .expect("S3 HTTP response")
        }
    }

    async fn create_application(
        state: &AppState,
        user_id: UserId,
        application_id: ApplicationId,
        email: &str,
        name: &str,
    ) {
        let now = OffsetDateTime::now_utc();
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
                name: "S3 multipart policy contract".to_owned(),
                permissions: vec![
                    "bucket:manage".to_owned(),
                    "bucket:list".to_owned(),
                    "media:upload".to_owned(),
                    "media:read".to_owned(),
                    "media:list".to_owned(),
                    "media:delete".to_owned(),
                ],
                expires_at: None,
                created_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("create access key");
    }

    async fn put_identity_policy(
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

    async fn delete_identity_policy(
        state: &AppState,
        application_id: ApplicationId,
        access_key_id: &str,
    ) {
        state
            .repository
            .delete_s3_identity_policy(&DeleteS3IdentityPolicy {
                application_id,
                access_key_id: access_key_id.to_owned(),
                updated_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("delete identity policy")
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
            "prismark-s3-multipart-policy-test",
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

    fn identity_policy(bucket: &str, actions: &[&str], list_uploads: bool) -> String {
        let mut statements = vec![serde_json::json!({
            "Effect": "Allow",
            "Action": actions,
            "Resource": format!("arn:aws:s3:::{bucket}/*")
        })];
        if list_uploads {
            statements.push(serde_json::json!({
                "Effect": "Allow",
                "Action": "s3:ListBucketMultipartUploads",
                "Resource": format!("arn:aws:s3:::{bucket}")
            }));
        }
        serde_json::json!({
            "Version": "2012-10-17",
            "Statement": statements
        })
        .to_string()
    }

    fn bucket_allow_policy(bucket: &str, caller_arn: &str) -> serde_json::Value {
        serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [
                {
                    "Effect": "Allow",
                    "Principal": {"AWS": caller_arn},
                    "Action": [
                        "s3:PutObject",
                        "s3:PutObjectTagging",
                        "s3:PutObjectAcl",
                        "s3:AbortMultipartUpload",
                        "s3:ListMultipartUploadParts"
                    ],
                    "Resource": format!("arn:aws:s3:::{bucket}/*")
                },
                {
                    "Effect": "Allow",
                    "Principal": {"AWS": caller_arn},
                    "Action": "s3:ListBucketMultipartUploads",
                    "Resource": format!("arn:aws:s3:::{bucket}")
                }
            ]
        })
    }

    fn xml_value(xml: &str, name: &str) -> Option<String> {
        let start_tag = format!("<{name}>");
        let end_tag = format!("</{name}>");
        let start = xml.find(&start_tag)? + start_tag.len();
        let end = xml[start..].find(&end_tag)? + start;
        Some(xml[start..end].to_owned())
    }

    async fn assert_s3_error(response: reqwest::Response, status: StatusCode, code: &str) {
        assert_eq!(response.status(), status);
        let body = response.text().await.expect("S3 error XML");
        assert!(body.contains(&format!("<Code>{code}</Code>")), "{body}");
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn multipart_pipeline_uses_identity_and_bucket_policy_without_legacy_fallback(
        pool: sqlx::PgPool,
    ) {
        let runtime = TestRuntime::start(pool).await;
        let owner_application_id = ApplicationId::new();
        let caller_application_id = ApplicationId::new();
        create_application(
            &runtime.state,
            UserId::new(),
            owner_application_id,
            "multipart-owner@example.com",
            "Multipart Owner",
        )
        .await;
        create_application(
            &runtime.state,
            UserId::new(),
            caller_application_id,
            "multipart-caller@example.com",
            "Multipart Caller",
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

        put_identity_policy(
            &runtime.state,
            owner_application_id,
            OWNER_KEY,
            r#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Action":"s3:CreateBucket","Resource":"*"}}"#,
        )
        .await;
        let bucket_name = format!(
            "multipart-policy-{}",
            owner_application_id
                .as_uuid()
                .simple()
                .to_string()
                .chars()
                .take(8)
                .collect::<String>()
        );
        assert_eq!(
            runtime
                .send(
                    Method::PUT,
                    &format!("/{bucket_name}"),
                    Vec::new(),
                    OWNER_KEY,
                    OWNER_SECRET,
                    &[],
                )
                .await
                .status(),
            StatusCode::OK,
        );

        let full_actions = [
            "s3:PutObject",
            "s3:PutObjectTagging",
            "s3:PutObjectAcl",
            "s3:AbortMultipartUpload",
            "s3:ListMultipartUploadParts",
        ];
        put_identity_policy(
            &runtime.state,
            caller_application_id,
            CALLER_KEY,
            &identity_policy(&bucket_name, &full_actions, true),
        )
        .await;
        let caller = runtime
            .state
            .repository
            .get_s3_identity_policy(CALLER_KEY)
            .await
            .expect("caller identity lookup")
            .expect("caller identity snapshot");
        let caller_arn = format!(
            "arn:aws:iam::{}:user/{CALLER_KEY}",
            caller.identity.account_id.as_str()
        );

        assert_s3_error(
            runtime
                .send(
                    Method::POST,
                    &format!("/{bucket_name}/target.bin?uploads"),
                    Vec::new(),
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[],
                )
                .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        put_bucket_policy(
            &runtime.state,
            owner_application_id,
            &bucket_name,
            bucket_allow_policy(&bucket_name, &caller_arn),
        )
        .await;
        put_identity_policy(
            &runtime.state,
            caller_application_id,
            CALLER_KEY,
            &identity_policy(&bucket_name, &["s3:PutObject"], false),
        )
        .await;
        assert_s3_error(
            runtime
                .send(
                    Method::POST,
                    &format!("/{bucket_name}/tagged.bin?uploads"),
                    Vec::new(),
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[("x-amz-tagging", "kind=test"), ("x-amz-acl", "private")],
                )
                .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        put_identity_policy(
            &runtime.state,
            caller_application_id,
            CALLER_KEY,
            &identity_policy(&bucket_name, &full_actions, true),
        )
        .await;

        let create = runtime
            .send(
                Method::POST,
                &format!("/{bucket_name}/target.bin?uploads"),
                Vec::new(),
                CALLER_KEY,
                CALLER_SECRET,
                &[("x-amz-tagging", "kind=test"), ("x-amz-acl", "private")],
            )
            .await;
        assert_eq!(create.status(), StatusCode::OK);
        let create_xml = create.text().await.expect("CreateMultipartUpload XML");
        let upload_id = xml_value(&create_xml, "UploadId").expect("UploadId");
        let fake_upload_id = "mh_mpu_00000000000000000000000000000000";

        let part_body = b"first multipart part".to_vec();
        assert_eq!(
            runtime
                .send(
                    Method::PUT,
                    &format!("/{bucket_name}/target.bin?partNumber=1&uploadId={upload_id}"),
                    part_body,
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[],
                )
                .await
                .status(),
            StatusCode::OK,
        );

        put_identity_policy(
            &runtime.state,
            caller_application_id,
            CALLER_KEY,
            &serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Deny",
                    "Action": "s3:PutObject",
                    "Resource": format!("arn:aws:s3:::{bucket_name}/target.bin")
                }
            })
            .to_string(),
        )
        .await;
        assert_s3_error(
            runtime
                .send(
                    Method::PUT,
                    &format!("/{bucket_name}/target.bin?partNumber=2&uploadId={fake_upload_id}"),
                    b"denied fake multipart part".to_vec(),
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[],
                )
                .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        for candidate_upload_id in [&upload_id, fake_upload_id] {
            assert_s3_error(
                runtime
                    .send(
                        Method::PUT,
                        &format!(
                            "/{bucket_name}/target.bin?partNumber=3&uploadId={candidate_upload_id}"
                        ),
                        Vec::new(),
                        CALLER_KEY,
                        CALLER_SECRET,
                        &[("x-amz-copy-source", "/missing/source.bin")],
                    )
                    .await,
                StatusCode::FORBIDDEN,
                "AccessDenied",
            )
            .await;
        }
        assert_s3_error(
            runtime
                .send(
                    Method::PUT,
                    &format!("/{bucket_name}/target.bin?partNumber=2&uploadId={upload_id}"),
                    b"denied multipart part".to_vec(),
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[],
                )
                .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        let complete_body = format!(
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>\"{}\"</ETag></Part></CompleteMultipartUpload>",
            hex::encode(md5::Md5::digest(b"first multipart part"))
        )
        .into_bytes();
        assert_s3_error(
            runtime
                .send(
                    Method::POST,
                    &format!("/{bucket_name}/target.bin?uploadId={upload_id}"),
                    complete_body.clone(),
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[("content-type", "application/xml")],
                )
                .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_s3_error(
            runtime
                .send(
                    Method::POST,
                    &format!("/{bucket_name}/target.bin?uploadId={fake_upload_id}"),
                    complete_body.clone(),
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[("content-type", "application/xml")],
                )
                .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        delete_identity_policy(&runtime.state, caller_application_id, CALLER_KEY).await;
        for candidate_upload_id in [&upload_id, fake_upload_id] {
            assert_s3_error(
                runtime
                    .send(
                        Method::GET,
                        &format!("/{bucket_name}/target.bin?uploadId={candidate_upload_id}"),
                        Vec::new(),
                        CALLER_KEY,
                        CALLER_SECRET,
                        &[],
                    )
                    .await,
                StatusCode::FORBIDDEN,
                "AccessDenied",
            )
            .await;
        }
        put_identity_policy(
            &runtime.state,
            caller_application_id,
            CALLER_KEY,
            &identity_policy(&bucket_name, &full_actions, true),
        )
        .await;

        let list_parts = runtime
            .send(
                Method::GET,
                &format!("/{bucket_name}/target.bin?uploadId={upload_id}"),
                Vec::new(),
                CALLER_KEY,
                CALLER_SECRET,
                &[],
            )
            .await;
        assert_eq!(list_parts.status(), StatusCode::OK);
        let list_parts_xml = list_parts.text().await.expect("ListParts XML");
        assert!(list_parts_xml.contains("<PartNumber>1</PartNumber>"));
        assert!(!list_parts_xml.contains("<PartNumber>2</PartNumber>"));

        let list_uploads = runtime
            .send(
                Method::GET,
                &format!("/{bucket_name}?uploads"),
                Vec::new(),
                CALLER_KEY,
                CALLER_SECRET,
                &[],
            )
            .await;
        assert_eq!(list_uploads.status(), StatusCode::OK);
        assert!(
            list_uploads
                .text()
                .await
                .expect("ListMultipartUploads XML")
                .contains(&upload_id)
        );
        assert_s3_error(
            runtime
                .send(
                    Method::GET,
                    &format!("/{bucket_name}/wrong.bin?uploadId={upload_id}"),
                    Vec::new(),
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[],
                )
                .await,
            StatusCode::NOT_FOUND,
            "NoSuchUpload",
        )
        .await;

        assert_eq!(
            runtime
                .send(
                    Method::POST,
                    &format!("/{bucket_name}/target.bin?uploadId={upload_id}"),
                    complete_body,
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[("content-type", "application/xml")],
                )
                .await
                .status(),
            StatusCode::OK,
        );

        let create_abort = runtime
            .send(
                Method::POST,
                &format!("/{bucket_name}/abort.bin?uploads"),
                Vec::new(),
                CALLER_KEY,
                CALLER_SECRET,
                &[],
            )
            .await;
        assert_eq!(create_abort.status(), StatusCode::OK);
        let abort_upload_id = xml_value(
            &create_abort
                .text()
                .await
                .expect("abort CreateMultipart XML"),
            "UploadId",
        )
        .expect("abort UploadId");

        put_identity_policy(
            &runtime.state,
            caller_application_id,
            CALLER_KEY,
            &identity_policy(&bucket_name, &["s3:PutObject"], false),
        )
        .await;
        for (method, path) in [
            (
                Method::GET,
                format!("/{bucket_name}/abort.bin?uploadId={abort_upload_id}"),
            ),
            (Method::GET, format!("/{bucket_name}?uploads")),
            (
                Method::DELETE,
                format!("/{bucket_name}/abort.bin?uploadId={abort_upload_id}"),
            ),
            (
                Method::GET,
                format!("/{bucket_name}/abort.bin?uploadId={fake_upload_id}"),
            ),
            (
                Method::DELETE,
                format!("/{bucket_name}/abort.bin?uploadId={fake_upload_id}"),
            ),
        ] {
            assert_s3_error(
                runtime
                    .send(method, &path, Vec::new(), CALLER_KEY, CALLER_SECRET, &[])
                    .await,
                StatusCode::FORBIDDEN,
                "AccessDenied",
            )
            .await;
        }
        put_identity_policy(
            &runtime.state,
            caller_application_id,
            CALLER_KEY,
            &identity_policy(&bucket_name, &full_actions, true),
        )
        .await;
        assert_eq!(
            runtime
                .send(
                    Method::DELETE,
                    &format!("/{bucket_name}/abort.bin?uploadId={abort_upload_id}"),
                    Vec::new(),
                    CALLER_KEY,
                    CALLER_SECRET,
                    &[],
                )
                .await
                .status(),
            StatusCode::NO_CONTENT,
        );

        let owner_audit = runtime
            .state
            .repository
            .list_audit(owner_application_id, 100)
            .await
            .expect("owner audit");
        for action in [
            "s3.multipart_created",
            "s3.object.uploaded",
            "s3.multipart_aborted",
        ] {
            assert!(
                owner_audit.iter().any(|event| {
                    event.action == action
                        && event.actor_type == "access_key"
                        && event.actor_id == CALLER_KEY
                }),
                "missing owner audit action {action}"
            );
        }
        let caller_audit = runtime
            .state
            .repository
            .list_audit(caller_application_id, 100)
            .await
            .expect("caller audit");
        assert!(!caller_audit.iter().any(|event| {
            event.actor_id == CALLER_KEY
                && matches!(
                    event.action.as_str(),
                    "s3.multipart_created" | "s3.object.uploaded" | "s3.multipart_aborted"
                )
        }));

        runtime.stop().await;
    }
}
