#[test]
fn s3_bucket_policy_classifiers_are_exact_and_reject_mixed_subresources() {
    let request_id = "request-id";
    assert_eq!(
        classify_s3_bucket_get(&Uri::from_static("/assets?policy"), request_id)
            .expect("GetBucketPolicy"),
        S3BucketGetOperation::GetPolicy,
    );
    assert_eq!(
        classify_s3_bucket_get(&Uri::from_static("/assets?policyStatus"), request_id)
            .expect("GetBucketPolicyStatus"),
        S3BucketGetOperation::GetPolicyStatus,
    );
    assert_eq!(
        classify_s3_bucket_put(&Uri::from_static("/assets?policy="), request_id)
            .expect("PutBucketPolicy"),
        S3BucketPutOperation::PutPolicy,
    );
    assert_eq!(
        classify_s3_bucket_delete(&Uri::from_static("/assets?policy"), request_id)
            .expect("DeleteBucketPolicy"),
        S3BucketDeleteOperation::Policy,
    );

    for uri in [
        "/assets?policy&policyStatus",
        "/assets?policy&versioning",
        "/assets?policyStatus&uploads",
        "/assets?policy&max-keys=10",
    ] {
        let error = classify_s3_bucket_get(&uri.parse().expect("URI"), request_id)
            .expect_err("mixed query must fail before authentication");
        assert_eq!(error.status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(error.code, "InvalidRequest", "{uri}");
    }
    for uri in ["/assets?policy&lifecycle", "/assets?policy&object-lock"] {
        let uri = uri.parse().expect("URI");
        assert_eq!(
            classify_s3_bucket_put(&uri, request_id)
                .expect_err("mixed PUT query")
                .code,
            "InvalidRequest",
        );
        assert_eq!(
            classify_s3_bucket_delete(&uri, request_id)
                .expect_err("mixed DELETE query")
                .code,
            "InvalidRequest",
        );
    }

    assert_eq!(
        classify_s3_bucket_get(&Uri::from_static("/assets?policystatus"), request_id)
            .expect("query names are case-sensitive"),
        S3BucketGetOperation::ListObjects,
    );
}

#[test]
fn expected_bucket_owner_requires_one_twelve_digit_value() {
    let mut headers = HeaderMap::new();
    assert_eq!(
        parse_s3_expected_bucket_owner(&headers, "/assets", "request-id")
            .expect("header is optional"),
        None,
    );
    headers.insert(
        S3_EXPECTED_BUCKET_OWNER_HEADER,
        HeaderValue::from_static("123456789012"),
    );
    assert_eq!(
        parse_s3_expected_bucket_owner(&headers, "/assets", "request-id")
            .expect("valid owner"),
        Some("123456789012".to_owned()),
    );

    for invalid in ["12345678901", "12345678901x", "1234567890123"] {
        let mut headers = HeaderMap::new();
        headers.insert(
            S3_EXPECTED_BUCKET_OWNER_HEADER,
            HeaderValue::from_str(invalid).expect("header"),
        );
        let error = parse_s3_expected_bucket_owner(&headers, "/assets", "request-id")
            .expect_err("invalid expected owner");
        assert_eq!(error.code, "InvalidArgument");
    }

    let mut duplicate = HeaderMap::new();
    duplicate.append(
        S3_EXPECTED_BUCKET_OWNER_HEADER,
        HeaderValue::from_static("123456789012"),
    );
    duplicate.append(
        S3_EXPECTED_BUCKET_OWNER_HEADER,
        HeaderValue::from_static("123456789012"),
    );
    assert_eq!(
        parse_s3_expected_bucket_owner(&duplicate, "/assets", "request-id")
            .expect_err("duplicate expected owner")
            .code,
        "InvalidArgument",
    );
}

#[test]
fn policy_body_length_distinguishes_short_and_excess_payloads() {
    assert!(validate_s3_policy_body_length(3, 3, "/assets", "request-id").is_ok());
    let short = validate_s3_policy_body_length(4, 3, "/assets", "request-id")
        .expect_err("short body");
    assert_eq!(short.code, "IncompleteBody");
    let excess = validate_s3_policy_body_length(2, 3, "/assets", "request-id")
        .expect_err("excess body");
    assert_eq!(excess.code, "InvalidRequest");
}

#[test]
fn policy_content_md5_is_optional_but_strict_when_present() {
    let content = br#"{"Version":"2012-10-17","Statement":[]}"#;
    assert!(
        validate_s3_optional_policy_content_md5(
            &HeaderMap::new(),
            content,
            "/assets",
            "request-id",
        )
        .is_ok()
    );
    let mut valid = HeaderMap::new();
    valid.insert(
        "content-md5",
        HeaderValue::from_str(&STANDARD.encode(md5::Md5::digest(content))).expect("MD5 header"),
    );
    assert!(
        validate_s3_optional_policy_content_md5(
            &valid,
            content,
            "/assets",
            "request-id",
        )
        .is_ok()
    );
    valid.insert("content-md5", HeaderValue::from_static("not-base64"));
    assert_eq!(
        validate_s3_optional_policy_content_md5(
            &valid,
            content,
            "/assets",
            "request-id",
        )
        .expect_err("invalid MD5")
        .code,
        "InvalidDigest",
    );
}

#[tokio::test]
async fn bucket_policy_success_payloads_follow_s3_wire_shapes() {
    assert_eq!(
        s3_bucket_policy_status_xml(true),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><PolicyStatus xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><IsPublic>true</IsPublic></PolicyStatus>"
    );
    assert!(s3_bucket_policy_status_xml(false).contains("<IsPublic>false</IsPublic>"));

    let policy = S3BucketPolicy::parse(
        br#"{"Statement":{"Resource":"arn:aws:s3:::assets","Principal":"*","Action":"s3:ListBucket","Effect":"Allow"},"Version":"2012-10-17"}"#,
        "assets",
    )
    .expect("policy");
    let response = s3_policy_json_response(policy.to_stable_json(), "request-id");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
    assert_eq!(response.headers()["x-amz-request-id"], "request-id");
    let body = to_bytes(response.into_body(), MAX_S3_POLICY_BYTES)
        .await
        .expect("JSON body");
    let body = std::str::from_utf8(&body).expect("UTF-8");
    assert_eq!(
        body,
        "{\"Version\":\"2012-10-17\",\"Statement\":[{\"Effect\":\"Allow\",\"Principal\":\"*\",\"Action\":\"s3:ListBucket\",\"Resource\":\"arn:aws:s3:::assets\"}]}"
    );
}

#[tokio::test]
async fn bucket_policy_errors_use_standard_s3_codes() {
    for (error, status, code) in [
        (
            S3ApiError::no_such_bucket_policy("/assets", "request-id"),
            StatusCode::NOT_FOUND,
            "NoSuchBucketPolicy",
        ),
        (
            S3ApiError::policy_too_large("/assets", "request-id"),
            StatusCode::BAD_REQUEST,
            "PolicyTooLarge",
        ),
        (
            S3ApiError::malformed_policy("/assets", "request-id"),
            StatusCode::BAD_REQUEST,
            "MalformedPolicy",
        ),
        (
            S3ApiError::bucket_already_exists("/assets", "request-id"),
            StatusCode::CONFLICT,
            "BucketAlreadyExists",
        ),
    ] {
        assert_eq!(error.status, status);
        assert_eq!(error.code, code);
        let body = to_bytes(error.into_response().into_body(), MAX_ERROR_RESPONSE_BYTES)
            .await
            .expect("error XML");
        assert!(
            std::str::from_utf8(&body)
                .expect("UTF-8")
                .contains(&format!("<Code>{code}</Code>"))
        );
    }
}
