#[cfg(test)]
mod s3_listing_http_tests {
    use mediahub_app::{
        S3MultipartUploadListItem, S3MultipartUploadPage, S3ObjectVersionListItem,
        S3ObjectVersionPage,
    };
    use mediahub_core::{
        ApplicationId, BucketId, Checksum, EntityTag, ObjectId, ObjectVersion, ObjectVersionId,
        ObjectVersionState, OffsetDateTime, S3VersionId, SourceProtocol, StoredObjectVersion,
    };
    use serde_json::json;

    use super::{
        ListMultipartUploadsQuery, ListObjectVersionsQuery, S3BucketGetOperation,
        classify_s3_bucket_get, list_multipart_uploads_result_xml,
        list_object_versions_result_xml, parse_list_multipart_uploads_query,
        parse_list_object_versions_query, s3_multipart_uploads_result_from_page,
        s3_object_versions_result_from_page,
    };

    fn data_version(
        application_id: ApplicationId,
        bucket_id: BucketId,
        object_id: ObjectId,
    ) -> ObjectVersion {
        let payload = StoredObjectVersion::new(
            "filesystem",
            "objects/null-version",
            None,
            None,
            EntityTag::new("etag<&>").expect("etag"),
            42,
            Some("image/jpeg".to_owned()),
            json!({}),
            Some(Checksum::sha256_hex("0".repeat(64)).expect("checksum")),
        )
        .expect("payload");
        ObjectVersion::new_object(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            S3VersionId::new("null").expect("null version"),
            1,
            true,
            ObjectVersionState::Committed,
            payload,
            None,
            false,
            "test",
            SourceProtocol::S3,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("data version")
    }

    fn delete_marker(
        application_id: ApplicationId,
        bucket_id: BucketId,
        object_id: ObjectId,
    ) -> ObjectVersion {
        ObjectVersion::new_delete_marker(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            S3VersionId::new("version-delete").expect("delete marker version"),
            2,
            false,
            "test",
            SourceProtocol::S3,
            OffsetDateTime::from_unix_timestamp(1).expect("timestamp"),
        )
        .expect("delete marker")
    }

    #[test]
    fn classifier_selects_version_and_multipart_listings_before_authentication() {
        assert_eq!(
            classify_s3_bucket_get(&"/assets?versions".parse().expect("URI"), "request")
                .expect("versions"),
            S3BucketGetOperation::ListObjectVersions,
        );
        assert_eq!(
            classify_s3_bucket_get(&"/assets?uploads".parse().expect("URI"), "request")
                .expect("uploads"),
            S3BucketGetOperation::ListMultipartUploads,
        );
        for raw_uri in [
            "/assets?versions&uploads",
            "/assets?versions&upload-id-marker=u",
            "/assets?uploads&version-id-marker=v",
            "/assets?key-marker=orphan",
        ] {
            let error = classify_s3_bucket_get(&raw_uri.parse().expect("URI"), "request")
                .expect_err("invalid operation mix");
            assert_eq!(error.code, "InvalidRequest", "{raw_uri}");
        }
    }

    #[test]
    fn listing_query_parsers_enforce_markers_limits_and_url_encoding() {
        let versions_uri = "/assets?versions&prefix=%E5%9B%BE%E7%89%87%2F&delimiter=%2F&key-marker=before%20key&version-id-marker=null&max-keys=17&encoding-type=url"
            .parse()
            .expect("URI");
        assert_eq!(
            parse_list_object_versions_query(&versions_uri, "request").expect("query"),
            ListObjectVersionsQuery {
                prefix: "图片/".to_owned(),
                delimiter: Some("/".to_owned()),
                key_marker: Some("before key".to_owned()),
                version_id_marker: Some(S3VersionId::new("null").expect("version marker")),
                max_keys: 17,
                encoding_url: true,
            }
        );

        let missing_key = "/assets?versions&version-id-marker=v"
            .parse()
            .expect("URI");
        assert_eq!(
            parse_list_object_versions_query(&missing_key, "request")
                .expect_err("paired marker")
                .code,
            "InvalidArgument"
        );
        let excessive = "/assets?uploads&max-uploads=1001"
            .parse()
            .expect("URI");
        assert_eq!(
            parse_list_multipart_uploads_query(&excessive, "request")
                .expect_err("limit")
                .code,
            "InvalidArgument"
        );

        let ignored_upload_marker = "/assets?uploads&upload-id-marker=opaque"
            .parse()
            .expect("URI");
        let parsed = parse_list_multipart_uploads_query(&ignored_upload_marker, "request")
            .expect("S3 accepts and ignores it without key-marker");
        assert_eq!(parsed.key_marker, None);
        assert_eq!(parsed.upload_id_marker.as_deref(), Some("opaque"));
    }

    #[test]
    fn list_object_versions_xml_golden_covers_null_version_delete_marker_and_next_markers() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let object_id = ObjectId::new();
        let key = "图片/a b.jpg";
        let result = s3_object_versions_result_from_page(
            application_id,
            bucket_id,
            "assets".to_owned(),
            "app-owner".to_owned(),
            "Prism & Co".to_owned(),
            ListObjectVersionsQuery {
                prefix: "图片/".to_owned(),
                delimiter: Some("/".to_owned()),
                key_marker: Some("before key".to_owned()),
                version_id_marker: Some(S3VersionId::new("null").expect("marker")),
                max_keys: 3,
                encoding_url: true,
            },
            S3ObjectVersionPage {
                items: vec![
                    S3ObjectVersionListItem {
                        key: key.to_owned(),
                        version: delete_marker(application_id, bucket_id, object_id),
                        is_latest: true,
                    },
                    S3ObjectVersionListItem {
                        key: key.to_owned(),
                        version: data_version(application_id, bucket_id, object_id),
                        is_latest: false,
                    },
                ],
                common_prefixes: vec!["图片/folder/".to_owned()],
                next_key_marker: Some(key.to_owned()),
                next_version_id_marker: Some(S3VersionId::new("null").expect("next marker")),
            },
        )
        .expect("result");
        let xml = list_object_versions_result_xml(&result).expect("XML");
        let expected = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<ListVersionsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
            "<Name>assets</Name><Prefix>%E5%9B%BE%E7%89%87%2F</Prefix>",
            "<KeyMarker>before%20key</KeyMarker><VersionIdMarker>null</VersionIdMarker>",
            "<NextKeyMarker>%E5%9B%BE%E7%89%87%2Fa%20b.jpg</NextKeyMarker>",
            "<NextVersionIdMarker>null</NextVersionIdMarker><Delimiter>%2F</Delimiter>",
            "<EncodingType>url</EncodingType><MaxKeys>3</MaxKeys><IsTruncated>true</IsTruncated>",
            "<DeleteMarker><Key>%E5%9B%BE%E7%89%87%2Fa%20b.jpg</Key>",
            "<VersionId>version-delete</VersionId><IsLatest>true</IsLatest>",
            "<LastModified>1970-01-01T00:00:01Z</LastModified>",
            "<Owner><ID>app-owner</ID><DisplayName>Prism &amp; Co</DisplayName></Owner></DeleteMarker>",
            "<Version><Key>%E5%9B%BE%E7%89%87%2Fa%20b.jpg</Key><VersionId>null</VersionId>",
            "<IsLatest>false</IsLatest><LastModified>1970-01-01T00:00:00Z</LastModified>",
            "<ETag>&quot;etag&lt;&amp;&gt;&quot;</ETag><Size>42</Size><StorageClass>STANDARD</StorageClass>",
            "<Owner><ID>app-owner</ID><DisplayName>Prism &amp; Co</DisplayName></Owner></Version>",
            "<CommonPrefixes><Prefix>%E5%9B%BE%E7%89%87%2Ffolder%2F</Prefix></CommonPrefixes>",
            "</ListVersionsResult>"
        );
        assert_eq!(xml, expected);
    }

    #[test]
    fn list_multipart_uploads_xml_golden_covers_owner_encoding_and_markers() {
        let key = "视频/a b.mp4";
        let result = s3_multipart_uploads_result_from_page(
            "assets".to_owned(),
            "app-owner".to_owned(),
            "Prism <Co>".to_owned(),
            ListMultipartUploadsQuery {
                prefix: "视频/".to_owned(),
                delimiter: Some("/".to_owned()),
                key_marker: Some("before key".to_owned()),
                upload_id_marker: Some("upload-1".to_owned()),
                max_uploads: 2,
                encoding_url: true,
            },
            S3MultipartUploadPage {
                items: vec![S3MultipartUploadListItem {
                    key: key.to_owned(),
                    upload_id: "upload<&>".to_owned(),
                    initiated_at: OffsetDateTime::UNIX_EPOCH,
                }],
                common_prefixes: vec!["视频/folder/".to_owned()],
                next_key_marker: Some(key.to_owned()),
                next_upload_id_marker: Some("upload-2".to_owned()),
            },
        );
        let xml = list_multipart_uploads_result_xml(&result).expect("XML");
        let expected = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<ListMultipartUploadsResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">",
            "<Bucket>assets</Bucket><KeyMarker>before%20key</KeyMarker>",
            "<UploadIdMarker>upload-1</UploadIdMarker>",
            "<NextKeyMarker>%E8%A7%86%E9%A2%91%2Fa%20b.mp4</NextKeyMarker>",
            "<NextUploadIdMarker>upload-2</NextUploadIdMarker><Delimiter>%2F</Delimiter>",
            "<Prefix>%E8%A7%86%E9%A2%91%2F</Prefix><EncodingType>url</EncodingType>",
            "<MaxUploads>2</MaxUploads><IsTruncated>true</IsTruncated>",
            "<Upload><Key>%E8%A7%86%E9%A2%91%2Fa%20b.mp4</Key><UploadId>upload&lt;&amp;&gt;</UploadId>",
            "<Initiator><ID>app-owner</ID><DisplayName>Prism &lt;Co&gt;</DisplayName></Initiator>",
            "<Owner><ID>app-owner</ID><DisplayName>Prism &lt;Co&gt;</DisplayName></Owner>",
            "<StorageClass>STANDARD</StorageClass><Initiated>1970-01-01T00:00:00Z</Initiated></Upload>",
            "<CommonPrefixes><Prefix>%E8%A7%86%E9%A2%91%2Ffolder%2F</Prefix></CommonPrefixes>",
            "</ListMultipartUploadsResult>"
        );
        assert_eq!(xml, expected);
    }

    #[test]
    fn listing_handlers_do_not_use_media_or_scan_object_storage() {
        let source = include_str!("s3_http_core.rs");
        let start = source
            .find("pub(super) async fn s3_list_object_versions")
            .expect("versions handler");
        let end = source[start..]
            .find("pub(super) async fn s3_get_bucket_location")
            .expect("listing handler boundary")
            + start;
        let handlers = &source[start..end];
        assert!(handlers.contains("list_s3_object_versions_page"));
        assert!(handlers.contains("list_s3_multipart_uploads_page"));
        for forbidden in ["S3Media", "list_s3_media", "object_store.list", "MediaRepository"] {
            assert!(!handlers.contains(forbidden), "legacy path leaked: {forbidden}");
        }
    }
}
