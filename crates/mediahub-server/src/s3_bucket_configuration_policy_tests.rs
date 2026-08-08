mod bucket_configuration_policy_http_contract {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use base64::{Engine, engine::general_purpose::STANDARD};
    use mediahub_app::{
        ApplicationRepository, AuthRepository, S3BucketRepository, S3IdentityPolicyRepository,
    };
    use mediahub_core::{ApplicationId, OffsetDateTime, UserId, VersioningStatus};
    use sha2::{Digest, Sha256};

    use super::http_contract::{
        assert_s3_error, create_data_policy_identity, data_policy_test_state,
        install_bucket_policy, install_identity_policy, send_data_policy_request,
    };
    use super::*;

    struct TestAccount {
        application_id: ApplicationId,
        access_key_id: &'static str,
        secret: &'static str,
    }

    async fn create_account(
        state: &AppState,
        email: &str,
        access_key_id: &'static str,
        secret: &'static str,
    ) -> TestAccount {
        let now = OffsetDateTime::now_utc();
        let user_id = UserId::new();
        let application_id = ApplicationId::new();
        state
            .repository
            .create_user(user_id, email, "hashed", now)
            .await
            .expect("create Bucket configuration test user");
        state
            .repository
            .create_application(
                application_id,
                user_id,
                "S3 Bucket Configuration Policy",
                &format!("app_{}", application_id.as_uuid().simple()),
                64 * 1024 * 1024,
                now,
            )
            .await
            .expect("create Bucket configuration test application");
        create_data_policy_identity(state, application_id, access_key_id, secret).await;
        TestAccount {
            application_id,
            access_key_id,
            secret,
        }
    }

    fn sign_exact_payload(
        request: &mut http::Request<Vec<u8>>,
        access_key_id: &str,
        secret: &str,
    ) {
        let payload_sha256 = hex::encode(Sha256::digest(request.body()));
        request.headers_mut().insert(
            HeaderName::from_static("x-amz-content-sha256"),
            HeaderValue::from_str(&payload_sha256).expect("payload SHA-256 header"),
        );
        let identity = aws_credential_types::Credentials::new(
            access_key_id,
            secret,
            None,
            None,
            "prismark-s3-bucket-configuration-policy-test",
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
            .expect("S3 Bucket configuration signing params")
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
                    value.to_str().expect("S3 Bucket configuration header"),
                )
            }),
            aws_sigv4::http_request::SignableBody::Precomputed(payload_sha256),
        )
        .expect("S3 Bucket configuration signable request");
        aws_sigv4::http_request::sign(signable, &params)
            .expect("S3 Bucket configuration signature")
            .into_parts()
            .0
            .apply_to_request_http1x(request);
    }

    async fn send(
        client: &reqwest::Client,
        address: SocketAddr,
        account: &TestAccount,
        method: Method,
        path_and_query: &str,
        body: Vec<u8>,
        content_md5: bool,
    ) -> reqwest::Response {
        let mut request = http::Request::builder()
            .method(method)
            .uri(format!("http://{address}{path_and_query}"))
            .header("host", address.to_string())
            .header(CONTENT_LENGTH, body.len().to_string());
        if content_md5 {
            request = request.header("content-md5", STANDARD.encode(md5::Md5::digest(&body)));
        }
        let mut request = request
            .body(body)
            .expect("S3 Bucket configuration request");
        sign_exact_payload(&mut request, account.access_key_id, account.secret);
        send_data_policy_request(client, request).await
    }

    async fn create_bucket(
        client: &reqwest::Client,
        address: SocketAddr,
        owner: &TestAccount,
        bucket_name: &str,
        object_lock: bool,
    ) {
        let mut request = http::Request::builder()
            .method(Method::PUT)
            .uri(format!("http://{address}/{bucket_name}"))
            .header("host", address.to_string());
        if object_lock {
            request = request.header("x-amz-bucket-object-lock-enabled", "true");
        }
        let mut request = request.body(Vec::new()).expect("CreateBucket request");
        sign_exact_payload(&mut request, owner.access_key_id, owner.secret);
        let response = send_data_policy_request(client, request).await;
        assert_eq!(response.status(), StatusCode::OK, "CreateBucket {bucket_name}");
    }

    async fn grant_cross_account_action(
        state: &AppState,
        owner_application_id: ApplicationId,
        caller: &TestAccount,
        bucket_name: &str,
        action: &str,
    ) {
        let resource = format!("arn:aws:s3:::{bucket_name}");
        install_identity_policy(
            state,
            caller.application_id,
            caller.access_key_id,
            &serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Action": action,
                    "Resource": resource,
                }
            })
            .to_string(),
        )
        .await;
        let identity = state
            .repository
            .get_s3_identity_policy(caller.access_key_id)
            .await
            .expect("load caller identity")
            .expect("caller identity policy");
        let principal_arn = format!(
            "arn:aws:iam::{}:user/{}",
            identity.identity.account_id.as_str(),
            caller.access_key_id,
        );
        install_bucket_policy(
            state,
            owner_application_id,
            bucket_name,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Principal": {"AWS": principal_arn},
                    "Action": action,
                    "Resource": resource,
                    "Condition": {
                        "IpAddress": {"aws:SourceIp": "127.0.0.1/32"}
                    }
                }
            }),
        )
        .await;
    }

    async fn allow_anonymous_get_action(
        state: &AppState,
        owner_application_id: ApplicationId,
        bucket_name: &str,
        action: &str,
    ) {
        install_bucket_policy(
            state,
            owner_application_id,
            bucket_name,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Principal": "*",
                    "Action": action,
                    "Resource": format!("arn:aws:s3:::{bucket_name}"),
                    "Condition": {
                        "IpAddress": {"aws:SourceIp": "127.0.0.1/32"}
                    }
                }
            }),
        )
        .await;
    }

    async fn assert_bucket_audit(
        pool: &sqlx::PgPool,
        owner_application_id: ApplicationId,
        actor_access_key_id: &str,
        bucket_name: &str,
        action: &str,
    ) {
        let row: (String, String, String) = sqlx::query_as(
            r#"
            SELECT application_id::text, actor_id, target_id
            FROM audit_logs
            WHERE action = $1
            ORDER BY created_at DESC, id DESC
            LIMIT 1
            "#,
        )
        .bind(action)
        .fetch_one(pool)
        .await
        .expect("load Bucket configuration audit");
        assert_eq!(row.0, owner_application_id.to_string());
        assert_eq!(row.1, actor_access_key_id);
        assert_eq!(row.2, bucket_name);
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn bucket_configuration_actions_use_unified_policy_target_owner_and_exact_body_sigv4(
        pool: sqlx::PgPool,
    ) {
        let audit_pool = pool.clone();
        let (state, storage_root) = data_policy_test_state(pool).await;
        let owner = create_account(
            &state,
            "s3-bucket-configuration-owner@example.com",
            "mh_ak_bucket_configuration_owner",
            "bucket-configuration-owner-secret",
        )
        .await;
        let caller = create_account(
            &state,
            "s3-bucket-configuration-caller@example.com",
            "mh_ak_bucket_configuration_caller",
            "bucket-configuration-caller-secret",
        )
        .await;

        install_identity_policy(
            &state,
            owner.application_id,
            owner.access_key_id,
            r#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Action":["s3:CreateBucket","s3:PutBucketObjectLockConfiguration","s3:PutBucketVersioning"],"Resource":"*"}}"#,
        )
        .await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("S3 Bucket configuration listener");
        let address = listener.local_addr().expect("S3 listener address");
        let server = tokio::spawn({
            let application = crate::s3_router::router(Arc::clone(&state));
            async move {
                axum::serve(
                    listener,
                    application.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .expect("S3 Bucket configuration Policy server");
            }
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("S3 Bucket configuration HTTP client");
        let configuration_bucket = "configuration-policy-assets";
        let object_lock_bucket = "configuration-policy-locked";
        create_bucket(&client, address, &owner, configuration_bucket, false).await;
        create_bucket(&client, address, &owner, object_lock_bucket, true).await;

        assert_s3_error(
            send(
                &client,
                address,
                &caller,
                Method::GET,
                &format!("/{configuration_bucket}?versioning"),
                Vec::new(),
                false,
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        grant_cross_account_action(
            &state,
            owner.application_id,
            &caller,
            configuration_bucket,
            "s3:GetBucketVersioning",
        )
        .await;
        assert_eq!(
            send(
                &client,
                address,
                &caller,
                Method::GET,
                &format!("/{configuration_bucket}?versioning"),
                Vec::new(),
                false,
            )
            .await
            .status(),
            StatusCode::OK,
        );

        let versioning = br#"<VersioningConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Status>Enabled</Status></VersioningConfiguration>"#.to_vec();
        let mut tampered = http::Request::builder()
            .method(Method::PUT)
            .uri(format!("http://{address}/{configuration_bucket}?versioning"))
            .header("host", address.to_string())
            .header("content-md5", STANDARD.encode(md5::Md5::digest(&versioning)))
            .body(versioning.clone())
            .expect("signed Bucket configuration request");
        sign_exact_payload(&mut tampered, caller.access_key_id, caller.secret);
        let last = tampered.body().len() - 1;
        tampered.body_mut()[last] = b'X';
        assert_s3_error(
            send_data_policy_request(&client, tampered).await,
            StatusCode::FORBIDDEN,
            "XAmzContentSHA256Mismatch",
        )
        .await;

        grant_cross_account_action(
            &state,
            owner.application_id,
            &caller,
            configuration_bucket,
            "s3:PutBucketVersioning",
        )
        .await;
        assert_eq!(
            send(
                &client,
                address,
                &caller,
                Method::PUT,
                &format!("/{configuration_bucket}?versioning"),
                versioning,
                true,
            )
            .await
            .status(),
            StatusCode::OK,
        );
        assert_eq!(
            state
                .repository
                .get_s3_bucket_versioning(owner.application_id, configuration_bucket)
                .await
                .expect("owner versioning lookup"),
            Some(VersioningStatus::Enabled),
        );

        allow_anonymous_get_action(
            &state,
            owner.application_id,
            configuration_bucket,
            "s3:GetBucketVersioning",
        )
        .await;
        assert_eq!(
            client
                .get(format!(
                    "http://{address}/{configuration_bucket}?versioning"
                ))
                .send()
                .await
                .expect("anonymous GetBucketVersioning")
                .status(),
            StatusCode::OK,
        );
        assert_s3_error(
            client
                .get(format!(
                    "http://{address}/{configuration_bucket}?versioning"
                ))
                .header("x-amz-date", "partial")
                .send()
                .await
                .expect("partial SigV4 GetBucketVersioning"),
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        let lifecycle = br#"<LifecycleConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Rule><ID>expire-preview</ID><Filter><Prefix>preview/</Prefix></Filter><Status>Enabled</Status><Expiration><Days>30</Days></Expiration></Rule></LifecycleConfiguration>"#.to_vec();
        grant_cross_account_action(
            &state,
            owner.application_id,
            &caller,
            configuration_bucket,
            "s3:PutLifecycleConfiguration",
        )
        .await;
        assert_eq!(
            send(
                &client,
                address,
                &caller,
                Method::PUT,
                &format!("/{configuration_bucket}?lifecycle"),
                lifecycle,
                true,
            )
            .await
            .status(),
            StatusCode::OK,
        );

        allow_anonymous_get_action(
            &state,
            owner.application_id,
            configuration_bucket,
            "s3:GetLifecycleConfiguration",
        )
        .await;
        assert_eq!(
            client
                .get(format!(
                    "http://{address}/{configuration_bucket}?lifecycle"
                ))
                .send()
                .await
                .expect("anonymous GetLifecycleConfiguration")
                .status(),
            StatusCode::OK,
        );

        grant_cross_account_action(
            &state,
            owner.application_id,
            &caller,
            configuration_bucket,
            "s3:GetLifecycleConfiguration",
        )
        .await;
        let lifecycle_response = send(
            &client,
            address,
            &caller,
            Method::GET,
            &format!("/{configuration_bucket}?lifecycle"),
            Vec::new(),
            false,
        )
        .await;
        assert_eq!(lifecycle_response.status(), StatusCode::OK);
        assert!(
            lifecycle_response
                .text()
                .await
                .expect("Lifecycle XML")
                .contains("expire-preview")
        );

        grant_cross_account_action(
            &state,
            owner.application_id,
            &caller,
            configuration_bucket,
            "s3:DeleteLifecycleConfiguration",
        )
        .await;
        assert_eq!(
            send(
                &client,
                address,
                &caller,
                Method::DELETE,
                &format!("/{configuration_bucket}?lifecycle"),
                Vec::new(),
                false,
            )
            .await
            .status(),
            StatusCode::NO_CONTENT,
        );

        grant_cross_account_action(
            &state,
            owner.application_id,
            &caller,
            object_lock_bucket,
            "s3:GetBucketObjectLockConfiguration",
        )
        .await;
        assert_eq!(
            send(
                &client,
                address,
                &caller,
                Method::GET,
                &format!("/{object_lock_bucket}?object-lock"),
                Vec::new(),
                false,
            )
            .await
            .status(),
            StatusCode::OK,
        );

        let object_lock = br#"<ObjectLockConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><ObjectLockEnabled>Enabled</ObjectLockEnabled><Rule><DefaultRetention><Mode>GOVERNANCE</Mode><Days>30</Days></DefaultRetention></Rule></ObjectLockConfiguration>"#.to_vec();
        grant_cross_account_action(
            &state,
            owner.application_id,
            &caller,
            object_lock_bucket,
            "s3:PutBucketObjectLockConfiguration",
        )
        .await;
        assert_eq!(
            send(
                &client,
                address,
                &caller,
                Method::PUT,
                &format!("/{object_lock_bucket}?object-lock"),
                object_lock,
                true,
            )
            .await
            .status(),
            StatusCode::OK,
        );

        allow_anonymous_get_action(
            &state,
            owner.application_id,
            object_lock_bucket,
            "s3:GetBucketObjectLockConfiguration",
        )
        .await;
        assert_eq!(
            client
                .get(format!(
                    "http://{address}/{object_lock_bucket}?object-lock"
                ))
                .send()
                .await
                .expect("anonymous GetObjectLockConfiguration")
                .status(),
            StatusCode::OK,
        );

        let owner_configuration = state
            .repository
            .get_s3_bucket_configuration(owner.application_id, object_lock_bucket)
            .await
            .expect("owner Object Lock lookup")
            .expect("owner Object Lock bucket");
        assert!(owner_configuration.default_retention().is_some());
        assert!(
            state
                .repository
                .get_s3_bucket_configuration(caller.application_id, object_lock_bucket)
                .await
                .expect("caller Object Lock lookup")
                .is_none(),
            "cross-account mutation must not persist under the caller tenant",
        );

        for (bucket_name, action) in [
            (configuration_bucket, "s3.bucket.versioning_updated"),
            (configuration_bucket, "s3.bucket.lifecycle_updated"),
            (configuration_bucket, "s3.bucket.lifecycle_deleted"),
            (object_lock_bucket, "s3.bucket.object_lock_updated"),
        ] {
            assert_bucket_audit(
                &audit_pool,
                owner.application_id,
                caller.access_key_id,
                bucket_name,
                action,
            )
            .await;
        }

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(storage_root);
    }
}
