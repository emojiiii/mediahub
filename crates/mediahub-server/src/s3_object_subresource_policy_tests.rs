#[test]
fn object_subresource_policy_actions_are_exact_and_version_aware() {
    use S3ObjectSubresourcePolicyOperation as Operation;

    for (operation, current, versioned) in [
        (
            Operation::GetTagging,
            S3PolicyAction::GetObjectTagging,
            S3PolicyAction::GetObjectVersionTagging,
        ),
        (
            Operation::PutTagging,
            S3PolicyAction::PutObjectTagging,
            S3PolicyAction::PutObjectVersionTagging,
        ),
        (
            Operation::DeleteTagging,
            S3PolicyAction::DeleteObjectTagging,
            S3PolicyAction::DeleteObjectVersionTagging,
        ),
        (
            Operation::GetAcl,
            S3PolicyAction::GetObjectAcl,
            S3PolicyAction::GetObjectVersionAcl,
        ),
        (
            Operation::PutAcl,
            S3PolicyAction::PutObjectAcl,
            S3PolicyAction::PutObjectVersionAcl,
        ),
    ] {
        assert_eq!(operation.policy_action(false), current);
        assert_eq!(operation.policy_action(true), versioned);
    }

    for (operation, action) in [
        (Operation::GetRetention, S3PolicyAction::GetObjectRetention),
        (Operation::PutRetention, S3PolicyAction::PutObjectRetention),
        (
            Operation::BypassGovernanceRetention,
            S3PolicyAction::BypassGovernanceRetention,
        ),
        (Operation::GetLegalHold, S3PolicyAction::GetObjectLegalHold),
        (Operation::PutLegalHold, S3PolicyAction::PutObjectLegalHold),
    ] {
        assert_eq!(operation.policy_action(false), action);
        assert_eq!(operation.policy_action(true), action);
    }
}

mod object_subresource_http_contract {
    use std::sync::Arc;

    use mediahub_app::{
        ApplicationRepository, AuthRepository, S3IdentityPolicyRepository,
    };
    use mediahub_core::{ApplicationId, OffsetDateTime, UserId};

    use super::*;

    async fn create_application(state: &AppState, email: &str, name: &str) -> ApplicationId {
        let now = OffsetDateTime::now_utc();
        let user_id = UserId::new();
        let application_id = ApplicationId::new();
        state
            .repository
            .create_user(user_id, email, "hashed", now)
            .await
            .expect("create policy contract user");
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
            .expect("create policy contract application");
        application_id
    }

    fn unsigned_request(
        method: Method,
        url: &str,
        host: &str,
        body: Vec<u8>,
    ) -> http::Request<Vec<u8>> {
        http::Request::builder()
            .method(method)
            .uri(url)
            .header("host", host)
            .body(body)
            .expect("S3 object subresource request")
    }

    fn signed_request(
        method: Method,
        url: &str,
        host: &str,
        body: Vec<u8>,
        access_key_id: &str,
        secret: &str,
    ) -> http::Request<Vec<u8>> {
        let mut request = unsigned_request(method, url, host, body);
        super::http_contract::sign_data_policy_request(&mut request, access_key_id, secret);
        request
    }

