#[cfg(test)]
mod bucket_protocol_tests {
    use axum::response::IntoResponse;
    use http::{StatusCode, Uri};
    use mediahub_core::{
        ApplicationId, Bucket, BucketId, BucketPolicy, OffsetDateTime, Visibility,
    };

    use super::{
        AppState, DEFAULT_S3_REGION, S3ApiError, S3BucketGetOperation,
        S3CreateBucketConfigurationError, classify_s3_bucket_get, s3_bucket_location_xml,
        s3_bucket_region_response, s3_create_bucket, s3_delete_bucket, s3_head_bucket,
        s3_list_buckets, s3_list_buckets_xml, s3_list_objects, validate_s3_bucket_name,
        validate_s3_create_bucket_configuration,
    };

    #[test]
    fn bucket_handlers_are_axum_route_compatible() {
        let _: axum::routing::MethodRouter<std::sync::Arc<AppState>> =
            axum::routing::get(s3_list_buckets);
        let _: axum::routing::MethodRouter<std::sync::Arc<AppState>> =
            axum::routing::get(s3_list_objects)
                .head(s3_head_bucket)
                .put(s3_create_bucket)
                .delete(s3_delete_bucket);
    }

    #[test]
    fn validates_general_purpose_bucket_names() {
        for valid in ["abc", "generated-images", "assets.example.com", "a1-b2.c3"] {
            assert_eq!(validate_s3_bucket_name(valid), Ok(()), "{valid}");
        }

        for invalid in [
            "ab",
            "UPPERCASE",
            "contains_underscore",
            "-starts-with-hyphen",
            "ends-with-hyphen-",
            "two..dots",
            "192.168.1.1",
            "xn--reserved",
            "reserved-s3alias",
            "reserved--ol-s3",
            "reserved.mrap",
            "reserved--x-s3",
            "reserved--table-s3",
        ] {
            assert_eq!(validate_s3_bucket_name(invalid), Err(()), "{invalid}");
        }
    }

    #[test]
    fn create_bucket_configuration_accepts_only_the_default_region_shape() {
        assert_eq!(validate_s3_create_bucket_configuration(b""), Ok(()));
        assert_eq!(validate_s3_create_bucket_configuration(b" \r\n\t"), Ok(()));
        assert_eq!(
            validate_s3_create_bucket_configuration(
                br#"<CreateBucketConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/"/>"#,
            ),
            Ok(())
        );

        for region in ["us-east-1", "eu-west-1", "EU", ""] {
            let xml = format!(
                "<CreateBucketConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><LocationConstraint>{region}</LocationConstraint></CreateBucketConfiguration>"
            );
            assert_eq!(
                validate_s3_create_bucket_configuration(xml.as_bytes()),
                Err(S3CreateBucketConfigurationError::InvalidLocationConstraint),
                "{region}"
            );
        }
    }

    #[test]
    fn create_bucket_configuration_rejects_malformed_or_extended_xml() {
        for xml in [
            "<WrongRoot/>",
            "<CreateBucketConfiguration><Unknown/></CreateBucketConfiguration>",
            "<CreateBucketConfiguration>",
            "<!DOCTYPE CreateBucketConfiguration><CreateBucketConfiguration/>",
            "<CreateBucketConfiguration unexpected=\"true\"/>",
        ] {
            assert_eq!(
                validate_s3_create_bucket_configuration(xml.as_bytes()),
                Err(S3CreateBucketConfigurationError::MalformedXml),
                "{xml}"
            );
        }
    }

    #[test]
    fn list_buckets_xml_has_aws_shape_and_escapes_owner() {
        let bucket = Bucket::new(
            BucketId::new(),
            ApplicationId::new(),
            "generated-images",
            BucketPolicy::unrestricted(Visibility::Private),
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("bucket");
        let xml = s3_list_buckets_xml("app<&>", "Prism & Ark", &[bucket]).expect("XML");

        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains(
            "<ListAllMyBucketsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">"
        ));
        assert!(xml.contains("<ID>app&lt;&amp;&gt;</ID>"));
        assert!(xml.contains("<DisplayName>Prism &amp; Ark</DisplayName>"));
        assert!(xml.contains("<Name>generated-images</Name>"));
        assert!(xml.contains("<CreationDate>1970-01-01T00:00:00Z</CreationDate>"));
        assert!(xml.ends_with("</Buckets></ListAllMyBucketsResult>"));
    }

    #[test]
    fn bucket_location_is_null_for_us_east_1() {
        assert_eq!(
            s3_bucket_location_xml(),
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><LocationConstraint xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"></LocationConstraint>"
        );
        let response = s3_bucket_region_response(StatusCode::OK, "request-1");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-amz-bucket-region"], DEFAULT_S3_REGION);
        assert_eq!(response.headers()["x-amz-request-id"], "request-1");
    }

    #[test]
    fn classifies_bucket_location_before_authentication() {
        let location = Uri::from_static("/s3/assets?location");
        assert_eq!(
            classify_s3_bucket_get(&location, "request-1").expect("classify"),
            S3BucketGetOperation::GetLocation
        );

        let list = Uri::from_static("/s3/assets?list-type=2&prefix=images%2F");
        assert_eq!(
            classify_s3_bucket_get(&list, "request-1").expect("classify"),
            S3BucketGetOperation::ListObjects
        );

        let mixed = Uri::from_static("/s3/assets?location&list-type=2");
        let error = classify_s3_bucket_get(&mixed, "request-1").expect_err("mixed query");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.code, "InvalidRequest");
    }

    #[tokio::test]
    async fn bucket_errors_use_s3_codes_statuses_and_xml_bodies() {
        let cases = [
            (
                S3ApiError::invalid_bucket_name("/invalid", "request-1"),
                StatusCode::BAD_REQUEST,
                "InvalidBucketName",
            ),
            (
                S3ApiError::bucket_already_owned_by_you("/owned", "request-1"),
                StatusCode::CONFLICT,
                "BucketAlreadyOwnedByYou",
            ),
            (
                S3ApiError::bucket_not_empty("/not-empty", "request-1"),
                StatusCode::CONFLICT,
                "BucketNotEmpty",
            ),
            (
                S3ApiError::no_such_bucket("/missing", "request-1"),
                StatusCode::NOT_FOUND,
                "NoSuchBucket",
            ),
            (
                S3ApiError::invalid_location_constraint("/region", "request-1"),
                StatusCode::BAD_REQUEST,
                "InvalidLocationConstraint",
            ),
            (
                S3ApiError::malformed_xml("/xml", "request-1"),
                StatusCode::BAD_REQUEST,
                "MalformedXML",
            ),
        ];

        for (error, status, code) in cases {
            assert_eq!(error.status, status);
            assert_eq!(error.code, code);
            let response = error.into_response();
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()["content-type"], "application/xml");
            assert_eq!(response.headers()["x-amz-request-id"], "request-1");
            let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("error XML body");
            let body = std::str::from_utf8(&body).expect("UTF-8 error XML");
            assert!(body.contains(&format!("<Code>{code}</Code>")), "{body}");
            assert!(body.contains("<RequestId>request-1</RequestId>"), "{body}");
        }
    }
}
