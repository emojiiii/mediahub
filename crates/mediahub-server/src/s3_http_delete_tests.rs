#[test]
fn s3_delete_response_covers_idempotent_and_marker_headers() {
    let idempotent = s3_delete_object_response(
        &DeleteObjectReceipt {
            version_id: None,
            delete_marker: false,
        },
        "/bucket/key",
        "request",
    )
    .expect("idempotent response");
    assert_eq!(idempotent.status(), StatusCode::NO_CONTENT);
    assert!(!idempotent.headers().contains_key("x-amz-version-id"));
    assert!(!idempotent.headers().contains_key("x-amz-delete-marker"));

    let marker = s3_delete_object_response(
        &DeleteObjectReceipt {
            version_id: Some(S3VersionId::new("marker-v1").expect("version")),
            delete_marker: true,
        },
        "/bucket/key",
        "request",
    )
    .expect("marker response");
    assert_eq!(marker.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        marker.headers().get("x-amz-version-id").expect("version"),
        "marker-v1"
    );
    assert_eq!(
        marker
            .headers()
            .get("x-amz-delete-marker")
            .expect("marker"),
        "true"
    );
}

#[test]
fn s3_delete_requires_a_strict_signed_governance_header() {
    let uri = Uri::from_static("/bucket/key");
    let mut signed = HeaderMap::new();
    signed.insert(
        "authorization",
        HeaderValue::from_static(
            "AWS4-HMAC-SHA256 Credential=test, SignedHeaders=host;x-amz-bypass-governance-retention, Signature=00",
        ),
    );
    signed.insert(
        "x-amz-bypass-governance-retention",
        HeaderValue::from_static("true"),
    );
    assert!(parse_s3_bypass_governance(&signed, &uri, "request").expect("signed true"));

    signed.insert(
        "x-amz-bypass-governance-retention",
        HeaderValue::from_static("false"),
    );
    assert!(!parse_s3_bypass_governance(&signed, &uri, "request").expect("signed false"));

    signed.insert(
        "x-amz-bypass-governance-retention",
        HeaderValue::from_static("TRUE"),
    );
    let invalid = parse_s3_bypass_governance(&signed, &uri, "request").expect_err("strict bool");
    assert_eq!((invalid.status, invalid.code), (StatusCode::BAD_REQUEST, "InvalidArgument"));

    let mut unsigned = HeaderMap::new();
    unsigned.insert(
        "authorization",
        HeaderValue::from_static(
            "AWS4-HMAC-SHA256 Credential=test, SignedHeaders=host, Signature=00",
        ),
    );
    unsigned.insert(
        "x-amz-bypass-governance-retention",
        HeaderValue::from_static("true"),
    );
    let denied =
        parse_s3_bypass_governance(&unsigned, &uri, "request").expect_err("unsigned bypass");
    assert_eq!((denied.status, denied.code), (StatusCode::FORBIDDEN, "AccessDenied"));
}

#[test]
fn s3_delete_maps_missing_version_and_all_lock_reasons_without_sensitive_details() {
    let missing = map_s3_object_service_error(
        S3ObjectServiceError::VersionNotFound,
        "/bucket/key",
        "request",
    );
    assert_eq!((missing.status, missing.code), (StatusCode::NOT_FOUND, "NoSuchVersion"));

    let cases = [
        (S3DeleteLockReason::LegalHold, "legal hold"),
        (S3DeleteLockReason::ComplianceRetention, "compliance retention"),
        (S3DeleteLockReason::GovernanceRetention, "governance retention"),
    ];
    for (reason, expected) in cases {
        let error = map_s3_object_service_error(
            S3ObjectServiceError::DeleteLocked(reason),
            "/bucket/key",
            "request",
        );
        assert_eq!((error.status, error.code), (StatusCode::FORBIDDEN, "AccessDenied"));
        assert!(error.message.contains(expected));
        assert!(!error.message.contains("202"));
    }
}

#[test]
fn s3_batch_delete_records_versions_markers_quiet_and_per_item_errors() {
    let mut result = DeleteResult::default();
    record_s3_batch_delete_result(
        &mut result,
        "exact<&>".to_owned(),
        Some("data-v1".to_owned()),
        false,
        Ok(DeleteObjectReceipt {
            version_id: Some(S3VersionId::new("data-v1").expect("version")),
            delete_marker: false,
        }),
    );
    record_s3_batch_delete_result(
        &mut result,
        "marker".to_owned(),
        None,
        false,
        Ok(DeleteObjectReceipt {
            version_id: Some(S3VersionId::new("marker-v1").expect("version")),
            delete_marker: true,
        }),
    );
    record_s3_batch_delete_result(
        &mut result,
        "quiet".to_owned(),
        None,
        true,
        Ok(DeleteObjectReceipt {
            version_id: None,
            delete_marker: false,
        }),
    );
    record_s3_batch_delete_result(
        &mut result,
        "missing".to_owned(),
        Some("absent-v1".to_owned()),
        true,
        Err(S3ObjectServiceError::VersionNotFound),
    );

    assert_eq!(result.deleted.len(), 2);
    assert_eq!(result.deleted[0].version_id.as_deref(), Some("data-v1"));
    assert!(!result.deleted[0].delete_marker);
    assert!(result.deleted[1].delete_marker);
    assert_eq!(
        result.deleted[1].delete_marker_version_id.as_deref(),
        Some("marker-v1")
    );
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.errors[0].version_id.as_deref(), Some("absent-v1"));
    assert_eq!(result.errors[0].code, "NoSuchVersion");

    let xml = delete_result_xml(&result).expect("DeleteResult XML");
    assert!(xml.contains("<Key>exact&lt;&amp;&gt;</Key><VersionId>data-v1</VersionId>"));
    assert!(xml.contains("<DeleteMarker>true</DeleteMarker>"));
    assert!(xml.contains("<DeleteMarkerVersionId>marker-v1</DeleteMarkerVersionId>"));
    assert!(xml.contains("<Error><Key>missing</Key><VersionId>absent-v1</VersionId>"));
    assert!(!xml.contains("quiet"));
}

#[test]
fn regular_delete_http_paths_do_not_use_media_deletion_scheduling() {
    let core = include_str!("s3_http_core.rs");
    let core = core
        .split_once("pub(super) async fn s3_bucket_post(")
        .expect("DeleteObjects handler")
        .1
        .split_once("pub(super) async fn s3_put_object(")
        .expect("next handler")
        .0;
    let support = include_str!("s3_http_support.rs");
    let support = support
        .split_once("pub(super) async fn s3_delete_object(")
        .expect("DeleteObject handler")
        .1
        .split_once("#[derive(Debug)]\npub(super) struct S3ApiError")
        .expect("S3 error type")
        .0;
    for source in [core, support] {
        assert!(!source.contains("schedule_s3_delete"));
        assert!(!source.contains("find_by_object_key"));
        assert!(!source.contains("MediaState"));
        assert!(!source.contains("media_delete_scheduled"));
        assert!(!source.contains("schedule_delete"));
    }
}
