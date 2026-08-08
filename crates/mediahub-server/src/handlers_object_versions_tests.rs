#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn immutable_object_version_preview_is_tenant_scoped_and_http_correct(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (owner_id, owner_headers) =
        authenticated_test_user(&state, "preview-owner@example.com", "user").await;
    let owner_application = state
        .repository
        .default_application_for_user(owner_id)
        .await
        .expect("owner application lookup")
        .expect("owner application");
    let owner_bucket = preview_test_bucket(&state, owner_application.id, "preview-files").await;
    let content = b"alpha,beta\n1,2\n";
    let (version_id, etag) = preview_test_data_version(
        &state,
        owner_application.id,
        owner_bucket.id(),
        "reports/annual.csv",
        "text/csv; charset=utf-8",
        content,
    )
    .await;
    let delete_marker_id = preview_test_delete_marker(
        &state,
        owner_application.id,
        owner_bucket.id(),
        "reports/deleted.csv",
    )
    .await;

    let (other_id, _) = authenticated_test_user(&state, "preview-other@example.com", "user").await;
    let other_application = state
        .repository
        .default_application_for_user(other_id)
        .await
        .expect("other application lookup")
        .expect("other application");
    let other_bucket = preview_test_bucket(&state, other_application.id, "other-preview").await;
    let (other_version_id, _) = preview_test_data_version(
        &state,
        other_application.id,
        other_bucket.id(),
        "private/other.txt",
        "text/plain",
        b"other-tenant",
    )
    .await;

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn({
        let application = router((*state).clone(), None);
        async move {
            axum::serve(listener, application)
                .await
                .expect("preview test server");
        }
    });
    let client = reqwest::Client::new();
    let manifest_url =
        format!("http://{address}/api/v1/object-versions/{version_id}/preview-manifest");
    let content_url = format!("http://{address}/api/v1/object-versions/{version_id}/content");

    let anonymous = client
        .get(&manifest_url)
        .send()
        .await
        .expect("anonymous manifest");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    let manifest = client
        .get(&manifest_url)
        .headers(owner_headers.clone())
        .send()
        .await
        .expect("preview manifest");
    assert_eq!(manifest.status(), StatusCode::OK);
    let manifest: serde_json::Value = manifest.json().await.expect("manifest JSON");
    assert_eq!(manifest["version_id"], version_id.to_string());
    assert_eq!(manifest["etag"], etag);
    assert_eq!(manifest["content_type"], "text/csv; charset=utf-8");
    assert_eq!(manifest["size"], content.len() as u64);
    assert_eq!(manifest["renderer"], "spreadsheet");
    assert_eq!(manifest["renderer_version"], "1");
    assert_eq!(manifest["mode"], "buffered");
    assert_eq!(
        manifest["max_bytes"],
        OBJECT_VERSION_BUFFERED_PREVIEW_MAX_BYTES
    );
    assert_eq!(
        manifest["content_url"],
        format!("/api/v1/object-versions/{version_id}/content")
    );
    assert_eq!(manifest["warnings"], serde_json::json!([]));

    let full = client
        .get(&content_url)
        .headers(owner_headers.clone())
        .send()
        .await
        .expect("full content");
    assert_eq!(full.status(), StatusCode::OK);
    assert_eq!(full.headers()[CONTENT_TYPE], "text/csv; charset=utf-8");
    assert_eq!(full.headers()[ACCEPT_RANGES], "bytes");
    assert_eq!(full.headers()[ETAG], format!("\"{etag}\""));
    assert_eq!(
        full.headers()[axum::http::header::CACHE_CONTROL],
        "private, max-age=31536000, immutable"
    );
    assert_eq!(
        full.bytes().await.expect("full content body").as_ref(),
        content
    );

    let head = client
        .head(&content_url)
        .headers(owner_headers.clone())
        .send()
        .await
        .expect("HEAD content");
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()[CONTENT_LENGTH], content.len().to_string());
    assert!(head.bytes().await.expect("HEAD body").is_empty());

    let not_modified = client
        .get(&content_url)
        .headers(owner_headers.clone())
        .header(IF_NONE_MATCH, format!("W/\"{etag}\""))
        .send()
        .await
        .expect("conditional content");
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
    assert!(
        not_modified
            .bytes()
            .await
            .expect("not-modified body")
            .is_empty()
    );

    let range = client
        .get(&content_url)
        .headers(owner_headers.clone())
        .header(RANGE, "bytes=2-6")
        .send()
        .await
        .expect("range content");
    assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        range.headers()[CONTENT_RANGE],
        format!("bytes 2-6/{}", content.len())
    );
    assert_eq!(range.headers()[CONTENT_LENGTH], "5");
    assert_eq!(range.bytes().await.expect("range body").as_ref(), b"pha,b");

    let suffix = client
        .get(&content_url)
        .headers(owner_headers.clone())
        .header(RANGE, "bytes=-4")
        .send()
        .await
        .expect("suffix range content");
    assert_eq!(suffix.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        suffix.bytes().await.expect("suffix body").as_ref(),
        b"1,2\n"
    );

    let invalid_range = client
        .get(&content_url)
        .headers(owner_headers.clone())
        .header(RANGE, "bytes=999-")
        .send()
        .await
        .expect("invalid range content");
    assert_eq!(invalid_range.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(
        invalid_range.headers()[CONTENT_RANGE],
        format!("bytes */{}", content.len())
    );

    for hidden_version_id in [delete_marker_id, other_version_id] {
        for suffix in ["preview-manifest", "content"] {
            let response = client
                .get(format!(
                    "http://{address}/api/v1/object-versions/{hidden_version_id}/{suffix}"
                ))
                .headers(owner_headers.clone())
                .send()
                .await
                .expect("hidden object version request");
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            assert_eq!(
                response
                    .json::<serde_json::Value>()
                    .await
                    .expect("hidden response JSON")["error"]["message"],
                "object version not found"
            );
        }
    }
    for missing in [
        mediahub_core::ObjectVersionId::new().to_string(),
        "not-a-version-id".to_owned(),
    ] {
        let response = client
            .get(format!(
                "http://{address}/api/v1/object-versions/{missing}/preview-manifest"
            ))
            .headers(owner_headers.clone())
            .send()
            .await
            .expect("missing object version request");
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    let legacy_media_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM media WHERE application_id = $1")
            .bind(owner_application.id.as_uuid())
            .fetch_one(state.repository.pool())
            .await
            .expect("count legacy Media rows");
    assert_eq!(legacy_media_count, 0);

    server.abort();
    drop(state);
    std::fs::remove_dir_all(storage_root).expect("remove preview storage root");
}

async fn preview_test_bucket(
    state: &AppState,
    application_id: ApplicationId,
    name: &str,
) -> Bucket {
    let bucket = Bucket::new(
        BucketId::new(),
        application_id,
        name,
        BucketPolicy::unrestricted(Visibility::Private),
        OffsetDateTime::now_utc(),
    )
    .expect("preview bucket");
    state
        .repository
        .create_bucket(&bucket)
        .await
        .expect("persist preview bucket");
    bucket
}

async fn preview_test_data_version(
    state: &AppState,
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: &str,
    content_type: &str,
    content: &[u8],
) -> (mediahub_core::ObjectVersionId, String) {
    let now = OffsetDateTime::now_utc();
    let object_id = mediahub_core::ObjectId::new();
    let version_id = mediahub_core::ObjectVersionId::new();
    let temporary_key = format!("preview-tests/staging/{version_id}");
    let storage_key = format!("preview-tests/objects/{version_id}");
    state
        .object_store
        .put_temporary(&temporary_key, content, content_type)
        .await
        .expect("write preview object");
    state
        .object_store
        .commit_temporary(&temporary_key, &storage_key)
        .await
        .expect("commit preview object");
    let etag = format!("preview-{version_id}");
    let checksum = hex::encode(Sha256::digest(content));
    let payload = mediahub_core::StoredObjectVersion::new(
        state.object_store.backend_name(),
        storage_key,
        None,
        None,
        mediahub_core::EntityTag::new(etag.clone()).expect("preview ETag"),
        content.len() as u64,
        Some(content_type.to_owned()),
        serde_json::json!({}),
        Some(mediahub_core::Checksum::sha256_hex(checksum).expect("preview checksum")),
    )
    .expect("preview payload");
    let object =
        mediahub_core::S3Object::new(object_id, application_id, bucket_id, object_key, now)
            .expect("preview logical object");
    let version = mediahub_core::ObjectVersion::new_object(
        version_id,
        object_id,
        application_id,
        bucket_id,
        mediahub_core::S3VersionId::new(version_id.to_string()).expect("external version ID"),
        1,
        false,
        mediahub_core::ObjectVersionState::Committed,
        payload,
        None,
        false,
        "preview-test",
        mediahub_core::SourceProtocol::Json,
        now,
    )
    .expect("preview object version");
    state
        .repository
        .create_s3_object_with_version(object, version, now)
        .await
        .expect("persist preview object version");
    (version_id, etag)
}

async fn preview_test_delete_marker(
    state: &AppState,
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: &str,
) -> mediahub_core::ObjectVersionId {
    let now = OffsetDateTime::now_utc();
    let object_id = mediahub_core::ObjectId::new();
    let version_id = mediahub_core::ObjectVersionId::new();
    let object =
        mediahub_core::S3Object::new(object_id, application_id, bucket_id, object_key, now)
            .expect("delete-marker logical object");
    let version = mediahub_core::ObjectVersion::new_delete_marker(
        version_id,
        object_id,
        application_id,
        bucket_id,
        mediahub_core::S3VersionId::new(version_id.to_string()).expect("delete-marker version ID"),
        1,
        false,
        "preview-test",
        mediahub_core::SourceProtocol::Json,
        now,
    )
    .expect("delete-marker version");
    state
        .repository
        .create_s3_object_with_version(object, version, now)
        .await
        .expect("persist delete-marker version");
    version_id
}
