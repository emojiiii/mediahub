#[cfg(test)]
mod object_lock_protocol_tests {
    use axum::response::IntoResponse;
    use http::{HeaderMap, HeaderValue, StatusCode, Uri};
    use mediahub_core::{DefaultRetentionPeriod, RetentionMode};

    use super::{
        AppState, S3ApiError, S3BucketConfigurationXmlError, S3BucketGetOperation,
        S3BucketPutOperation, classify_s3_bucket_delete, classify_s3_bucket_get,
        classify_s3_bucket_put, parse_s3_bucket_object_lock_xml,
        parse_s3_create_bucket_object_lock_header, s3_bucket_get, s3_bucket_object_lock_xml,
        s3_bucket_put, validate_s3_configuration_content_md5,
    };

    #[test]
    fn object_lock_handlers_are_axum_route_compatible() {
        let _: axum::routing::MethodRouter<std::sync::Arc<AppState>> =
            axum::routing::get(s3_bucket_get).put(s3_bucket_put);
    }

    #[test]
    fn create_bucket_object_lock_header_accepts_only_one_literal_true() {
        let uri = Uri::from_static("/locked-bucket");
        assert!(!parse_s3_create_bucket_object_lock_header(
            &HeaderMap::new(),
            &uri,
            "request-id",
        )
        .expect("absent header"));

        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-bucket-object-lock-enabled",
            HeaderValue::from_static("true"),
        );
        assert!(parse_s3_create_bucket_object_lock_header(&headers, &uri, "request-id")
            .expect("literal true"));

