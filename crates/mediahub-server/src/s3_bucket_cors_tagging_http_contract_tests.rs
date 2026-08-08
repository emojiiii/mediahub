mod bucket_cors_tagging_http_contract {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use base64::{Engine, engine::general_purpose::STANDARD};
    use mediahub_app::{
        ApplicationRepository, AuthRepository, S3BucketCorsRepository, S3BucketTaggingRepository,
        S3IdentityPolicyRepository,
    };
    use mediahub_core::{ApplicationId, OffsetDateTime, UserId};
    use sha2::{Digest, Sha256};

    use super::http_contract::{
        assert_s3_error, create_data_policy_identity, data_policy_test_state,
        install_bucket_policy, install_identity_policy, send_data_policy_request,
    };
    use super::*;

    const CORS_ORIGIN: &str = "https://app.example.com";

    struct TestAccount {
        application_id: ApplicationId,
        account_id: String,
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
            .expect("create CORS/Tagging contract user");
        state
            .repository
            .create_application(
                application_id,
                user_id,
                "S3 Bucket CORS and Tagging",
                &format!("app_{}", application_id.as_uuid().simple()),
                64 * 1024 * 1024,
                now,
            )
            .await
            .expect("create CORS/Tagging contract application");
        create_data_policy_identity(state, application_id, access_key_id, secret).await;
        let identity = state
            .repository
            .get_s3_identity_policy(access_key_id)
            .await
            .expect("load CORS/Tagging identity")
            .expect("CORS/Tagging access key identity");
        TestAccount {
            application_id,
            account_id: identity.identity.account_id.as_str().to_owned(),
            access_key_id,
            secret,
        }
    }

    fn sign_exact_payload(request: &mut http::Request<Vec<u8>>, access_key_id: &str, secret: &str) {
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
            "prismark-s3-cors-tagging-http-contract",
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
            .expect("CORS/Tagging signing params")
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
                    value.to_str().expect("CORS/Tagging request header"),
                )
            }),
            aws_sigv4::http_request::SignableBody::Precomputed(payload_sha256),
        )
        .expect("CORS/Tagging signable request");
        aws_sigv4::http_request::sign(signable, &params)
            .expect("CORS/Tagging signature")
            .into_parts()
            .0
            .apply_to_request_http1x(request);
    }

    #[allow(clippy::too_many_arguments)]
    async fn send_signed(
        client: &reqwest::Client,
        address: SocketAddr,
        account: &TestAccount,
        method: Method,
        path_and_query: &str,
        body: Vec<u8>,
        content_md5: bool,
        expected_owner: Option<&str>,
        extra_headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut builder = http::Request::builder()
            .method(method)
            .uri(format!("http://{address}{path_and_query}"))
            .header("host", address.to_string());
        if !body.is_empty() {
            builder = builder.header(CONTENT_LENGTH, body.len());
        }
        if content_md5 {
            builder = builder.header("content-md5", STANDARD.encode(md5::Md5::digest(&body)));
        }
        if let Some(expected_owner) = expected_owner {
            builder = builder.header(S3_EXPECTED_BUCKET_OWNER_HEADER, expected_owner);
        }
        for (name, value) in extra_headers {
            builder = builder.header(*name, *value);
        }
        let mut request = builder.body(body).expect("CORS/Tagging HTTP request");
        sign_exact_payload(&mut request, account.access_key_id, account.secret);
        send_data_policy_request(client, request).await
    }

    async fn start_server(
        state: Arc<AppState>,
    ) -> (SocketAddr, reqwest::Client, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("CORS/Tagging HTTP listener");
        let address = listener
            .local_addr()
            .expect("CORS/Tagging listener address");
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                crate::s3_router::router(state).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .expect("CORS/Tagging HTTP server");
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .expect("CORS/Tagging HTTP client");
        (address, client, server)
    }

    async fn create_bucket(
        client: &reqwest::Client,
        address: SocketAddr,
        owner: &TestAccount,
        bucket_name: &str,
    ) {
        assert_eq!(
            send_signed(
                client,
                address,
                owner,
                Method::PUT,
                &format!("/{bucket_name}"),
                Vec::new(),
                false,
                None,
                &[],
            )
            .await
            .status(),
            StatusCode::OK,
            "CreateBucket {bucket_name}",
        );
    }

    fn cors_xml() -> Vec<u8> {
        format!(
            r#"<CORSConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><CORSRule><ID>first</ID><AllowedOrigin>{CORS_ORIGIN}</AllowedOrigin><AllowedMethod>GET</AllowedMethod><AllowedMethod>HEAD</AllowedMethod><AllowedHeader>x-amz-*</AllowedHeader><ExposeHeader>x-first</ExposeHeader><MaxAgeSeconds>10</MaxAgeSeconds></CORSRule><CORSRule><ID>second</ID><AllowedOrigin>{CORS_ORIGIN}</AllowedOrigin><AllowedMethod>GET</AllowedMethod><AllowedHeader>*</AllowedHeader><ExposeHeader>x-second</ExposeHeader><MaxAgeSeconds>20</MaxAgeSeconds></CORSRule></CORSConfiguration>"#
        )
        .into_bytes()
    }

    fn tagging_xml(count: usize) -> Vec<u8> {
        let mut xml =
            String::from(r#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet>"#);
        for index in 0..count {
            xml.push_str(&format!(
                "<Tag><Key>key-{index:02}</Key><Value>value-{index:02}</Value></Tag>"
            ));
        }
        xml.push_str("</TagSet></Tagging>");
        xml.into_bytes()
    }

    async fn assert_target_audit(
        pool: &sqlx::PgPool,
        owner_application_id: ApplicationId,
        actor_access_key_id: &str,
        bucket_name: &str,
        action: &str,
    ) {
        let row: (String, String, String, String) = sqlx::query_as(
            r#"
            SELECT application_id::text, actor_id, target_type, summary->>'name'
              FROM audit_logs
             WHERE action = $1
             ORDER BY created_at DESC, id DESC
             LIMIT 1
            "#,
        )
        .bind(action)
        .fetch_one(pool)
        .await
        .expect("load CORS/Tagging audit event");
        assert_eq!(row.0, owner_application_id.to_string());
        assert_eq!(row.1, actor_access_key_id);
        assert_eq!(row.2, "bucket");
        assert_eq!(row.3, bucket_name);
    }

    async fn install_owner_actions(state: &AppState, owner: &TestAccount, bucket_name: &str) {
        install_identity_policy(
            state,
            owner.application_id,
            owner.access_key_id,
            &serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Action": [
                        "s3:CreateBucket",
                        "s3:GetBucketCORS",
                        "s3:PutBucketCORS",
                        "s3:GetBucketTagging",
                        "s3:PutBucketTagging"
                    ],
                    "Resource": ["*", format!("arn:aws:s3:::{bucket_name}")]
                }
            })
            .to_string(),
        )
        .await;
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn cors_and_tagging_round_trip_require_md5_expected_owner_and_target_audit(
        pool: sqlx::PgPool,
    ) {
        let audit_pool = pool.clone();
        let (state, storage_root) = data_policy_test_state(pool).await;
        let owner = create_account(
            &state,
            "s3-cors-tagging-owner@example.com",
            "mh_ak_cors_tagging_owner",
            "cors-tagging-owner-secret",
        )
        .await;
        let bucket_name = "cors-tagging-round-trip";
        install_owner_actions(&state, &owner, bucket_name).await;
        let (address, client, server) = start_server(Arc::clone(&state)).await;
        create_bucket(&client, address, &owner, bucket_name).await;

        for (subresource, code) in [
            ("cors", "NoSuchCORSConfiguration"),
            ("tagging", "NoSuchTagSet"),
        ] {
            assert_s3_error(
                send_signed(
                    &client,
                    address,
                    &owner,
                    Method::GET,
                    &format!("/{bucket_name}?{subresource}"),
                    Vec::new(),
                    false,
                    Some(&owner.account_id),
                    &[],
                )
                .await,
                StatusCode::NOT_FOUND,
                code,
            )
            .await;
        }

        let cors = cors_xml();
        assert_s3_error(
            send_signed(
                &client,
                address,
                &owner,
                Method::PUT,
                &format!("/{bucket_name}?cors"),
                cors.clone(),
                false,
                None,
                &[],
            )
            .await,
            StatusCode::BAD_REQUEST,
            "InvalidDigest",
        )
        .await;
        assert_s3_error(
            send_signed(
                &client,
                address,
                &owner,
                Method::PUT,
                &format!("/{bucket_name}?cors"),
                cors.clone(),
                true,
                Some("000000000000"),
                &[],
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_eq!(
            send_signed(
                &client,
                address,
                &owner,
                Method::PUT,
                &format!("/{bucket_name}?cors"),
                cors,
                true,
                Some(&owner.account_id),
                &[],
            )
            .await
            .status(),
            StatusCode::OK,
        );
        let persisted_cors = state
            .repository
            .get_s3_bucket_cors(owner.application_id, bucket_name)
            .await
            .expect("load persisted Bucket CORS")
            .expect("Bucket CORS bucket");
        assert_eq!(persisted_cors.revision, 1);
        assert_eq!(
            persisted_cors
                .configuration
                .as_ref()
                .expect("persisted Bucket CORS")
                .rules()
                .len(),
            2
        );
        let cors_get = send_signed(
            &client,
            address,
            &owner,
            Method::GET,
            &format!("/{bucket_name}?cors"),
            Vec::new(),
            false,
            Some(&owner.account_id),
            &[],
        )
        .await;
        assert_eq!(cors_get.status(), StatusCode::OK);
        let cors_body = cors_get.text().await.expect("GetBucketCors XML");
        assert!(cors_body.contains("<ID>first</ID>"), "{cors_body}");
        assert!(
            cors_body.contains("<ExposeHeader>x-second</ExposeHeader>"),
            "{cors_body}"
        );
        assert_eq!(
            send_signed(
                &client,
                address,
                &owner,
                Method::DELETE,
                &format!("/{bucket_name}?cors"),
                Vec::new(),
                false,
                Some(&owner.account_id),
                &[],
            )
            .await
            .status(),
            StatusCode::NO_CONTENT,
        );
        let deleted_cors = state
            .repository
            .get_s3_bucket_cors(owner.application_id, bucket_name)
            .await
            .expect("load deleted Bucket CORS")
            .expect("Bucket CORS bucket");
        assert_eq!(deleted_cors.revision, 2);
        assert!(deleted_cors.configuration.is_none());
        assert_s3_error(
            send_signed(
                &client,
                address,
                &owner,
                Method::GET,
                &format!("/{bucket_name}?cors"),
                Vec::new(),
                false,
                None,
                &[],
            )
            .await,
            StatusCode::NOT_FOUND,
            "NoSuchCORSConfiguration",
        )
        .await;

        let fifty_tags = tagging_xml(50);
        assert_s3_error(
            send_signed(
                &client,
                address,
                &owner,
                Method::PUT,
                &format!("/{bucket_name}?tagging"),
                fifty_tags.clone(),
                false,
                None,
                &[],
            )
            .await,
            StatusCode::BAD_REQUEST,
            "InvalidDigest",
        )
        .await;
        assert_eq!(
            send_signed(
                &client,
                address,
                &owner,
                Method::PUT,
                &format!("/{bucket_name}?tagging"),
                fifty_tags,
                true,
                Some(&owner.account_id),
                &[],
            )
            .await
            .status(),
            StatusCode::OK,
        );
        let persisted_tags = state
            .repository
            .get_s3_bucket_tagging(owner.application_id, bucket_name)
            .await
            .expect("load persisted Bucket Tagging")
            .expect("Bucket Tagging bucket");
        assert_eq!(persisted_tags.revision, 1);
        assert_eq!(
            persisted_tags
                .tags
                .as_ref()
                .expect("persisted Bucket Tagging")
                .iter()
                .count(),
            50
        );
        let tagging_get = send_signed(
            &client,
            address,
            &owner,
            Method::GET,
            &format!("/{bucket_name}?tagging"),
            Vec::new(),
            false,
            Some(&owner.account_id),
            &[],
        )
        .await;
        assert_eq!(tagging_get.status(), StatusCode::OK);
        let tagging_body = tagging_get.text().await.expect("GetBucketTagging XML");
        assert_eq!(tagging_body.matches("<Tag>").count(), 50, "{tagging_body}");
        assert!(tagging_body.contains("<Key>key-49</Key>"), "{tagging_body}");
        assert_eq!(
            send_signed(
                &client,
                address,
                &owner,
                Method::DELETE,
                &format!("/{bucket_name}?tagging"),
                Vec::new(),
                false,
                Some(&owner.account_id),
                &[],
            )
            .await
            .status(),
            StatusCode::NO_CONTENT,
        );
        let deleted_tags = state
            .repository
            .get_s3_bucket_tagging(owner.application_id, bucket_name)
            .await
            .expect("load deleted Bucket Tagging")
            .expect("Bucket Tagging bucket");
        assert_eq!(deleted_tags.revision, 2);
        assert!(deleted_tags.tags.is_none());
        assert_s3_error(
            send_signed(
                &client,
                address,
                &owner,
                Method::GET,
                &format!("/{bucket_name}?tagging"),
                Vec::new(),
                false,
                None,
                &[],
            )
            .await,
            StatusCode::NOT_FOUND,
            "NoSuchTagSet",
        )
        .await;

        for action in [
            "s3.bucket.cors_updated",
            "s3.bucket.cors_deleted",
            "s3.bucket.tagging_updated",
            "s3.bucket.tagging_deleted",
        ] {
            assert_target_audit(
                &audit_pool,
                owner.application_id,
                owner.access_key_id,
                bucket_name,
                action,
            )
            .await;
        }

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(storage_root);
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn cross_account_bucket_policy_allows_exact_actions_and_explicit_deny_wins(
        pool: sqlx::PgPool,
    ) {
        let audit_pool = pool.clone();
        let (state, storage_root) = data_policy_test_state(pool).await;
        let owner = create_account(
            &state,
            "s3-cors-tagging-policy-owner@example.com",
            "mh_ak_cors_tagging_policy_owner",
            "cors-tagging-policy-owner-secret",
        )
        .await;
        let caller = create_account(
            &state,
            "s3-cors-tagging-policy-caller@example.com",
            "mh_ak_cors_tagging_policy_caller",
            "cors-tagging-policy-caller-secret",
        )
        .await;
        let bucket_name = "cors-tagging-cross-account";
        install_owner_actions(&state, &owner, bucket_name).await;
        let (address, client, server) = start_server(Arc::clone(&state)).await;
        create_bucket(&client, address, &owner, bucket_name).await;

        let actions = [
            "s3:GetBucketCORS",
            "s3:PutBucketCORS",
            "s3:GetBucketTagging",
            "s3:PutBucketTagging",
        ];
        install_identity_policy(
            &state,
            caller.application_id,
            caller.access_key_id,
            &serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Action": actions,
                    "Resource": format!("arn:aws:s3:::{bucket_name}")
                }
            })
            .to_string(),
        )
        .await;
        let principal = format!(
            "arn:aws:iam::{}:user/{}",
            caller.account_id, caller.access_key_id
        );
        install_bucket_policy(
            &state,
            owner.application_id,
            bucket_name,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Principal": {"AWS": principal},
                    "Action": actions,
                    "Resource": format!("arn:aws:s3:::{bucket_name}"),
                    "Condition": {"IpAddress": {"aws:SourceIp": "127.0.0.1/32"}}
                }
            }),
        )
        .await;

        assert_eq!(
            send_signed(
                &client,
                address,
                &caller,
                Method::PUT,
                &format!("/{bucket_name}?cors"),
                cors_xml(),
                true,
                Some(&owner.account_id),
                &[],
            )
            .await
            .status(),
            StatusCode::OK,
        );
        assert_eq!(
            send_signed(
                &client,
                address,
                &caller,
                Method::GET,
                &format!("/{bucket_name}?cors"),
                Vec::new(),
                false,
                Some(&owner.account_id),
                &[],
            )
            .await
            .status(),
            StatusCode::OK,
        );
        assert_eq!(
            send_signed(
                &client,
                address,
                &caller,
                Method::PUT,
                &format!("/{bucket_name}?tagging"),
                tagging_xml(1),
                true,
                Some(&owner.account_id),
                &[],
            )
            .await
            .status(),
            StatusCode::OK,
        );
        assert!(
            state
                .repository
                .get_s3_bucket_cors(caller.application_id, bucket_name)
                .await
                .expect("caller-tenant CORS lookup")
                .is_none(),
            "cross-account CORS must persist under the target tenant",
        );
        assert!(
            state
                .repository
                .get_s3_bucket_tagging(caller.application_id, bucket_name)
                .await
                .expect("caller-tenant Tagging lookup")
                .is_none(),
            "cross-account Tagging must persist under the target tenant",
        );
        assert_target_audit(
            &audit_pool,
            owner.application_id,
            caller.access_key_id,
            bucket_name,
            "s3.bucket.cors_updated",
        )
        .await;
        assert_target_audit(
            &audit_pool,
            owner.application_id,
            caller.access_key_id,
            bucket_name,
            "s3.bucket.tagging_updated",
        )
        .await;

        install_bucket_policy(
            &state,
            owner.application_id,
            bucket_name,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": [
                    {
                        "Effect": "Allow",
                        "Principal": {"AWS": principal},
                        "Action": actions,
                        "Resource": format!("arn:aws:s3:::{bucket_name}")
                    },
                    {
                        "Effect": "Deny",
                        "Principal": {"AWS": principal},
                        "Action": ["s3:GetBucketCORS", "s3:PutBucketTagging"],
                        "Resource": format!("arn:aws:s3:::{bucket_name}")
                    }
                ]
            }),
        )
        .await;
        assert_s3_error(
            send_signed(
                &client,
                address,
                &caller,
                Method::GET,
                &format!("/{bucket_name}?cors"),
                Vec::new(),
                false,
                None,
                &[],
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_s3_error(
            send_signed(
                &client,
                address,
                &caller,
                Method::PUT,
                &format!("/{bucket_name}?tagging"),
                tagging_xml(2),
                true,
                None,
                &[],
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        let unchanged = state
            .repository
            .get_s3_bucket_tagging(owner.application_id, bucket_name)
            .await
            .expect("load owner Tagging after explicit Deny")
            .expect("owner Tagging bucket");
        assert_eq!(unchanged.revision, 1);
        assert_eq!(
            unchanged
                .tags
                .as_ref()
                .expect("Tagging survives denied overwrite")
                .iter()
                .count(),
            1
        );

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(storage_root);
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn cors_options_and_actual_get_use_first_matching_rule_and_fail_closed(
        pool: sqlx::PgPool,
    ) {
        let (state, storage_root) = data_policy_test_state(pool).await;
        let owner = create_account(
            &state,
            "s3-cors-runtime-owner@example.com",
            "mh_ak_cors_runtime_owner",
            "cors-runtime-owner-secret",
        )
        .await;
        let bucket_name = "cors-runtime-contract";
        install_owner_actions(&state, &owner, bucket_name).await;
        let (address, client, server) = start_server(Arc::clone(&state)).await;
        create_bucket(&client, address, &owner, bucket_name).await;
        let bucket_url = format!("http://{address}/{bucket_name}");

        assert_s3_error(
            client
                .request(Method::OPTIONS, &bucket_url)
                .header("origin", CORS_ORIGIN)
                .header("access-control-request-method", "GET")
                .send()
                .await
                .expect("OPTIONS without CORS configuration"),
            StatusCode::FORBIDDEN,
            "AccessForbidden",
        )
        .await;
        assert_eq!(
            send_signed(
                &client,
                address,
                &owner,
                Method::PUT,
                &format!("/{bucket_name}?cors"),
                cors_xml(),
                true,
                None,
                &[],
            )
            .await
            .status(),
            StatusCode::OK,
        );

        let preflight = client
            .request(Method::OPTIONS, &bucket_url)
            .header("origin", CORS_ORIGIN)
            .header("access-control-request-method", "GET")
            .header("access-control-request-headers", "X-Amz-Date")
            .send()
            .await
            .expect("matching CORS preflight");
        assert_eq!(preflight.status(), StatusCode::OK);
        assert_eq!(
            preflight
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some(CORS_ORIGIN)
        );
        assert_eq!(
            preflight
                .headers()
                .get("access-control-allow-methods")
                .and_then(|value| value.to_str().ok()),
            Some("GET")
        );
        assert_eq!(
            preflight
                .headers()
                .get("access-control-expose-headers")
                .and_then(|value| value.to_str().ok()),
            Some("x-first")
        );
        assert_eq!(
            preflight
                .headers()
                .get("access-control-allow-headers")
                .and_then(|value| value.to_str().ok()),
            Some("X-Amz-Date")
        );
        assert_eq!(
            preflight
                .headers()
                .get("access-control-max-age")
                .and_then(|value| value.to_str().ok()),
            Some("10"),
            "the first matching rule must win",
        );

        let actual = send_signed(
            &client,
            address,
            &owner,
            Method::GET,
            &format!("/{bucket_name}?cors"),
            Vec::new(),
            false,
            None,
            &[("origin", CORS_ORIGIN)],
        )
        .await;
        assert_eq!(actual.status(), StatusCode::OK);
        assert_eq!(
            actual
                .headers()
                .get("access-control-allow-origin")
                .and_then(|value| value.to_str().ok()),
            Some(CORS_ORIGIN)
        );
        assert_eq!(
            actual
                .headers()
                .get("access-control-expose-headers")
                .and_then(|value| value.to_str().ok()),
            Some("x-first"),
            "the first matching rule must decorate the actual response",
        );

        assert_s3_error(
            client
                .request(Method::OPTIONS, &bucket_url)
                .header("origin", "https://denied.example.com")
                .header("access-control-request-method", "GET")
                .send()
                .await
                .expect("mismatched CORS preflight"),
            StatusCode::FORBIDDEN,
            "AccessForbidden",
        )
        .await;
        let mismatched_actual = send_signed(
            &client,
            address,
            &owner,
            Method::GET,
            &format!("/{bucket_name}?cors"),
            Vec::new(),
            false,
            None,
            &[("origin", "https://denied.example.com")],
        )
        .await;
        assert_eq!(mismatched_actual.status(), StatusCode::OK);
        assert!(
            mismatched_actual
                .headers()
                .get("access-control-allow-origin")
                .is_none()
        );
        assert!(
            mismatched_actual
                .headers()
                .get("access-control-expose-headers")
                .is_none()
        );

        server.abort();
        let _ = server.await;
        let _ = std::fs::remove_dir_all(storage_root);
    }
}