    async fn expect_s3_error(response: reqwest::Response, code: &str) {
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response.text().await.expect("S3 policy error XML");
        assert!(body.contains(&format!("<Code>{code}</Code>")), "{body}");
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn object_subresources_enforce_version_deny_no_fallback_and_retention_dual_action(
        pool: sqlx::PgPool,
    ) {
        let (state, storage_root) =
            super::http_contract::data_policy_test_state(pool.clone()).await;
        let owner_application_id =
            create_application(&state, "s3-subresource-owner@example.com", "Owner").await;
        let caller_application_id =
            create_application(&state, "s3-subresource-caller@example.com", "Caller").await;
        let owner_key = "mh_ak_subresource_owner";
        let owner_secret = "subresource-owner-secret";
        let caller_key = "mh_ak_subresource_caller";
        let caller_secret = "subresource-caller-secret";
        super::http_contract::create_data_policy_identity(
            &state,
            owner_application_id,
            owner_key,
            owner_secret,
        )
        .await;
        super::http_contract::create_data_policy_identity(
            &state,
            caller_application_id,
            caller_key,
            caller_secret,
        )
        .await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("S3 object subresource listener");
        let address = listener.local_addr().expect("S3 object subresource address");
        let server = tokio::spawn({
            let application = crate::s3_router::router(Arc::clone(&state));
            async move {
                axum::serve(
                    listener,
                    application.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .expect("S3 object subresource server");
            }
        });
        let client = reqwest::Client::new();
        let bucket_name = "subresource-policy-assets";
        let object_key = "locked.txt";
        let bucket_url = format!("http://{address}/{bucket_name}");
        let object_url = format!("{bucket_url}/{object_key}");

        let mut create_bucket = unsigned_request(
            Method::PUT,
            &bucket_url,
            &address.to_string(),
            Vec::new(),
        );
        create_bucket.headers_mut().insert(
            HeaderName::from_static("x-amz-bucket-object-lock-enabled"),
            HeaderValue::from_static("true"),
        );
        super::http_contract::sign_data_policy_request(
            &mut create_bucket,
            owner_key,
            owner_secret,
        );
        assert_eq!(
            super::http_contract::send_data_policy_request(&client, create_bucket)
                .await
                .status(),
            StatusCode::OK,
        );
        super::http_contract::install_identity_policy(
            &state,
            owner_application_id,
            owner_key,
            &format!(
                r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Action":"s3:PutObject","Resource":"arn:aws:s3:::{bucket_name}/{object_key}"}}}}"#
            ),
        )
        .await;
        let put_object = signed_request(
            Method::PUT,
            &object_url,
            &address.to_string(),
            b"locked-data".to_vec(),
            owner_key,
            owner_secret,
        );
        let put_object = super::http_contract::send_data_policy_request(&client, put_object).await;
        assert_eq!(put_object.status(), StatusCode::OK);
        let version_id = put_object
            .headers()
            .get("x-amz-version-id")
            .and_then(|value| value.to_str().ok())
            .expect("PutObject version ID")
            .to_owned();

        let version_tagging_url = format!("{object_url}?tagging&versionId={version_id}");
        let no_policy = signed_request(
            Method::GET,
            &version_tagging_url,
            &address.to_string(),
            Vec::new(),
            caller_key,
            caller_secret,
        );
        expect_s3_error(
            super::http_contract::send_data_policy_request(&client, no_policy).await,
            "AccessDenied",
        )
        .await;

        let version_tagging_identity = format!(
            r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Action":"s3:GetObjectVersionTagging","Resource":"arn:aws:s3:::{bucket_name}/{object_key}"}}}}"#
        );
        super::http_contract::install_identity_policy(
            &state,
            caller_application_id,
            caller_key,
            &version_tagging_identity,
        )
        .await;
        let caller = state
            .repository
            .get_s3_identity_policy(caller_key)
            .await
            .expect("caller identity lookup")
            .expect("caller identity snapshot");
        let caller_arn = format!(
            "arn:aws:iam::{}:user/{caller_key}",
            caller.identity.account_id.as_str()
        );
        super::http_contract::install_bucket_policy(
            &state,
            owner_application_id,
            bucket_name,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Principal": {"AWS": caller_arn},
                    "Action": "s3:GetObjectVersionTagging",
                    "Resource": format!("arn:aws:s3:::{bucket_name}/{object_key}")
                }
            }),
        )
        .await;
        let version_allowed = signed_request(
            Method::GET,
            &version_tagging_url,
            &address.to_string(),
            Vec::new(),
            caller_key,
            caller_secret,
        );
        assert_eq!(
            super::http_contract::send_data_policy_request(&client, version_allowed)
                .await
                .status(),
            StatusCode::OK,
        );
        let current_action_denied = signed_request(
            Method::GET,
            &format!("{object_url}?tagging"),
            &address.to_string(),
            Vec::new(),
            caller_key,
            caller_secret,
        );
        expect_s3_error(
            super::http_contract::send_data_policy_request(&client, current_action_denied).await,
            "AccessDenied",
        )
        .await;