        for invalid in ["false", "TRUE", "1", "yes"] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-amz-bucket-object-lock-enabled",
                HeaderValue::from_str(invalid).expect("valid header bytes"),
            );
            let error =
                parse_s3_create_bucket_object_lock_header(&headers, &uri, "request-id")
                    .expect_err("invalid Object Lock create header");
            assert_eq!(error.status, StatusCode::BAD_REQUEST);
            assert_eq!(error.code, "InvalidArgument");
        }

        let mut duplicate = HeaderMap::new();
        duplicate.append(
            "x-amz-bucket-object-lock-enabled",
            HeaderValue::from_static("true"),
        );
        duplicate.append(
            "x-amz-bucket-object-lock-enabled",
            HeaderValue::from_static("true"),
        );
        let error = parse_s3_create_bucket_object_lock_header(&duplicate, &uri, "request-id")
            .expect_err("duplicate Object Lock create header");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.code, "InvalidRequest");
    }

    #[test]
    fn object_lock_xml_round_trips_supported_configurations() {
        for (mode, period, expected_mode, expected_period) in [
            (
                RetentionMode::Governance,
                DefaultRetentionPeriod::Days(30),
                "GOVERNANCE",
                "<Days>30</Days>",
            ),
            (
                RetentionMode::Compliance,
                DefaultRetentionPeriod::Years(2),
                "COMPLIANCE",
                "<Years>2</Years>",
            ),
        ] {
            let input = format!(
                "<ObjectLockConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><ObjectLockEnabled>Enabled</ObjectLockEnabled><Rule><DefaultRetention><Mode>{expected_mode}</Mode>{expected_period}</DefaultRetention></Rule></ObjectLockConfiguration>"
            );
            let retention = parse_s3_bucket_object_lock_xml(input.as_bytes())
                .expect("supported Object Lock XML")
                .expect("default retention");
            assert_eq!(retention.mode(), mode);
            assert_eq!(retention.period(), period);
            let rendered = s3_bucket_object_lock_xml(Some(retention));
            assert!(rendered.contains(expected_mode));
            assert!(rendered.contains(expected_period));
            assert_eq!(
                parse_s3_bucket_object_lock_xml(rendered.as_bytes())
                    .expect("rendered Object Lock XML"),
                Some(retention),
            );
        }

        let enabled_only = parse_s3_bucket_object_lock_xml(
            br#"<ObjectLockConfiguration><ObjectLockEnabled>Enabled</ObjectLockEnabled></ObjectLockConfiguration>"#,
        )
        .expect("Object Lock without a default rule");
        assert_eq!(enabled_only, None);
        assert!(!s3_bucket_object_lock_xml(None).contains("<Rule>"));
    }

    #[test]
    fn object_lock_xml_rejects_disablement_and_every_ambiguous_shape() {
        let invalid = [
            "<ObjectLockConfiguration/>",
            "<ObjectLockConfiguration><ObjectLockEnabled>Disabled</ObjectLockEnabled></ObjectLockConfiguration>",
            "<ObjectLockConfiguration><ObjectLockEnabled>Enabled</ObjectLockEnabled><Unknown/></ObjectLockConfiguration>",
            "<ObjectLockConfiguration><Rule><DefaultRetention><Mode>GOVERNANCE</Mode><Days>1</Days></DefaultRetention></Rule><ObjectLockEnabled>Enabled</ObjectLockEnabled></ObjectLockConfiguration>",
            "<ObjectLockConfiguration><ObjectLockEnabled>Enabled</ObjectLockEnabled><Rule><DefaultRetention><Mode>GOVERNANCE</Mode><Days>1</Days><Years>1</Years></DefaultRetention></Rule></ObjectLockConfiguration>",
            "<ObjectLockConfiguration><ObjectLockEnabled>Enabled</ObjectLockEnabled><Rule><DefaultRetention><Mode>GOVERNANCE</Mode><Days>0</Days></DefaultRetention></Rule></ObjectLockConfiguration>",
            "<ObjectLockConfiguration><ObjectLockEnabled>Enabled</ObjectLockEnabled><Rule><DefaultRetention><Mode>STANDARD</Mode><Days>1</Days></DefaultRetention></Rule></ObjectLockConfiguration>",
        ];
        for xml in invalid {
            assert!(
                parse_s3_bucket_object_lock_xml(xml.as_bytes()).is_err(),
                "{xml}"
            );
        }

        assert_eq!(
            parse_s3_bucket_object_lock_xml(invalid[1].as_bytes()),
            Err(S3BucketConfigurationXmlError::Invalid(
                "ObjectLockEnabled must be Enabled.".to_owned()
            )),
        );
    }

    #[test]
    fn object_lock_subresource_is_classified_and_cannot_be_deleted() {
        let get_uri = Uri::from_static("/locked-bucket?object-lock&x-id=GetObjectLockConfiguration");
        assert_eq!(
            classify_s3_bucket_get(&get_uri, "request-id").expect("GET object-lock"),
            S3BucketGetOperation::GetObjectLock,
        );
        assert_eq!(
            classify_s3_bucket_put(&Uri::from_static("/locked-bucket?object-lock"), "request-id")
                .expect("PUT object-lock"),
            S3BucketPutOperation::PutObjectLock,
        );
        let delete = classify_s3_bucket_delete(
            &Uri::from_static("/locked-bucket?object-lock"),
            "request-id",
        )
        .expect_err("Object Lock cannot be deleted");
        assert_eq!(delete.status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(delete.code, "MethodNotAllowed");

        for uri in [
            "/locked-bucket?object-lock&versioning",
            "/locked-bucket?object-lock&lifecycle",
            "/locked-bucket?object-lock&max-keys=1",
        ] {
            let error = classify_s3_bucket_get(&uri.parse().expect("URI"), "request-id")
                .expect_err("ambiguous Object Lock request");
            assert_eq!(error.code, "InvalidRequest");
        }
    }

    #[tokio::test]
    async fn object_lock_http_errors_are_standard_xml_and_put_requires_content_md5() {
        let error = S3ApiError::object_lock_configuration_not_found(
            "/locked-bucket",
            "request-id",
        );
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("error body");
        assert!(std::str::from_utf8(&body)
            .expect("UTF-8")
            .contains("<Code>ObjectLockConfigurationNotFoundError</Code>"));

        let uri = Uri::from_static("/locked-bucket?object-lock");
        let missing_md5 = validate_s3_configuration_content_md5(
            &HeaderMap::new(),
            b"<ObjectLockConfiguration/>",
            &uri,
            "request-id",
        )
        .expect_err("Content-MD5 is mandatory");
        assert_eq!(missing_md5.status, StatusCode::BAD_REQUEST);
        assert_eq!(missing_md5.code, "InvalidDigest");
    }
}
