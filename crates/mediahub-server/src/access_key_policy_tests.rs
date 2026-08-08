fn access_key_policy_put_request(headers: &HeaderMap, body: impl Into<Body>) -> Request {
    let mut request = Request::builder()
        .method(Method::PUT)
        .uri("/api/v1/access-keys/test/s3-policy")
        .body(body.into())
        .expect("policy request");
    *request.headers_mut() = headers.clone();
    request
        .headers_mut()
        .insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    request
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn access_key_s3_policy_control_plane_is_tenant_fenced_strict_and_audited(
    pool: sqlx::PgPool,
) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (owner_user_id, owner_headers) =
        authenticated_test_user(&state, "identity-policy-owner@example.com", "user").await;
    let owner_application = state
        .repository
        .default_application_for_user(owner_user_id)
        .await
        .expect("owner application lookup")
        .expect("owner application");
    let (_, other_headers) =
        authenticated_test_user(&state, "identity-policy-other@example.com", "user").await;
    let access_key_id = "mh_ak_control_policy";
    create_s3_policy_test_access_key(
        &state,
        owner_application.id,
        access_key_id,
        "identity-policy-secret",
    )
    .await;

    let missing = get_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        owner_headers.clone(),
    )
    .await
    .expect_err("legacy permissions must not synthesize a policy");
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.code, "not_found");

    let unauthenticated = put_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        Extension(RequestId("req-policy-unauthenticated".into())),
        access_key_policy_put_request(&HeaderMap::new(), "{"),
    )
    .await
    .expect_err("authentication must run before policy parsing");
    assert_eq!(unauthenticated.status, StatusCode::UNAUTHORIZED);

    let mut missing_csrf_headers = owner_headers.clone();
    missing_csrf_headers.remove("x-csrf-token");
    let missing_csrf = put_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        Extension(RequestId("req-policy-csrf".into())),
        access_key_policy_put_request(&missing_csrf_headers, "{"),
    )
    .await
    .expect_err("CSRF must run before policy parsing");
    assert_eq!(missing_csrf.status, StatusCode::FORBIDDEN);

    let cross_tenant_get = get_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        other_headers.clone(),
    )
    .await
    .expect_err("other tenant must not inspect policy state");
    assert_eq!(cross_tenant_get.status, StatusCode::NOT_FOUND);
    let cross_tenant_put = put_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        Extension(RequestId("req-policy-other-put".into())),
        access_key_policy_put_request(&other_headers, "{"),
    )
    .await
    .expect_err("ownership must run before policy parsing");
    assert_eq!(cross_tenant_put.status, StatusCode::NOT_FOUND);
    let cross_tenant_delete = delete_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        other_headers.clone(),
        Extension(RequestId("req-policy-other-delete".into())),
    )
    .await
    .expect_err("other tenant must not delete policy state");
    assert_eq!(cross_tenant_delete.status, StatusCode::NOT_FOUND);

    let oversized = put_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        Extension(RequestId("req-policy-oversized".into())),
        access_key_policy_put_request(&owner_headers, vec![b' '; MAX_S3_POLICY_BYTES + 1]),
    )
    .await
    .expect_err("oversized policy");
    assert_eq!(oversized.status, StatusCode::PAYLOAD_TOO_LARGE);

    let deny_all = r#"{
        "Statement":{"Resource":"*","Action":"s3:*","Effect":"Deny","Sid":"DenyAll"},
        "Version":"2012-10-17"
    }"#;
    let put_response = put_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        Extension(RequestId("req-policy-put".into())),
        access_key_policy_put_request(&owner_headers, deny_all),
    )
    .await
    .expect("put identity policy");
    assert_eq!(put_response.status(), StatusCode::OK);
    assert_eq!(put_response.headers()[CONTENT_TYPE], "application/json");
    let put_body = String::from_utf8(
        to_bytes(put_response.into_body(), MAX_S3_POLICY_BYTES)
            .await
            .expect("policy body")
            .to_vec(),
    )
    .expect("UTF-8 policy");
    S3IdentityPolicy::parse(put_body.as_bytes()).expect("canonical policy response");
    assert!(put_body.contains("\"Statement\":[{"));
    assert!(!put_body.contains("identity-policy-secret"));

    let get_response = get_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        owner_headers.clone(),
    )
    .await
    .expect("get identity policy");
    let get_body = String::from_utf8(
        to_bytes(get_response.into_body(), MAX_S3_POLICY_BYTES)
            .await
            .expect("policy body")
            .to_vec(),
    )
    .expect("UTF-8 policy");
    assert_eq!(get_body, put_body);

    for request_id in ["req-policy-delete", "req-policy-delete-repeat"] {
        let status = delete_access_key_s3_policy(
            State(Arc::clone(&state)),
            Path(access_key_id.to_owned()),
            owner_headers.clone(),
            Extension(RequestId(request_id.into())),
        )
        .await
        .expect("idempotent delete");
        assert_eq!(status, StatusCode::NO_CONTENT);
    }
    let deleted = get_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        owner_headers.clone(),
    )
    .await
    .expect_err("deleted policy must be explicit 404");
    assert_eq!(deleted.status, StatusCode::NOT_FOUND);

    state
        .repository
        .revoke_access_key(
            access_key_id,
            owner_application.id,
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("revoke access key");
    let revoked_put = put_access_key_s3_policy(
        State(Arc::clone(&state)),
        Path(access_key_id.to_owned()),
        Extension(RequestId("req-policy-revoked".into())),
        access_key_policy_put_request(&owner_headers, deny_all),
    )
    .await
    .expect_err("revoked key policy must not be updated");
    assert_eq!(revoked_put.status, StatusCode::CONFLICT);

    let audit = state
        .repository
        .list_audit(owner_application.id, 20)
        .await
        .expect("policy audit");
    let updated = audit
        .iter()
        .find(|event| event.action == "access_key.s3_policy.updated")
        .expect("policy update audit");
    assert_eq!(updated.target_id, access_key_id);
    assert_eq!(updated.summary["revision"], 1);
    assert_eq!(
        updated.summary["sha256"]
            .as_str()
            .expect("policy digest")
            .len(),
        64
    );
    assert!(!updated.summary.to_string().contains("DenyAll"));
    assert!(!updated.summary.to_string().contains("secret"));
    assert!(audit.iter().any(|event| {
        event.action == "access_key.s3_policy.deleted" && event.summary["sha256"].is_null()
    }));

    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn access_key_s3_policy_http_round_trip_enforces_security_and_limits(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (owner_user_id, owner_headers) =
        authenticated_test_user(&state, "identity-policy-http-owner@example.com", "user").await;
    let owner_application = state
        .repository
        .default_application_for_user(owner_user_id)
        .await
        .expect("owner application lookup")
        .expect("owner application");
    let (_, other_headers) =
        authenticated_test_user(&state, "identity-policy-http-other@example.com", "user").await;
    let access_key_id = "mh_ak_http_control_policy";
    create_s3_policy_test_access_key(
        &state,
        owner_application.id,
        access_key_id,
        "identity-policy-http-secret",
    )
    .await;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind control-plane test server");
    let address = listener.local_addr().expect("control-plane address");
    let server_state = (*state).clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(server_state, None))
            .await
            .expect("control-plane test server");
    });
    let client = reqwest::Client::new();
    let url = format!("http://{address}/api/v1/access-keys/{access_key_id}/s3-policy");
    let deny_all = r#"{"Version":"2012-10-17","Statement":{"Sid":"DenyAll","Effect":"Deny","Action":"s3:*","Resource":"*"}}"#;

    let missing = client
        .get(&url)
        .headers(owner_headers.clone())
        .send()
        .await
        .expect("missing policy request");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let mut no_csrf = owner_headers.clone();
    no_csrf.remove("x-csrf-token");
    let csrf = client
        .put(&url)
        .headers(no_csrf)
        .header(CONTENT_TYPE, "application/json")
        .body("{")
        .send()
        .await
        .expect("CSRF request");
    assert_eq!(csrf.status(), StatusCode::FORBIDDEN);

    for response in [
        client.get(&url).headers(other_headers.clone()).send().await,
        client
            .put(&url)
            .headers(other_headers.clone())
            .header(CONTENT_TYPE, "application/json")
            .body("{")
            .send()
            .await,
        client
            .delete(&url)
            .headers(other_headers.clone())
            .send()
            .await,
    ] {
        assert_eq!(
            response.expect("cross-tenant policy request").status(),
            StatusCode::NOT_FOUND
        );
    }

    let oversized = client
        .put(&url)
        .headers(owner_headers.clone())
        .header(CONTENT_TYPE, "application/json")
        .body(vec![b' '; MAX_S3_POLICY_BYTES + 1])
        .send()
        .await
        .expect("oversized policy request");
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let put = client
        .put(&url)
        .headers(owner_headers.clone())
        .header(CONTENT_TYPE, "application/json")
        .body(deny_all)
        .send()
        .await
        .expect("put policy request");
    assert_eq!(put.status(), StatusCode::OK);
    assert_eq!(put.headers()[CONTENT_TYPE], "application/json");
    let canonical = put.text().await.expect("canonical policy body");
    assert!(canonical.contains("\"Statement\":[{"));

    let get = client
        .get(&url)
        .headers(owner_headers.clone())
        .send()
        .await
        .expect("get policy request");
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(get.text().await.expect("get policy body"), canonical);

    for _ in 0..2 {
        let delete = client
            .delete(&url)
            .headers(owner_headers.clone())
            .send()
            .await
            .expect("delete policy request");
        assert_eq!(delete.status(), StatusCode::NO_CONTENT);
    }

    state
        .repository
        .revoke_access_key(
            access_key_id,
            owner_application.id,
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("revoke access key");
    let revoked = client
        .put(&url)
        .headers(owner_headers.clone())
        .header(CONTENT_TYPE, "application/json")
        .body(deny_all)
        .send()
        .await
        .expect("revoked policy request");
    assert_eq!(revoked.status(), StatusCode::CONFLICT);

    let audit = state
        .repository
        .list_audit(owner_application.id, 20)
        .await
        .expect("policy audit");
    assert!(audit.iter().any(|event| {
        event.action == "access_key.s3_policy.updated"
            && event.summary["sha256"]
                .as_str()
                .is_some_and(|value| value.len() == 64)
            && !event.summary.to_string().contains("DenyAll")
    }));
    assert!(audit.iter().any(|event| {
        event.action == "access_key.s3_policy.deleted" && event.summary["sha256"].is_null()
    }));

    server.abort();
    let _ = server.await;
    drop(client);
    drop(state);
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}