        super::http_contract::install_bucket_policy(
            &state,
            owner_application_id,
            bucket_name,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": [
                    {
                        "Effect": "Allow",
                        "Principal": {"AWS": caller_arn},
                        "Action": "s3:GetObjectVersionTagging",
                        "Resource": format!("arn:aws:s3:::{bucket_name}/{object_key}")
                    },
                    {
                        "Effect": "Deny",
                        "Principal": {"AWS": caller_arn},
                        "Action": "s3:GetObjectVersionTagging",
                        "Resource": format!("arn:aws:s3:::{bucket_name}/{object_key}")
                    }
                ]
            }),
        )
        .await;
        let bucket_denied = signed_request(
            Method::GET,
            &version_tagging_url,
            &address.to_string(),
            Vec::new(),
            caller_key,
            caller_secret,
        );
        expect_s3_error(
            super::http_contract::send_data_policy_request(&client, bucket_denied).await,
            "AccessDenied",
        )
        .await;

        let retention_resource = format!("arn:aws:s3:::{bucket_name}/{object_key}");
        let retention_only_identity = format!(
            r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Action":"s3:PutObjectRetention","Resource":"{retention_resource}"}}}}"#
        );
        super::http_contract::install_identity_policy(
            &state,
            caller_application_id,
            caller_key,
            &retention_only_identity,
        )
        .await;
        super::http_contract::install_bucket_policy(
            &state,
            owner_application_id,
            bucket_name,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Principal": {"AWS": caller_arn},
                    "Action": ["s3:PutObjectRetention", "s3:BypassGovernanceRetention"],
                    "Resource": retention_resource
                }
            }),
        )
        .await;
        let retention_xml = br#"<Retention xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Mode>GOVERNANCE</Mode><RetainUntilDate>2037-01-01T00:00:00Z</RetainUntilDate></Retention>"#.to_vec();
        let retention_url = format!("{object_url}?retention&versionId={version_id}");
        let mut missing_bypass_action = unsigned_request(
            Method::PUT,
            &retention_url,
            &address.to_string(),
            retention_xml.clone(),
        );
        missing_bypass_action.headers_mut().insert(
            HeaderName::from_static("content-md5"),
            HeaderValue::from_str(&STANDARD.encode(<md5::Md5 as md5::Digest>::digest(
                &retention_xml,
            )))
            .expect("retention MD5"),
        );
        missing_bypass_action.headers_mut().insert(
            HeaderName::from_static("x-amz-bypass-governance-retention"),
            HeaderValue::from_static("true"),
        );
        super::http_contract::sign_data_policy_request(
            &mut missing_bypass_action,
            caller_key,
            caller_secret,
        );
        expect_s3_error(
            super::http_contract::send_data_policy_request(&client, missing_bypass_action).await,
            "AccessDenied",
        )
        .await;

        let retention_with_bypass_identity = format!(
            r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Action":["s3:PutObjectRetention","s3:BypassGovernanceRetention"],"Resource":"{retention_resource}"}}}}"#
        );
        super::http_contract::install_identity_policy(
            &state,
            caller_application_id,
            caller_key,
            &retention_with_bypass_identity,
        )
        .await;
        let mut retention_allowed = unsigned_request(
            Method::PUT,
            &retention_url,
            &address.to_string(),
            retention_xml.clone(),
        );
        retention_allowed.headers_mut().insert(
            HeaderName::from_static("content-md5"),
            HeaderValue::from_str(&STANDARD.encode(<md5::Md5 as md5::Digest>::digest(
                &retention_xml,
            )))
            .expect("retention MD5"),
        );
        retention_allowed.headers_mut().insert(
            HeaderName::from_static("x-amz-bypass-governance-retention"),
            HeaderValue::from_static("true"),
        );
        super::http_contract::sign_data_policy_request(
            &mut retention_allowed,
            caller_key,
            caller_secret,
        );
        assert_eq!(
            super::http_contract::send_data_policy_request(&client, retention_allowed)
                .await
                .status(),
            StatusCode::OK,
        );

        let remaining_actions = [
            "s3:GetObjectVersionTagging",
            "s3:PutObjectVersionTagging",
            "s3:DeleteObjectVersionTagging",
            "s3:GetObjectVersionAcl",
            "s3:PutObjectVersionAcl",
            "s3:GetObjectRetention",
            "s3:GetObjectLegalHold",
            "s3:PutObjectLegalHold",
        ];
        let remaining_identity_policy = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": {
                "Effect": "Allow",
                "Action": remaining_actions,
                "Resource": retention_resource
            }
        })
        .to_string();
        super::http_contract::install_identity_policy(
            &state,
            caller_application_id,
            caller_key,
            &remaining_identity_policy,
        )
        .await;
        super::http_contract::install_bucket_policy(
            &state,
            owner_application_id,
            bucket_name,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Principal": {"AWS": caller_arn},
                    "Action": remaining_actions,
                    "Resource": retention_resource
                }
            }),
        )
        .await;

        let tagging_xml = br#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet><Tag><Key>policy</Key><Value>allowed</Value></Tag></TagSet></Tagging>"#.to_vec();
        let mut put_tagging = unsigned_request(
            Method::PUT,
            &version_tagging_url,
            &address.to_string(),
            tagging_xml.clone(),
        );
        put_tagging.headers_mut().insert(
            HeaderName::from_static("content-md5"),
            HeaderValue::from_str(&STANDARD.encode(<md5::Md5 as md5::Digest>::digest(
                &tagging_xml,
            )))
            .expect("tagging MD5"),
        );
        super::http_contract::sign_data_policy_request(
            &mut put_tagging,
            caller_key,
            caller_secret,
        );
        assert_eq!(
            super::http_contract::send_data_policy_request(&client, put_tagging)
                .await
                .status(),
            StatusCode::OK,
        );
        let get_tagging = signed_request(
            Method::GET,
            &version_tagging_url,
            &address.to_string(),
            Vec::new(),
            caller_key,
            caller_secret,
        );
        let get_tagging =
            super::http_contract::send_data_policy_request(&client, get_tagging).await;
        assert_eq!(get_tagging.status(), StatusCode::OK);
        assert!(
            get_tagging
                .text()
                .await
                .expect("GetObjectTagging XML")
                .contains("<Key>policy</Key><Value>allowed</Value>")
        );
        let delete_tagging = signed_request(
            Method::DELETE,
            &version_tagging_url,
            &address.to_string(),
            Vec::new(),
            caller_key,
            caller_secret,
        );
        assert_eq!(
            super::http_contract::send_data_policy_request(&client, delete_tagging)
                .await
                .status(),
            StatusCode::NO_CONTENT,
        );

        let acl_url = format!("{object_url}?acl&versionId={version_id}");
        let mut put_acl = unsigned_request(
            Method::PUT,
            &acl_url,
            &address.to_string(),
            Vec::new(),
        );
        put_acl.headers_mut().insert(
            HeaderName::from_static("x-amz-acl"),
            HeaderValue::from_static("private"),
        );
        super::http_contract::sign_data_policy_request(&mut put_acl, caller_key, caller_secret);
        let put_acl = super::http_contract::send_data_policy_request(&client, put_acl).await;
        let put_acl_status = put_acl.status();
        let put_acl_body = put_acl.text().await.expect("PutObjectAcl response body");
        assert_eq!(put_acl_status, StatusCode::OK, "{put_acl_body}");
        let get_acl = signed_request(
            Method::GET,
            &acl_url,
            &address.to_string(),
            Vec::new(),
            caller_key,
            caller_secret,
        );
        assert_eq!(
            super::http_contract::send_data_policy_request(&client, get_acl)
                .await
                .status(),
            StatusCode::OK,
        );

        let get_retention = signed_request(
            Method::GET,
            &retention_url,
            &address.to_string(),
            Vec::new(),
            caller_key,
            caller_secret,
        );
        assert_eq!(
            super::http_contract::send_data_policy_request(&client, get_retention)
                .await
                .status(),
            StatusCode::OK,
        );

        let legal_hold_url = format!("{object_url}?legal-hold&versionId={version_id}");
        let legal_hold_xml = br#"<LegalHold xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><Status>ON</Status></LegalHold>"#.to_vec();
        let mut put_legal_hold = unsigned_request(
            Method::PUT,
            &legal_hold_url,
            &address.to_string(),
            legal_hold_xml.clone(),
        );
        put_legal_hold.headers_mut().insert(
            HeaderName::from_static("content-md5"),
            HeaderValue::from_str(&STANDARD.encode(<md5::Md5 as md5::Digest>::digest(
                &legal_hold_xml,
            )))
            .expect("legal hold MD5"),
        );
        super::http_contract::sign_data_policy_request(
            &mut put_legal_hold,
            caller_key,
            caller_secret,
        );
        assert_eq!(
            super::http_contract::send_data_policy_request(&client, put_legal_hold)
                .await
                .status(),
            StatusCode::OK,
        );
        let get_legal_hold = signed_request(
            Method::GET,
            &legal_hold_url,
            &address.to_string(),
            Vec::new(),
            caller_key,
            caller_secret,
        );
        let get_legal_hold =
            super::http_contract::send_data_policy_request(&client, get_legal_hold).await;
        assert_eq!(get_legal_hold.status(), StatusCode::OK);
        assert!(
            get_legal_hold
                .text()
                .await
                .expect("GetObjectLegalHold XML")
                .contains("<Status>ON</Status>")
        );

        let owner_audit_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM audit_logs WHERE application_id = $1 AND actor_id = $2 AND action = 's3.object_retention_updated'",
        )
        .bind(owner_application_id.as_uuid())
        .bind(caller_key)
        .fetch_one(&pool)
        .await
        .expect("owner resource audit count");
        assert_eq!(owner_audit_count, 1);
        let caller_audit_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM audit_logs WHERE application_id = $1 AND actor_id = $2 AND action = 's3.object_retention_updated'",
        )
        .bind(caller_application_id.as_uuid())
        .bind(caller_key)
        .fetch_one(&pool)
        .await
        .expect("caller resource audit count");
        assert_eq!(caller_audit_count, 0);

        server.abort();
        let _ = server.await;
        drop(client);
        drop(state);
        std::fs::remove_dir_all(storage_root).expect("remove test object storage");
    }
}
