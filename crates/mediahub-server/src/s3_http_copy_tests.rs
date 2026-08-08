mod s3_copy_tests {
    use super::*;

    #[test]
    fn copy_source_decodes_key_and_version_without_confusing_encoded_query_chars() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-copy-source",
            HeaderValue::from_static("/source-bucket/folder%2Freport%3Ffinal.pdf?versionId=null"),
        );
        let source = parse_s3_copy_source(&headers, "/destination/key", "request")
            .expect("valid copy source");
        assert_eq!(source.bucket_name, "source-bucket");
        assert_eq!(source.object_key, "folder/report?final.pdf");
        assert_eq!(
            source.version_id.as_ref().map(S3VersionId::as_str),
            Some("null")
        );
    }

    #[test]
    fn copy_source_rejects_duplicate_or_unknown_query_parameters() {
        for value in [
            "/source-bucket/key?versionId=null&versionId=other",
            "/source-bucket/key?notVersionId=null",
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(
                "x-amz-copy-source",
                HeaderValue::from_str(value).expect("header"),
            );
            let error = parse_s3_copy_source(&headers, "/destination/key", "request")
                .expect_err("invalid query");
            assert_eq!(error.code, "InvalidArgument");
        }
    }

    #[test]
    fn upload_part_copy_range_is_strict_and_half_open_internally() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-copy-source-range",
            HeaderValue::from_static("bytes=5-9"),
        );
        assert_eq!(
            parse_s3_copy_source_range(&headers, 10, "/bucket/key", "request")
                .expect("valid range"),
            Some(S3ByteRange {
                start: 5,
                end_exclusive: 10,
            })
        );
        headers.insert(
            "x-amz-copy-source-range",
            HeaderValue::from_static("bytes=5-10"),
        );
        let error = parse_s3_copy_source_range(&headers, 10, "/bucket/key", "request")
            .expect_err("out of bounds");
        assert_eq!(error.code, "InvalidRange");
    }

    #[test]
    fn metadata_directive_defaults_to_copy_and_rejects_unknown_values() {
        let mut headers = HeaderMap::new();
        assert_eq!(
            parse_s3_metadata_directive(&headers, "/bucket/key", "request")
                .expect("default directive"),
            S3MetadataDirective::Copy
        );
        headers.insert(
            "x-amz-metadata-directive",
            HeaderValue::from_static("REPLACE"),
        );
        assert_eq!(
            parse_s3_metadata_directive(&headers, "/bucket/key", "request")
                .expect("replace directive"),
            S3MetadataDirective::Replace
        );
        headers.insert(
            "x-amz-metadata-directive",
            HeaderValue::from_static("MERGE"),
        );
        assert_eq!(
            parse_s3_metadata_directive(&headers, "/bucket/key", "request")
                .expect_err("unknown directive")
                .code,
            "InvalidArgument"
        );
    }

    #[test]
    fn copy_routes_tagging_to_directive_parser_and_rejects_storage_classes() {
        let mut headers = HeaderMap::new();
        headers.insert(
            "x-amz-tagging",
            HeaderValue::from_static("project=prismark"),
        );
        reject_unsupported_s3_copy_headers(&headers, false, "/bucket/key", "request")
            .expect("CopyObject tagging is handled by its directive parser");
        assert_eq!(
            parse_s3_tagging_header(&headers, "/bucket/key", "request")
                .expect("tag header")
                .expect("tags")
                .len(),
            1
        );
        headers.remove("x-amz-tagging");
        headers.insert("x-amz-storage-class", HeaderValue::from_static("GLACIER"));
        assert_eq!(
            reject_unsupported_s3_copy_headers(&headers, false, "/bucket/key", "request")
                .expect_err("unsupported storage class")
                .code,
            "NotImplemented"
        );
    }

    #[test]
    fn copy_etag_conditions_distinguish_strong_and_weak_comparisons() {
        assert!(s3_copy_etag_matches("\"abc\"", "abc", false));
        assert!(!s3_copy_etag_matches("W/\"abc\"", "abc", false));
        assert!(s3_copy_etag_matches("W/\"abc\"", "abc", true));
        assert!(s3_copy_etag_matches("*", "abc", false));
    }

    #[test]
    fn copy_result_xml_escapes_etag_and_uses_the_s3_namespace() {
        let xml = copy_object_result_xml("2026-08-08T00:00:00Z", "\"a&b\"").expect("copy XML");
        assert!(
            xml.contains("<CopyObjectResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">")
        );
        assert!(xml.contains("<ETag>&quot;a&amp;b&quot;</ETag>"));
    }
}
