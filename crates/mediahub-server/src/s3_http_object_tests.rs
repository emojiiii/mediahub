#[test]
fn s3_object_http_content_md5_parser_accepts_exact_sixteen_byte_digest() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("content-md5"),
        HeaderValue::from_static("XUFAKrxLKna5cZ2REBfFkg=="),
    );
    assert_eq!(
        parse_content_md5(&headers, "/bucket/key", "request").expect("valid digest"),
        Some("5d41402abc4b2a76b9719d911017c592".to_owned())
    );

    headers.insert(
        HeaderName::from_static("content-md5"),
        HeaderValue::from_static("not-base64"),
    );
    let error = parse_content_md5(&headers, "/bucket/key", "request")
        .expect_err("invalid digest must fail");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.code, "InvalidDigest");
}

#[test]
fn s3_object_http_metadata_parser_builds_a_bounded_json_object() {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-amz-meta-project"),
        HeaderValue::from_static("prismark"),
    );
    headers.insert(
        HeaderName::from_static("x-amz-meta-facet"),
        HeaderValue::from_static("preview"),
    );
    assert_eq!(
        s3_user_metadata(&headers, "/bucket/key", "request").expect("metadata"),
        serde_json::json!({"project": "prismark", "facet": "preview"})
    );

    let oversized = "x".repeat(MAX_S3_USER_METADATA_BYTES + 1);
    headers.insert(
        HeaderName::from_static("x-amz-meta-project"),
        HeaderValue::from_str(&oversized).expect("header value"),
    );
    let error = s3_user_metadata(&headers, "/bucket/key", "request")
        .expect_err("oversized metadata must fail");
    assert_eq!(error.code, "MetadataTooLarge");
}

