#[cfg(test)]
mod object_version_list_http_tests {
    use mediahub_app::{S3ObjectListItem, S3ObjectPage};
    use mediahub_core::{
        ApplicationId, BucketId, Checksum, EntityTag, ObjectId, ObjectVersion, ObjectVersionId,
        ObjectVersionState, S3VersionId, SourceProtocol, StoredObjectVersion,
    };
    use serde_json::json;

    use super::{
        ContinuationTokenCodec, ListObjectsV2Query, S3ListError, s3_list_api_error,
        s3_list_result_from_object_page,
    };

    fn object_item(
        application_id: ApplicationId,
        bucket_id: BucketId,
        key: &str,
    ) -> S3ObjectListItem {
        let payload = StoredObjectVersion::new(
            "filesystem",
            format!("objects/{key}"),
            None,
            None,
            EntityTag::new("etag<&>").expect("etag"),
            42,
            Some("image/png".to_owned()),
            json!({}),
            Some(Checksum::sha256_hex("0".repeat(64)).expect("checksum")),
        )
        .expect("payload");
        let version = ObjectVersion::new_object(
            ObjectVersionId::new(),
            ObjectId::new(),
            application_id,
            bucket_id,
            S3VersionId::new("version-1").expect("version id"),
            1,
            false,
            ObjectVersionState::Committed,
            payload,
            None,
            false,
            "test",
            SourceProtocol::S3,
            mediahub_core::OffsetDateTime::UNIX_EPOCH,
        )
        .expect("object version");
        S3ObjectListItem {
            key: key.to_owned(),
            version,
        }
    }

    fn delete_marker_item(
        application_id: ApplicationId,
        bucket_id: BucketId,
        key: &str,
    ) -> S3ObjectListItem {
        let version = ObjectVersion::new_delete_marker(
            ObjectVersionId::new(),
            ObjectId::new(),
            application_id,
            bucket_id,
            S3VersionId::new("version-delete").expect("version id"),
            1,
            false,
            "test",
            SourceProtocol::S3,
            mediahub_core::OffsetDateTime::UNIX_EPOCH,
        )
        .expect("delete marker");
        S3ObjectListItem {
            key: key.to_owned(),
            version,
        }
    }

    fn xml_element(xml: &str, name: &str) -> String {
        let start_tag = format!("<{name}>");
        let end_tag = format!("</{name}>");
        let start = xml.find(&start_tag).expect("start tag") + start_tag.len();
        let end = xml[start..].find(&end_tag).expect("end tag") + start;
        xml[start..end].to_owned()
    }

    #[test]
    fn list_handler_uses_only_object_version_repository_path() {
        let source = include_str!("s3_http_core.rs");
        let start = source
            .find("pub(super) async fn s3_list_objects")
            .expect("handler");
        let end = source[start..]
            .find("pub(super) async fn s3_get_bucket_location")
            .expect("next handler")
            + start;
        let handler = &source[start..end];
        assert!(handler.contains("find_s3_bucket"));
        assert!(handler.contains("list_current_s3_objects"));
        assert!(handler.contains("S3ObjectListQuery"));
        for legacy in [
            "find_bucket_by_name",
            "list_s3_media_page",
            "S3MediaListQuery",
            "object_store.list",
        ] {
            assert!(!handler.contains(legacy), "legacy path leaked: {legacy}");
        }
    }

    #[test]
    fn delimiter_page_and_continuation_token_share_one_window() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let query = ListObjectsV2Query::parse(Some(
            "list-type=2&prefix=photos%2F&delimiter=%2F&max-keys=2",
        ))
        .expect("query");
        let result = s3_list_result_from_object_page(
            application_id,
            bucket_id,
            "assets".to_owned(),
            query,
            S3ObjectPage {
                items: vec![object_item(
                    application_id,
                    bucket_id,
                    "photos/a.jpg",
                )],
                common_prefixes: vec!["photos/albums/".to_owned()],
                next_cursor: Some("photos/albums/".to_owned()),
            },
        );
        let codec = ContinuationTokenCodec::new([31; 32]);
        let xml = result.to_xml(&codec).expect("xml");
        assert!(xml.contains("<KeyCount>2</KeyCount>"));
        assert!(xml.contains("<Key>photos/a.jpg</Key>"));
        assert!(xml.contains("<CommonPrefixes><Prefix>photos/albums/</Prefix>"));
        assert!(xml.contains("<IsTruncated>true</IsTruncated>"));

        let token = xml_element(&xml, "NextContinuationToken");
        let second_query = ListObjectsV2Query::parse(Some(&format!(
            "list-type=2&prefix=photos%2F&delimiter=%2F&max-keys=2&continuation-token={token}"
        )))
        .expect("second query");
        assert_eq!(
            second_query
                .decode_continuation_cursor(&codec, "assets")
                .expect("decode"),
            Some("photos/albums/".to_owned())
        );
    }

    #[test]
    fn invariant_filter_hides_delete_markers_and_xml_escapes_quoted_etag() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let query = ListObjectsV2Query::parse(Some("list-type=2&max-keys=2")).expect("query");
        let result = s3_list_result_from_object_page(
            application_id,
            bucket_id,
            "assets".to_owned(),
            query,
            S3ObjectPage {
                items: vec![
                    delete_marker_item(application_id, bucket_id, "deleted.txt"),
                    object_item(application_id, bucket_id, "visible.png"),
                ],
                common_prefixes: Vec::new(),
                next_cursor: None,
            },
        );
        let xml = result
            .to_xml(&ContinuationTokenCodec::new([32; 32]))
            .expect("xml");
        assert!(!xml.contains("deleted.txt"));
        assert!(xml.contains("<KeyCount>1</KeyCount>"));
        assert!(xml.contains("<ETag>&quot;etag&lt;&amp;&gt;&quot;</ETag>"));
    }

    #[test]
    fn max_keys_zero_does_not_expose_repository_entries_or_cursor() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let query = ListObjectsV2Query::parse(Some("list-type=2&max-keys=0")).expect("query");
        let result = s3_list_result_from_object_page(
            application_id,
            bucket_id,
            "assets".to_owned(),
            query,
            S3ObjectPage {
                items: vec![object_item(application_id, bucket_id, "must-not-appear")],
                common_prefixes: vec!["must-not-appear/".to_owned()],
                next_cursor: Some("must-not-appear".to_owned()),
            },
        );
        let xml = result
            .to_xml(&ContinuationTokenCodec::new([33; 32]))
            .expect("xml");
        assert!(xml.contains("<KeyCount>0</KeyCount>"));
        assert!(xml.contains("<IsTruncated>false</IsTruncated>"));
        assert!(!xml.contains("must-not-appear"));
        assert!(!xml.contains("<NextContinuationToken>"));
    }

    #[test]
    fn list_errors_keep_s3_invalid_token_and_invalid_argument_codes() {
        let invalid_token = s3_list_api_error(
            S3ListError::InvalidContinuationToken,
            "/assets",
            "request-1",
        );
        assert_eq!(invalid_token.status, http::StatusCode::BAD_REQUEST);
        assert_eq!(invalid_token.code, "InvalidToken");

        let invalid_argument = s3_list_api_error(
            S3ListError::InvalidDelimiter,
            "/assets",
            "request-1",
        );
        assert_eq!(invalid_argument.status, http::StatusCode::BAD_REQUEST);
        assert_eq!(invalid_argument.code, "InvalidArgument");
    }}
