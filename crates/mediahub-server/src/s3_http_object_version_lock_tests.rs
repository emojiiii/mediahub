#[cfg(test)]
mod object_version_lock_protocol_tests {
    use axum::http::{HeaderMap, HeaderValue, Uri};
    use mediahub_core::{ObjectRetention, OffsetDateTime, RetentionMode};
    use time::Duration;

    use super::{
        S3ObjectVersionLockOperation, classify_s3_object_version_lock,
        has_s3_object_lock_headers, parse_put_object_lock_headers,
        parse_s3_object_legal_hold_xml, parse_s3_object_retention_xml,
        reject_copy_object_lock_headers, s3_object_legal_hold_xml, s3_object_retention_xml,
    };

    #[test]
    fn object_lock_subresources_are_strict_and_mutually_exclusive() {
        assert_eq!(
            classify_s3_object_version_lock(
                &Uri::from_static("/bucket/key?retention&versionId=v1"),
                "request",
            )
            .expect("classification"),
            Some(S3ObjectVersionLockOperation::Retention)
        );
        assert_eq!(
            classify_s3_object_version_lock(
                &Uri::from_static("/bucket/key?legal-hold"),
                "request",
            )
            .expect("classification"),
            Some(S3ObjectVersionLockOperation::LegalHold)
        );
        assert!(
            classify_s3_object_version_lock(
                &Uri::from_static("/bucket/key?retention=wrong"),
                "request",
            )
            .is_err()
        );
        assert!(
            classify_s3_object_version_lock(
                &Uri::from_static("/bucket/key?retention&legal-hold"),
                "request",
            )
            .is_err()
        );
        assert!(
            classify_s3_object_version_lock(
                &Uri::from_static("/bucket/key?retention&uploadId=u1"),
                "request",
            )
            .is_err()
        );
    }

    #[test]
    fn put_object_lock_headers_require_pairs_future_dates_and_exact_values() {
        let uri = Uri::from_static("/bucket/key");
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-object-lock-mode",
            HeaderValue::from_static("GOVERNANCE"),
        );
        assert!(parse_put_object_lock_headers(&headers, &uri, "request", now).is_err());

        headers.insert(
            "x-amz-object-lock-retain-until-date",
            HeaderValue::from_static("2030-01-01T00:00:00Z"),
        );
        headers.insert(
            "x-amz-object-lock-legal-hold",
            HeaderValue::from_static("OFF"),
        );
        let parsed = parse_put_object_lock_headers(&headers, &uri, "request", now)
            .expect("headers");
        assert_eq!(parsed.retention.expect("retention").mode(), RetentionMode::Governance);
        assert_eq!(parsed.legal_hold, Some(false));
        assert!(has_s3_object_lock_headers(&headers));

        headers.insert(
            "x-amz-object-lock-legal-hold",
            HeaderValue::from_static("INVALID"),
        );
        assert!(parse_put_object_lock_headers(&headers, &uri, "request", now).is_err());
    }

    #[test]
    fn retention_and_legal_hold_xml_are_strict_and_round_trip() {
        let uri = Uri::from_static("/bucket/key?retention");
        let retain_until = OffsetDateTime::now_utc() + Duration::days(30);
        let retention = ObjectRetention::new(RetentionMode::Compliance, retain_until);
        let xml = s3_object_retention_xml(retention).expect("retention XML");
        let parsed = parse_s3_object_retention_xml(xml.as_bytes(), &uri, "request")
            .expect("retention");
        assert_eq!(parsed.mode(), RetentionMode::Compliance);
        assert_eq!(parsed.retain_until(), retain_until);

        let legal_xml = s3_object_legal_hold_xml(true);
        assert!(
            parse_s3_object_legal_hold_xml(legal_xml.as_bytes(), &uri, "request")
                .expect("legal hold")
        );
        assert!(
            parse_s3_object_retention_xml(
                br#"<Retention><RetainUntilDate>2099-01-01T00:00:00Z</RetainUntilDate><Mode>GOVERNANCE</Mode></Retention>"#,
                &uri,
                "request",
            )
            .is_err()
        );
        assert!(
            parse_s3_object_legal_hold_xml(
                br#"<LegalHold><Status>YES</Status></LegalHold>"#,
                &uri,
                "request",
            )
            .is_err()
        );
    }

    #[test]
    fn copy_object_rejects_lock_headers_instead_of_silently_dropping_them() {
        let uri = Uri::from_static("/bucket/copy");
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-object-lock-mode",
            HeaderValue::from_static("GOVERNANCE"),
        );
        let error = reject_copy_object_lock_headers(&headers, &uri, "request")
            .expect_err("CopyObject lock headers must be rejected");
        assert_eq!(error.code, "InvalidRequest");
        assert!(error.message.contains("CopyObject Object Lock"));
    }
}