#[test]
fn s3_object_http_single_range_parser_supports_closed_open_and_suffix_ranges() {
    assert_eq!(
        parse_s3_single_range("bytes=2-5", 10, "/bucket/key", "request").expect("closed"),
        S3ByteRange {
            start: 2,
            end_exclusive: 6
        }
    );
    assert_eq!(
        parse_s3_single_range("bytes=7-", 10, "/bucket/key", "request").expect("open"),
        S3ByteRange {
            start: 7,
            end_exclusive: 10
        }
    );
    assert_eq!(
        parse_s3_single_range("bytes=-3", 10, "/bucket/key", "request").expect("suffix"),
        S3ByteRange {
            start: 7,
            end_exclusive: 10
        }
    );
    let error = parse_s3_single_range("bytes=1-2,4-5", 10, "/bucket/key", "request")
        .expect_err("multiple ranges must fail");
    assert_eq!(error.status, StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(error.code, "InvalidRange");
}

#[test]
fn s3_object_http_condition_parser_uses_strong_if_match_and_weak_if_none_match() {
    let mut headers = HeaderMap::new();
    headers.insert(IF_MATCH, HeaderValue::from_static("W/\"abc\""));
    assert!(
        !if_match_satisfied(&headers, "abc", "/bucket/key", "request")
            .expect("If-Match parse")
    );
    headers.insert(IF_MATCH, HeaderValue::from_static("\"abc\""));
    assert!(
        if_match_satisfied(&headers, "abc", "/bucket/key", "request")
            .expect("If-Match parse")
    );
    headers.insert(IF_NONE_MATCH, HeaderValue::from_static("W/\"abc\""));
    assert!(
        if_none_match_matches_s3(&headers, "abc", "/bucket/key", "request")
            .expect("If-None-Match parse")
    );
}

#[test]
fn s3_object_http_headers_include_version_metadata_range_and_http_date() {
    let payload = StoredObjectVersion::new(
        "filesystem",
        "s3/objects/version",
        None,
        None,
        mediahub_core::EntityTag::new("5d41402abc4b2a76b9719d911017c592").expect("etag"),
        10,
        Some("text/plain".to_owned()),
        serde_json::json!({"project": "prismark"}),
        None,
    )
    .expect("payload");
    let version = ObjectVersion::new_object(
        mediahub_core::ObjectVersionId::new(),
        mediahub_core::ObjectId::new(),
        ApplicationId::new(),
        BucketId::new(),
        S3VersionId::new("v1").expect("version id"),
        1,
        false,
        mediahub_core::ObjectVersionState::Committed,
        payload.clone(),
        None,
        false,
        "access-key",
        mediahub_core::SourceProtocol::S3,
        OffsetDateTime::UNIX_EPOCH,
    )
    .expect("version");
    let mut headers = HeaderMap::new();
    insert_s3_object_headers(
        &mut headers,
        &version,
        &payload,
        Some(&S3ByteRange {
            start: 2,
            end_exclusive: 6,
        }),
        2,
    )
    .expect("response headers");
    assert_eq!(headers.get(CONTENT_LENGTH).expect("length"), "4");
    assert_eq!(headers.get(CONTENT_RANGE).expect("range"), "bytes 2-5/10");
    assert_eq!(headers.get(CONTENT_TYPE).expect("type"), "text/plain");
    assert_eq!(headers.get(ETAG).expect("etag"), "\"5d41402abc4b2a76b9719d911017c592\"");
    assert_eq!(headers.get(LAST_MODIFIED).expect("modified"), "Thu, 01 Jan 1970 00:00:00 GMT");
    assert_eq!(headers.get("x-amz-version-id").expect("version"), "v1");
    assert_eq!(headers.get("x-amz-tagging-count").expect("tag count"), "2");
    assert_eq!(headers.get("x-amz-meta-project").expect("metadata"), "prismark");
}

#[test]
fn s3_object_http_service_errors_map_to_protocol_codes() {
    let bucket = map_s3_object_service_error(
        S3ObjectServiceError::BucketNotFound,
        "/bucket/key",
        "request",
    );
    assert_eq!((bucket.status, bucket.code), (StatusCode::NOT_FOUND, "NoSuchBucket"));

    let version = map_s3_object_service_error(
        S3ObjectServiceError::VersionNotFound,
        "/bucket/key",
        "request",
    );
    assert_eq!((version.status, version.code), (StatusCode::NOT_FOUND, "NoSuchVersion"));

    let conflict = map_s3_object_service_error(
        S3ObjectServiceError::Repository(RepositoryError::Conflict),
        "/bucket/key",
        "request",
    );
    assert_eq!((conflict.status, conflict.code), (StatusCode::CONFLICT, "OperationAborted"));

    let unavailable = map_s3_object_service_error(
        S3ObjectServiceError::Repository(RepositoryError::Unavailable("database".to_owned())),
        "/bucket/key",
        "request",
    );
    assert_eq!(
        (unavailable.status, unavailable.code),
        (StatusCode::SERVICE_UNAVAILABLE, "ServiceUnavailable")
    );
}

#[tokio::test]
async fn s3_object_http_delete_marker_error_is_standard_s3_xml_with_marker_headers() {
    let error = map_s3_object_service_error(
        S3ObjectServiceError::DeleteMarker {
            version_id: S3VersionId::new("delete-v1").expect("version"),
            is_current: true,
        },
        "/bucket/key",
        "request",
    );
    let response = error.into_response();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(response.headers().get("x-amz-delete-marker").expect("marker"), "true");
    assert_eq!(response.headers().get("x-amz-version-id").expect("version"), "delete-v1");
    let body = to_bytes(response.into_body(), MAX_ERROR_RESPONSE_BYTES)
        .await
        .expect("XML body");
    let body = String::from_utf8(body.to_vec()).expect("UTF-8 XML");
    assert!(body.contains("<Code>NoSuchKey</Code>"));
    assert!(body.contains("<RequestId>request</RequestId>"));

    let explicit = map_s3_object_service_error(
        S3ObjectServiceError::DeleteMarker {
            version_id: S3VersionId::new("delete-v1").expect("version"),
            is_current: false,
        },
        "/bucket/key?versionId=delete-v1",
        "request",
    )
    .into_response();
    assert_eq!(explicit.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(
        explicit.headers().get("x-amz-delete-marker").expect("marker"),
        "true"
    );
}

#[test]
fn multipart_takeover_reuses_attached_intent_without_composing_again() {
    let source = include_str!("s3_http_multipart.rs");
    let helper = source
        .split("async fn claim_existing_multipart_intent")
        .nth(1)
        .expect("takeover helper")
        .split("fn canonical_completed_part_etag")
        .next()
        .expect("takeover helper boundary");
    assert!(helper.contains("find_upload_intent"));
    assert!(helper.contains("UploadIntentState::Ready"));
    assert!(helper.contains("UploadIntentState::Committing"));
    assert!(helper.contains("lease_until().is_some_and(|until| until <= now)"));
    assert!(helper.contains("claim_upload_intent"));
    assert!(!helper.contains("compose_temporary"));
    assert!(!helper.contains("begin_put"));
}
