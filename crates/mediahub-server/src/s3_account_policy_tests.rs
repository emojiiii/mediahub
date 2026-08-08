mod account_policy_http_contract {
    use std::{net::SocketAddr, sync::Arc, time::Duration};

    use mediahub_app::{ApplicationRepository, AuthRepository, S3IdentityPolicyRepository};
    use mediahub_core::{ApplicationId, OffsetDateTime, UserId};

    use super::*;

    fn signed_account_request(
        method: Method,
        url: &str,
        host: &str,
        access_key_id: &str,
        secret: &str,
    ) -> http::Request<Vec<u8>> {
        let mut request = http::Request::builder()
            .method(method)
            .uri(url)
            .header("host", host)
            .body(Vec::new())
            .expect("S3 account policy request");
        super::http_contract::sign_data_policy_request(&mut request, access_key_id, secret);
        request
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn account_actions_require_explicit_identity_policy_without_legacy_fallback(
        pool: sqlx::PgPool,
    ) {
        let (state, storage_root) = super::http_contract::data_policy_test_state(pool).await;
        let now = OffsetDateTime::now_utc();
        let user_id = UserId::new();
        let application_id = ApplicationId::new();
        state
            .repository
            .create_user(user_id, "s3-account-policy@example.com", "hashed", now)
            .await
            .expect("create account policy user");
        state
            .repository
            .create_application(
                application_id,
                user_id,
                "S3 Account Policy",
                &format!("app_{}", application_id.as_uuid().simple()),
                64 * 1024 * 1024,
                now,
            )
            .await
            .expect("create account policy application");
        let access_key_id = "mh_ak_account_policy_test";
        let secret = "account-policy-test-secret";
        super::http_contract::create_data_policy_identity(
            &state,
            application_id,
            access_key_id,
            secret,
        )
        .await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("S3 account listener");
        let address = listener.local_addr().expect("S3 account address");
        let server = tokio::spawn({
            let application = crate::s3_router::router(Arc::clone(&state));
            async move {
                axum::serve(
                    listener,
                    application.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .expect("S3 account policy server");
            }
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("HTTP client");
        let root_url = format!("http://{address}/");
        let bucket_name = "account-policy-bucket";
        let bucket_url = format!("http://{address}/{bucket_name}");

        for request in [
            signed_account_request(Method::GET, &root_url, &address.to_string(), access_key_id, secret),
            signed_account_request(Method::PUT, &bucket_url, &address.to_string(), access_key_id, secret),
        ] {
            super::http_contract::assert_s3_error(
                super::http_contract::send_data_policy_request(&client, request).await,
                StatusCode::FORBIDDEN,
                "AccessDenied",
            )
            .await;
        }

        super::http_contract::install_identity_policy(
            &state,
            application_id,
            access_key_id,
            r#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Action":"s3:ListAllMyBuckets","Resource":"*"}}"#,
        )
        .await;
        let list = super::http_contract::send_data_policy_request(
            &client,
            signed_account_request(Method::GET, &root_url, &address.to_string(), access_key_id, secret),
        )
        .await;
        assert_eq!(list.status(), StatusCode::OK);
        let list_xml = list.text().await.expect("ListBuckets XML");
        let identity = state
            .repository
            .get_s3_identity_policy(access_key_id)
            .await
            .expect("account identity snapshot")
            .expect("account identity");
        assert!(list_xml.contains(identity.identity.account_id.as_str()));
        assert!(list_xml.contains("PrismArk Account"));
        super::http_contract::assert_s3_error(
            super::http_contract::send_data_policy_request(
                &client,
                signed_account_request(Method::PUT, &bucket_url, &address.to_string(), access_key_id, secret),
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        super::http_contract::install_identity_policy(
            &state,
            application_id,
            access_key_id,
            r#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Action":"s3:CreateBucket","Resource":"*","Condition":{"IpAddress":{"aws:SourceIp":"127.0.0.1/32"}}}}"#,
        )
        .await;
        let create = super::http_contract::send_data_policy_request(
            &client,
            signed_account_request(Method::PUT, &bucket_url, &address.to_string(), access_key_id, secret),
        )
        .await;
        assert_eq!(create.status(), StatusCode::OK);

        let lock_bucket_name = "account-policy-lock";
        let lock_bucket_url = format!("http://{address}/{lock_bucket_name}");
        let mut create_lock_bucket = http::Request::builder()
            .method(Method::PUT)
            .uri(&lock_bucket_url)
            .header("host", address.to_string())
            .header("x-amz-bucket-object-lock-enabled", "true")
            .body(Vec::new())
            .expect("CreateBucket Object Lock request");
        super::http_contract::sign_data_policy_request(
            &mut create_lock_bucket,
            access_key_id,
            secret,
        );
        super::http_contract::assert_s3_error(
            super::http_contract::send_data_policy_request(&client, create_lock_bucket).await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        super::http_contract::install_identity_policy(
            &state,
            application_id,
            access_key_id,
            &format!(
                r#"{{"Version":"2012-10-17","Statement":[{{"Effect":"Allow","Action":"s3:CreateBucket","Resource":"*"}},{{"Effect":"Allow","Action":["s3:PutBucketObjectLockConfiguration","s3:PutBucketVersioning"],"Resource":"arn:aws:s3:::{lock_bucket_name}"}}]}}"#
            ),
        )
        .await;
        let mut create_lock_bucket = http::Request::builder()
            .method(Method::PUT)
            .uri(&lock_bucket_url)
            .header("host", address.to_string())
            .header("x-amz-bucket-object-lock-enabled", "true")
            .body(Vec::new())
            .expect("authorized CreateBucket Object Lock request");
        super::http_contract::sign_data_policy_request(
            &mut create_lock_bucket,
            access_key_id,
            secret,
        );
        assert_eq!(
            super::http_contract::send_data_policy_request(&client, create_lock_bucket)
                .await
                .status(),
            StatusCode::OK
        );

        super::http_contract::install_identity_policy(
            &state,
            application_id,
            access_key_id,
            r#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Action":"s3:ListAllMyBuckets","Resource":"*"}}"#,
        )
        .await;
        let list = super::http_contract::send_data_policy_request(
            &client,
            signed_account_request(Method::GET, &root_url, &address.to_string(), access_key_id, secret),
        )
        .await;
        assert_eq!(list.status(), StatusCode::OK);
        let list_xml = list.text().await.expect("ListBuckets XML after create");
        assert!(list_xml.contains(bucket_name));
        assert!(list_xml.contains(lock_bucket_name));
        super::http_contract::assert_s3_error(
            super::http_contract::send_data_policy_request(
                &client,
                signed_account_request(Method::PUT, &bucket_url, &address.to_string(), access_key_id, secret),
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;

        server.abort();
        let _ = server.await;
        drop(client);
        drop(state);
        std::fs::remove_dir_all(storage_root).expect("remove account policy object storage");
    }
}
