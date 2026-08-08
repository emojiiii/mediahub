mod put_copy_http_contract {
    use std::{sync::Arc, time::Duration};

    use mediahub_app::{
        ApplicationRepository, AuthRepository, DeleteS3IdentityPolicy, S3BucketRepository,
        S3IdentityPolicyRepository,
    };
    use mediahub_core::{ApplicationId, OffsetDateTime, UserId, VersioningStatus};

    use super::http_contract::{
        assert_s3_error, create_data_policy_identity, data_policy_test_state,
        install_bucket_policy, install_identity_policy, send_data_policy_request,
        sign_data_policy_request,
    };
    use super::*;

    struct TestAccount {
        application_id: ApplicationId,
        access_key_id: &'static str,
        secret: &'static str,
    }

    async fn create_test_account(
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
            .expect("create policy account user");
        state
            .repository
            .create_application(
                application_id,
                user_id,
                "S3 Put Copy Policy",
                &format!("app_{}", application_id.as_uuid().simple()),
                64 * 1024 * 1024,
                now,
            )
            .await
            .expect("create policy account application");
        create_data_policy_identity(state, application_id, access_key_id, secret).await;
        TestAccount {
            application_id,
            access_key_id,
            secret,
        }
    }

    async fn create_bucket(
        client: &reqwest::Client,
        address: std::net::SocketAddr,
        account: &TestAccount,
        bucket_name: &str,
    ) {
        let url = format!("http://{address}/{bucket_name}");
        let mut request = http::Request::builder()
            .method(Method::PUT)
            .uri(url)
            .header("host", address.to_string())
            .body(Vec::new())
            .expect("CreateBucket request");
        sign_data_policy_request(&mut request, account.access_key_id, account.secret);
        let response = send_data_policy_request(client, request).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "CreateBucket {bucket_name}"
        );
    }

    async fn put_object(
        client: &reqwest::Client,
        address: std::net::SocketAddr,
        account: &TestAccount,
        bucket_name: &str,
        object_key: &str,
        body: &[u8],
        extra_headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut builder = http::Request::builder()
            .method(Method::PUT)
            .uri(format!("http://{address}/{bucket_name}/{object_key}"))
            .header("host", address.to_string())
            .header(CONTENT_TYPE, "text/plain");
        for (name, value) in extra_headers {
            builder = builder.header(*name, *value);
        }
        let mut request = builder
            .body(body.to_vec())
            .expect("PutObject policy request");
        sign_data_policy_request(&mut request, account.access_key_id, account.secret);
        send_data_policy_request(client, request).await
    }

    async fn copy_object(
        client: &reqwest::Client,
        address: std::net::SocketAddr,
        account: &TestAccount,
        target_bucket: &str,
        target_key: &str,
        source: &str,
        extra_headers: &[(&str, &str)],
    ) -> reqwest::Response {
        let mut builder = http::Request::builder()
            .method(Method::PUT)
            .uri(format!("http://{address}/{target_bucket}/{target_key}"))
            .header("host", address.to_string())
            .header("x-amz-copy-source", source);
        for (name, value) in extra_headers {
            builder = builder.header(*name, *value);
        }
        let mut request = builder.body(Vec::new()).expect("CopyObject policy request");
        sign_data_policy_request(&mut request, account.access_key_id, account.secret);
        send_data_policy_request(client, request).await
    }

    async fn get_object(
        client: &reqwest::Client,
        address: std::net::SocketAddr,
        account: &TestAccount,
        bucket_name: &str,
        object_key: &str,
    ) -> reqwest::Response {
        let mut request = http::Request::builder()
            .method(Method::GET)
            .uri(format!("http://{address}/{bucket_name}/{object_key}"))
            .header("host", address.to_string())
            .body(Vec::new())
            .expect("GetObject policy request");
        sign_data_policy_request(&mut request, account.access_key_id, account.secret);
        send_data_policy_request(client, request).await
    }

    async fn assert_no_target_intent(pool: &sqlx::PgPool, object_key: &str) {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM s3_upload_intents WHERE object_key = $1",
        )
        .bind(object_key)
        .fetch_one(pool)
        .await
        .expect("count target upload intents");
        assert_eq!(count, 0, "unexpected upload intent for {object_key}");
    }

    async fn assert_resource_audit(
        pool: &sqlx::PgPool,
        action: &str,
        object_key: &str,
        owner: ApplicationId,
        actor_access_key: &str,
    ) {
        let row: (String, String) = sqlx::query_as(
            r#"
            SELECT application_id::text, actor_id
            FROM audit_logs
            WHERE action = $1
              AND summary ->> 'object_key' = $2
            ORDER BY created_at DESC, id DESC
            LIMIT 1
            "#,
        )
        .bind(action)
        .bind(object_key)
        .fetch_one(pool)
        .await
        .expect("load S3 resource audit");
        assert_eq!(row.0, owner.to_string());
        assert_eq!(row.1, actor_access_key);
    }

    #[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
    async fn put_and_copy_authorize_target_source_versions_and_resource_audits(
        pool: sqlx::PgPool,
    ) {
        let audit_pool = pool.clone();
        let (state, storage_root) = data_policy_test_state(pool).await;
        let owner = create_test_account(
            &state,
            "s3-put-copy-owner@example.com",
            "mh_ak_put_copy_owner",
            "put-copy-owner-secret",
        )
        .await;
        let external = create_test_account(
            &state,
            "s3-put-copy-external@example.com",
            "mh_ak_put_copy_external",
            "put-copy-external-secret",
        )
        .await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("S3 Put/Copy listener");
        let address = listener.local_addr().expect("S3 Put/Copy address");
        let server = tokio::spawn({
            let application = crate::s3_router::router(Arc::clone(&state));
            async move {
                axum::serve(
                    listener,
                    application.into_make_service_with_connect_info::<std::net::SocketAddr>(),
                )
                .await
                .expect("S3 Put/Copy policy server");
            }
        });
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("S3 Put/Copy HTTP client");
        let target_bucket = "put-copy-target";
        let external_bucket = "put-copy-external";
        for account in [&owner, &external] {
            install_identity_policy(
                &state,
                account.application_id,
                account.access_key_id,
                r#"{"Version":"2012-10-17","Statement":{"Effect":"Allow","Action":"s3:CreateBucket","Resource":"*"}}"#,
            )
            .await;
        }
        create_bucket(&client, address, &owner, target_bucket).await;
        create_bucket(&client, address, &external, external_bucket).await;

        assert_s3_error(
            put_object(
                &client,
                address,
                &owner,
                target_bucket,
                "no-policy-put.txt",
                b"legacy permission must not authorize",
                &[],
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_no_target_intent(&audit_pool, "no-policy-put.txt").await;

        let owner_policy = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [
                {
                    "Effect": "Allow",
                    "Action": "s3:PutObject",
                    "Resource": format!("arn:aws:s3:::{target_bucket}/*")
                },
                {
                    "Effect": "Allow",
                    "Action": ["s3:GetObject", "s3:GetObjectVersion"],
                    "Resource": [
                        format!("arn:aws:s3:::{target_bucket}/*"),
                        format!("arn:aws:s3:::{external_bucket}/*")
                    ]
                },
                {
                    "Effect": "Allow",
                    "Action": "s3:PutObjectTagging",
                    "Resource": format!(
                        "arn:aws:s3:::{target_bucket}/tagged-copy-source.txt"
                    )
                }
            ]
        });
        install_identity_policy(
            &state,
            owner.application_id,
            owner.access_key_id,
            &owner_policy.to_string(),
        )
        .await;
        let external_policy = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [
                {
                    "Effect": "Allow",
                    "Action": "s3:PutObject",
                    "Resource": [
                        format!("arn:aws:s3:::{external_bucket}/*"),
                        format!("arn:aws:s3:::{target_bucket}/cross-owner-*")
                    ]
                },
                {
                    "Effect": "Allow",
                    "Action": ["s3:GetObject", "s3:GetObjectVersion"],
                    "Resource": format!("arn:aws:s3:::{external_bucket}/*")
                }
            ]
        });
        install_identity_policy(
            &state,
            external.application_id,
            external.access_key_id,
            &external_policy.to_string(),
        )
        .await;
        state
            .repository
            .set_s3_bucket_versioning(
                external.application_id,
                external_bucket,
                VersioningStatus::Enabled,
                OffsetDateTime::now_utc(),
            )
            .await
            .expect("enable external source versioning");

        for (object_key, extra_headers) in [
            (
                "tagging-without-action.txt",
                vec![("x-amz-tagging", "color=blue")],
            ),
            (
                "acl-without-action.txt",
                vec![("x-amz-acl", "private")],
            ),
            (
                "retention-without-action.txt",
                vec![
                    ("x-amz-object-lock-mode", "GOVERNANCE"),
                    (
                        "x-amz-object-lock-retain-until-date",
                        "2099-01-01T00:00:00Z",
                    ),
                ],
            ),
            (
                "legal-hold-without-action.txt",
                vec![("x-amz-object-lock-legal-hold", "ON")],
            ),
        ] {
            assert_s3_error(
                put_object(
                    &client,
                    address,
                    &owner,
                    target_bucket,
                    object_key,
                    b"supplemental action required",
                    &extra_headers,
                )
                .await,
                StatusCode::FORBIDDEN,
                "AccessDenied",
            )
            .await;
            assert_no_target_intent(&audit_pool, object_key).await;
        }

        let tagged_copy_source = put_object(
            &client,
            address,
            &owner,
            target_bucket,
            "tagged-copy-source.txt",
            b"copy must authorize inherited tags",
            &[("x-amz-tagging", "color=blue")],
        )
        .await;
        assert_eq!(tagged_copy_source.status(), StatusCode::OK);

        let same_source = put_object(
            &client,
            address,
            &owner,
            target_bucket,
            "same-source.txt",
            b"same-bucket-source",
            &[],
        )
        .await;
        assert_eq!(same_source.status(), StatusCode::OK);
        let version_one = put_object(
            &client,
            address,
            &external,
            external_bucket,
            "versioned-source.txt",
            b"version-one",
            &[],
        )
        .await;
        assert_eq!(version_one.status(), StatusCode::OK);
        let version_one_id = version_one
            .headers()
            .get("x-amz-version-id")
            .expect("first source version header")
            .to_str()
            .expect("first source version")
            .to_owned();
        let version_two = put_object(
            &client,
            address,
            &external,
            external_bucket,
            "versioned-source.txt",
            b"version-two",
            &[],
        )
        .await;
        assert_eq!(version_two.status(), StatusCode::OK);

        let owner_identity = state
            .repository
            .get_s3_identity_policy(owner.access_key_id)
            .await
            .expect("owner identity lookup")
            .expect("owner identity snapshot");
        let owner_arn = format!(
            "arn:aws:iam::{}:user/{}",
            owner_identity.identity.account_id.as_str(),
            owner.access_key_id
        );
        let external_identity = state
            .repository
            .get_s3_identity_policy(external.access_key_id)
            .await
            .expect("external identity lookup")
            .expect("external identity snapshot");
        let external_arn = format!(
            "arn:aws:iam::{}:user/{}",
            external_identity.identity.account_id.as_str(),
            external.access_key_id
        );
        install_bucket_policy(
            &state,
            external.application_id,
            external_bucket,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": {
                    "Effect": "Allow",
                    "Principal": {"AWS": owner_arn},
                    "Action": ["s3:GetObject", "s3:GetObjectVersion"],
                    "Resource": format!("arn:aws:s3:::{external_bucket}/*")
                }
            }),
        )
        .await;

        let same_copy = copy_object(
            &client,
            address,
            &owner,
            target_bucket,
            "same-copy.txt",
            &format!("/{target_bucket}/same-source.txt"),
            &[],
        )
        .await;
        assert_eq!(same_copy.status(), StatusCode::OK);
        let empty_replace_copy = copy_object(
            &client,
            address,
            &owner,
            target_bucket,
            "empty-replace-copy.txt",
            &format!("/{target_bucket}/same-source.txt"),
            &[
                ("x-amz-tagging-directive", "REPLACE"),
                ("x-amz-tagging", ""),
            ],
        )
        .await;
        assert_eq!(empty_replace_copy.status(), StatusCode::OK);
        assert_s3_error(
            copy_object(
                &client,
                address,
                &owner,
                target_bucket,
                "copy-inherited-tags-without-action.txt",
                &format!("/{target_bucket}/tagged-copy-source.txt"),
                &[],
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_no_target_intent(&audit_pool, "copy-inherited-tags-without-action.txt").await;
        let cross_copy = copy_object(
            &client,
            address,
            &owner,
            target_bucket,
            "cross-copy.txt",
            &format!("/{external_bucket}/versioned-source.txt"),
            &[],
        )
        .await;
        assert_eq!(cross_copy.status(), StatusCode::OK);
        let version_copy = copy_object(
            &client,
            address,
            &owner,
            target_bucket,
            "version-copy.txt",
            &format!(
                "/{external_bucket}/versioned-source.txt?versionId={version_one_id}"
            ),
            &[],
        )
        .await;
        assert_eq!(version_copy.status(), StatusCode::OK);
        assert_eq!(
            version_copy
                .headers()
                .get("x-amz-copy-source-version-id")
                .expect("copy source version header"),
            version_one_id.as_str()
        );
        let copied_version = get_object(
            &client,
            address,
            &owner,
            target_bucket,
            "version-copy.txt",
        )
        .await;
        assert_eq!(copied_version.status(), StatusCode::OK);
        assert_eq!(
            copied_version.bytes().await.expect("copied version body"),
            "version-one"
        );

        assert_s3_error(
            copy_object(
                &client,
                address,
                &owner,
                target_bucket,
                "copy-tagging-without-action.txt",
                &format!("/{target_bucket}/same-source.txt"),
                &[
                    ("x-amz-tagging-directive", "REPLACE"),
                    ("x-amz-tagging", "color=blue"),
                ],
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_no_target_intent(&audit_pool, "copy-tagging-without-action.txt").await;
        assert_s3_error(
            copy_object(
                &client,
                address,
                &owner,
                target_bucket,
                "copy-acl-without-action.txt",
                &format!("/{target_bucket}/same-source.txt"),
                &[("x-amz-acl", "private")],
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_no_target_intent(&audit_pool, "copy-acl-without-action.txt").await;

        install_bucket_policy(
            &state,
            external.application_id,
            external_bucket,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": [
                    {
                        "Effect": "Allow",
                        "Principal": {"AWS": owner_arn},
                        "Action": ["s3:GetObject", "s3:GetObjectVersion"],
                        "Resource": format!("arn:aws:s3:::{external_bucket}/*")
                    },
                    {
                        "Effect": "Deny",
                        "Principal": {"AWS": owner_arn},
                        "Action": "s3:GetObject",
                        "Resource": format!(
                            "arn:aws:s3:::{external_bucket}/missing-denied-source.txt"
                        )
                    }
                ]
            }),
        )
        .await;
        assert_s3_error(
            copy_object(
                &client,
                address,
                &owner,
                target_bucket,
                "source-denied-target.txt",
                &format!("/{external_bucket}/missing-denied-source.txt"),
                &[],
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_no_target_intent(&audit_pool, "source-denied-target.txt").await;

        install_bucket_policy(
            &state,
            owner.application_id,
            target_bucket,
            serde_json::json!({
                "Version": "2012-10-17",
                "Statement": [
                    {
                        "Effect": "Allow",
                        "Principal": {"AWS": external_arn},
                        "Action": "s3:PutObject",
                        "Resource": format!("arn:aws:s3:::{target_bucket}/cross-owner-*")
                    },
                    {
                        "Effect": "Deny",
                        "Principal": {"AWS": owner_arn},
                        "Action": "s3:PutObject",
                        "Resource": format!("arn:aws:s3:::{target_bucket}/target-denied.txt")
                    }
                ]
            }),
        )
        .await;
        assert_s3_error(
            copy_object(
                &client,
                address,
                &owner,
                target_bucket,
                "target-denied.txt",
                &format!("/{external_bucket}/source-does-not-exist.txt"),
                &[],
            )
            .await,
            StatusCode::FORBIDDEN,
            "AccessDenied",
        )
        .await;
        assert_no_target_intent(&audit_pool, "target-denied.txt").await;

        let cross_owner_put = put_object(
            &client,
            address,
            &external,
            target_bucket,
            "cross-owner-put.txt",
            b"cross-account target",
            &[],
        )
        .await;
        assert_eq!(cross_owner_put.status(), StatusCode::OK);
        assert_resource_audit(
            &audit_pool,
            "s3.object.uploaded",
            "cross-owner-put.txt",
            owner.application_id,
            external.access_key_id,
        )
        .await;
        let cross_owner_copy = copy_object(
            &client,
            address,
            &external,
            target_bucket,
            "cross-owner-copy.txt",
            &format!("/{external_bucket}/versioned-source.txt"),
            &[],
        )
        .await;
        assert_eq!(cross_owner_copy.status(), StatusCode::OK);
        assert_resource_audit(
            &audit_pool,
            "s3.object.copied",
            "cross-owner-copy.txt",
            owner.application_id,
            external.access_key_id,
        )
        .await;

        state
            .repository
            .delete_s3_identity_policy(&DeleteS3IdentityPolicy {
                application_id: owner.application_id,
                access_key_id: owner.access_key_id.to_owned(),
                updated_at: OffsetDateTime::now_utc(),
            })
            .await
            .expect("delete owner identity policy")
            .expect("owner identity snapshot");
        for object_key in ["no-identity-put.txt", "no-identity-copy.txt"] {
            let response = if object_key == "no-identity-put.txt" {
                put_object(
                    &client,
                    address,
                    &owner,
                    target_bucket,
                    object_key,
                    b"legacy upload grant must not fallback",
                    &[],
                )
                .await
            } else {
                copy_object(
                    &client,
                    address,
                    &owner,
                    target_bucket,
                    object_key,
                    &format!("/{target_bucket}/same-source.txt"),
                    &[],
                )
                .await
            };
            assert_s3_error(response, StatusCode::FORBIDDEN, "AccessDenied").await;
            assert_no_target_intent(&audit_pool, object_key).await;
        }

        server.abort();
        let _ = server.await;
        drop(client);
        drop(state);
        std::fs::remove_dir_all(storage_root).expect("remove Put/Copy policy test storage");
    }
}
