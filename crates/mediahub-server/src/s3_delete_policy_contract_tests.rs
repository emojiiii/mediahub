async fn create_delete_policy_access_key(
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
                .expect("encrypt delete test key"),
            secret_key_version: state.access_key_cipher.version(),
            secret_last_four: secret
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect(),
            name: "S3 Delete Policy".to_owned(),
            permissions: vec![
                "bucket:manage".to_owned(),
                "bucket:list".to_owned(),
                "media:upload".to_owned(),
                "media:delete".to_owned(),
            ],
            expires_at: None,
            created_at: OffsetDateTime::now_utc(),
        })
        .await
        .expect("create delete test access key");
}

async fn put_delete_identity_policy(
    state: &AppState,
    application_id: ApplicationId,
    access_key_id: &str,
    statements: serde_json::Value,
) {
    let document = serde_json::json!({
        "Version": "2012-10-17",
        "Statement": statements,
    });
    mediahub_app::S3IdentityPolicyRepository::put_s3_identity_policy(
        &state.repository,
        &mediahub_app::PutS3IdentityPolicy {
            application_id,
            access_key_id: access_key_id.to_owned(),
            policy: mediahub_app::S3IdentityPolicyDocument::parse(
                &serde_json::to_vec(&document).expect("serialize delete identity policy"),
            )
            .expect("parse delete identity policy"),
            updated_at: OffsetDateTime::now_utc(),
        },
    )
    .await
    .expect("persist delete identity policy")
    .expect("delete identity access key");
}

async fn put_delete_test_object(
    client: &reqwest::Client,
    address: std::net::SocketAddr,
    bucket_url: &str,
    object_key: &str,
    access_key_id: &str,
    secret: &str,
    governance: bool,
) -> String {
    let object_url = format!("{bucket_url}/{object_key}");
    let mut request = http::Request::builder()
        .method(Method::PUT)
        .uri(&object_url)
        .header("host", address.to_string())
        .header(CONTENT_TYPE, "text/plain");
    if governance {
        let retain_until = (OffsetDateTime::now_utc() + time::Duration::days(30))
            .replace_nanosecond(0)
            .expect("whole-second governance retention")
            .format(&time::format_description::well_known::Rfc3339)
            .expect("governance retention timestamp");
        request = request
            .header("x-amz-object-lock-mode", "GOVERNANCE")
            .header("x-amz-object-lock-retain-until-date", retain_until);
    }
    let mut request = request
        .body(format!("delete-policy:{object_key}").into_bytes())
        .expect("PutObject request");
    sign_s3_test_request(&mut request, access_key_id, secret, None);
    let response = send_s3_test_request(client, request).await;
    assert_eq!(response.status(), StatusCode::OK, "PUT {object_key}");
    response
        .headers()
        .get("x-amz-version-id")
        .expect("versioned PutObject response")
        .to_str()
        .expect("version ID header")
        .to_owned()
}

async fn send_delete_policy_request(
    client: &reqwest::Client,
    address: std::net::SocketAddr,
    object_url: &str,
    version_id: Option<&str>,
    bypass_governance: bool,
    access_key_id: &str,
    secret: &str,
) -> reqwest::Response {
    let uri = version_id.map_or_else(
        || object_url.to_owned(),
        |version_id| format!("{object_url}?versionId={version_id}"),
    );
    let mut request = http::Request::builder()
        .method(Method::DELETE)
        .uri(uri)
        .header("host", address.to_string());
    if bypass_governance {
        request = request.header("x-amz-bypass-governance-retention", "true");
    }
    let mut request = request.body(Vec::new()).expect("DeleteObject request");
    sign_s3_test_request(&mut request, access_key_id, secret, None);
    send_s3_test_request(client, request).await
}

async fn assert_delete_policy_error(
    response: reqwest::Response,
    status: StatusCode,
    code: &str,
) {
    assert_eq!(response.status(), status);
    let body = response.text().await.expect("S3 error XML");
    assert!(body.contains(&format!("<Code>{code}</Code>")), "{body}");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn s3_delete_policy_http_contract_enforces_actions_batch_bypass_and_target_tenant(
    pool: sqlx::PgPool,
) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (owner_user_id, _) =
        authenticated_test_user(&state, "s3-delete-policy-owner@example.com", "user").await;
    let owner_application = state
        .repository
        .default_application_for_user(owner_user_id)
        .await
        .expect("owner application lookup")
        .expect("owner application");
    let owner_key = "mh_ak_delete_policy_owner";
    let owner_secret = "delete-policy-owner-secret";
    create_delete_policy_access_key(&state, owner_application.id, owner_key, owner_secret).await;

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn({
        let application = s3_router::router(Arc::clone(&state));
        async move {
            axum::serve(
                listener,
                application.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .await
            .expect("S3 delete Policy test server");
        }
    });
    let client = reqwest::Client::new();
    let bucket_name = "delete-policy-assets";
    let bucket_url = format!("http://{address}/{bucket_name}");
    let mut create_bucket = http::Request::builder()
        .method(Method::PUT)
        .uri(&bucket_url)
        .header("host", address.to_string())
        .header("x-amz-bucket-object-lock-enabled", "true")
        .body(Vec::new())
        .expect("CreateBucket request");
    sign_s3_test_request(&mut create_bucket, owner_key, owner_secret, None);
    assert_eq!(
        send_s3_test_request(&client, create_bucket).await.status(),
        StatusCode::OK
    );

    put_delete_identity_policy(
        &state,
        owner_application.id,
        owner_key,
        serde_json::json!({
            "Effect": "Allow",
            "Action": ["s3:PutObject", "s3:PutObjectRetention"],
            "Resource": format!("arn:aws:s3:::{bucket_name}/*")
        }),
    )
    .await;

    let mut versions = std::collections::HashMap::new();
    for key in [
        "legacy-only.txt",
        "plain-allow.txt",
        "plain-deny.txt",
        "version-allow.txt",
        "version-deny.txt",
        "batch-allow.txt",
        "batch-deny.txt",
        "cross-account.txt",
    ] {
        versions.insert(
            key,
            put_delete_test_object(
                &client,
                address,
                &bucket_url,
                key,
                owner_key,
                owner_secret,
                false,
            )
            .await,
        );
    }
    let governance_version = put_delete_test_object(
        &client,
        address,
        &bucket_url,
        "governance.txt",
        owner_key,
        owner_secret,
        true,
    )
    .await;

    mediahub_app::S3IdentityPolicyRepository::delete_s3_identity_policy(
        &state.repository,
        &mediahub_app::DeleteS3IdentityPolicy {
            application_id: owner_application.id,
            access_key_id: owner_key.to_owned(),
            updated_at: OffsetDateTime::now_utc(),
        },
    )
    .await
    .expect("delete seed identity policy")
    .expect("owner identity after seed policy deletion");

    let legacy_only_url = format!("{bucket_url}/legacy-only.txt");
    assert_delete_policy_error(
        send_delete_policy_request(
            &client,
            address,
            &legacy_only_url,
            None,
            false,
            owner_key,
            owner_secret,
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;
    let mut bad_signature = http::Request::builder()
        .method(Method::DELETE)
        .uri(&legacy_only_url)
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("bad signature delete request");
    sign_s3_test_request(&mut bad_signature, owner_key, "wrong-secret", None);
    assert_delete_policy_error(
        send_s3_test_request(&client, bad_signature).await,
        StatusCode::FORBIDDEN,
        "SignatureDoesNotMatch",
    )
    .await;

    let object_arn = |key: &str| format!("arn:aws:s3:::{bucket_name}/{key}");
    put_delete_identity_policy(
        &state,
        owner_application.id,
        owner_key,
        serde_json::json!([
            {
                "Effect": "Allow",
                "Action": "s3:DeleteObject",
                "Resource": [object_arn("plain-allow.txt"), object_arn("batch-allow.txt")]
            },
            {
                "Effect": "Deny",
                "Action": "s3:DeleteObject",
                "Resource": [object_arn("plain-deny.txt"), object_arn("batch-deny.txt")]
            },
            {
                "Effect": "Allow",
                "Action": "s3:DeleteObjectVersion",
                "Resource": object_arn("version-allow.txt")
            },
            {
                "Effect": "Deny",
                "Action": "s3:DeleteObjectVersion",
                "Resource": object_arn("version-deny.txt")
            }
        ]),
    )
    .await;

    assert_eq!(
        send_delete_policy_request(
            &client,
            address,
            &format!("{bucket_url}/plain-allow.txt"),
            None,
            false,
            owner_key,
            owner_secret,
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_delete_policy_error(
        send_delete_policy_request(
            &client,
            address,
            &format!("{bucket_url}/plain-deny.txt"),
            None,
            false,
            owner_key,
            owner_secret,
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;
    assert_eq!(
        send_delete_policy_request(
            &client,
            address,
            &format!("{bucket_url}/version-allow.txt"),
            Some(&versions["version-allow.txt"]),
            false,
            owner_key,
            owner_secret,
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    assert_delete_policy_error(
        send_delete_policy_request(
            &client,
            address,
            &format!("{bucket_url}/version-deny.txt"),
            Some(&versions["version-deny.txt"]),
            false,
            owner_key,
            owner_secret,
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;

    let batch_body = br#"<Delete><Object><Key>batch-allow.txt</Key></Object><Object><Key>batch-deny.txt</Key></Object></Delete>"#.to_vec();
    let mut batch = http::Request::builder()
        .method(Method::POST)
        .uri(format!("{bucket_url}?delete"))
        .header("host", address.to_string())
        .header(CONTENT_TYPE, "application/xml")
        .header("content-md5", s3_test_content_md5(&batch_body))
        .body(batch_body)
        .expect("DeleteObjects request");
    sign_s3_test_request(&mut batch, owner_key, owner_secret, None);
    let batch = send_s3_test_request(&client, batch).await;
    assert_eq!(batch.status(), StatusCode::OK);
    let batch_xml = batch.text().await.expect("DeleteResult XML");
    assert!(batch_xml.contains("<Deleted><Key>batch-allow.txt</Key>"));
    assert!(batch_xml.contains("<Error><Key>batch-deny.txt</Key>"));
    assert!(batch_xml.contains("<Code>AccessDenied</Code>"));

    let governance_url = format!("{bucket_url}/governance.txt");
    put_delete_identity_policy(
        &state,
        owner_application.id,
        owner_key,
        serde_json::json!({
            "Effect": "Allow",
            "Action": "s3:BypassGovernanceRetention",
            "Resource": object_arn("governance.txt")
        }),
    )
    .await;
    assert_delete_policy_error(
        send_delete_policy_request(
            &client,
            address,
            &governance_url,
            Some(&governance_version),
            true,
            owner_key,
            owner_secret,
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;
    put_delete_identity_policy(
        &state,
        owner_application.id,
        owner_key,
        serde_json::json!({
            "Effect": "Allow",
            "Action": "s3:DeleteObjectVersion",
            "Resource": object_arn("governance.txt")
        }),
    )
    .await;
    assert_delete_policy_error(
        send_delete_policy_request(
            &client,
            address,
            &governance_url,
            Some(&governance_version),
            true,
            owner_key,
            owner_secret,
        )
        .await,
        StatusCode::FORBIDDEN,
        "AccessDenied",
    )
    .await;
    put_delete_identity_policy(
        &state,
        owner_application.id,
        owner_key,
        serde_json::json!({
            "Effect": "Allow",
            "Action": ["s3:DeleteObjectVersion", "s3:BypassGovernanceRetention"],
            "Resource": object_arn("governance.txt")
        }),
    )
    .await;
    assert_eq!(
        send_delete_policy_request(
            &client,
            address,
            &governance_url,
            Some(&governance_version),
            true,
            owner_key,
            owner_secret,
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );

    let (caller_user_id, _) =
        authenticated_test_user(&state, "s3-delete-policy-caller@example.com", "user").await;
    let caller_application = state
        .repository
        .default_application_for_user(caller_user_id)
        .await
        .expect("caller application lookup")
        .expect("caller application");
    let caller_key = "mh_ak_delete_policy_caller";
    let caller_secret = "delete-policy-caller-secret";
    create_delete_policy_access_key(&state, caller_application.id, caller_key, caller_secret).await;
    put_delete_identity_policy(
        &state,
        caller_application.id,
        caller_key,
        serde_json::json!({
            "Effect": "Allow",
            "Action": "s3:DeleteObject",
            "Resource": object_arn("cross-account.txt")
        }),
    )
    .await;
    let caller_identity = mediahub_app::S3IdentityPolicyRepository::get_s3_identity_policy(
        &state.repository,
        caller_key,
    )
    .await
    .expect("caller identity lookup")
    .expect("caller identity");
    let caller_arn = format!(
        "arn:aws:iam::{}:user/{caller_key}",
        caller_identity.identity.account_id.as_str()
    );
    mediahub_app::S3BucketPolicyRepository::put_s3_bucket_policy(
        &state.repository,
        owner_application.id,
        bucket_name,
        mediahub_app::S3BucketPolicyDocument::new(serde_json::json!({
            "Version": "2012-10-17",
            "Statement": {
                "Effect": "Allow",
                "Principal": {"AWS": caller_arn},
                "Action": "s3:DeleteObject",
                "Resource": object_arn("cross-account.txt")
            }
        }))
        .expect("cross-account bucket policy"),
        OffsetDateTime::now_utc(),
    )
    .await
    .expect("persist cross-account bucket policy")
    .expect("owner bucket identity");
    assert_eq!(
        send_delete_policy_request(
            &client,
            address,
            &format!("{bucket_url}/cross-account.txt"),
            None,
            false,
            caller_key,
            caller_secret,
        )
        .await
        .status(),
        StatusCode::NO_CONTENT
    );
    let owner_audit = state
        .repository
        .list_audit(owner_application.id, 100)
        .await
        .expect("owner audit list");
    assert!(owner_audit.iter().any(|event| {
        event.action == "s3.object.deleted"
            && event.actor_id == caller_key
            && event.summary["object_key"] == "cross-account.txt"
    }));
    let caller_audit = state
        .repository
        .list_audit(caller_application.id, 100)
        .await
        .expect("caller audit list");
    assert!(!caller_audit.iter().any(|event| {
        event.action == "s3.object.deleted" && event.summary["object_key"] == "cross-account.txt"
    }));

    server.abort();
    let _ = server.await;
    drop(client);
    drop(state);
    std::fs::remove_dir_all(storage_root).expect("remove S3 delete Policy object storage");
}
