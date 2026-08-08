#[test]
fn object_tagging_is_classified_before_plain_object_operations() {
    assert!(
        classify_s3_object_tagging(
            &"/assets/key?tagging&versionId=v1"
                .parse()
                .expect("URI"),
            "request",
        )
        .expect("tagging classifier")
    );
    assert!(
        !classify_s3_object_tagging(&"/assets/key".parse().expect("URI"), "request")
            .expect("plain classifier")
    );
    for uri in [
        "/assets/key?tagging=unexpected",
        "/assets/key?tagging&tagging",
        "/assets/key?acl&tagging",
        "/assets/key?tagging&uploadId=upload",
    ] {
        assert_eq!(
            classify_s3_object_tagging(&uri.parse().expect("URI"), "request")
                .expect_err("ambiguous tagging")
                .code,
            "InvalidArgument"
        );
    }
}

#[test]
fn tagging_xml_requires_namespace_and_published_element_order() {
    let parsed = parse_object_tagging_xml(
        r#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet><Tag><Key>stage</Key><Value>prod</Value></Tag><Tag><Key>项目</Key><Value>万象仓</Value></Tag></TagSet></Tagging>"#
            .as_bytes(),
    )
    .expect("valid tagging XML");
    assert_eq!(parsed.len(), 2);
    let rendered = object_tagging_xml(&parsed).expect("tagging response");
    assert!(rendered.starts_with(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Tagging xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><TagSet>"
    ));
    assert!(rendered.contains("<Tag><Key>stage</Key><Value>prod</Value></Tag>"));

    for xml in [
        r#"<Tagging><TagSet /></Tagging>"#,
        r#"<Tagging xmlns="urn:wrong"><TagSet /></Tagging>"#,
        r#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet><Tag><Value>v</Value><Key>k</Key></Tag></TagSet></Tagging>"#,
        r#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet><Tag><Key>k</Key></Tag></TagSet></Tagging>"#,
    ] {
        assert_eq!(
            parse_object_tagging_xml(xml.as_bytes()),
            Err(S3TaggingXmlError::MalformedXml)
        );
    }
}

#[test]
fn tagging_xml_maps_duplicate_limits_and_control_whitespace_to_invalid_tag() {
    let duplicate = br#"<Tagging xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><TagSet><Tag><Key>same</Key><Value>1</Value></Tag><Tag><Key>same</Key><Value>2</Value></Tag></TagSet></Tagging>"#;
    assert!(matches!(
        parse_object_tagging_xml(duplicate),
        Err(S3TaggingXmlError::InvalidTag(
            mediahub_core::S3TaggingError::DuplicateKey
        ))
    ));
    let control = b"<Tagging xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><TagSet><Tag><Key>bad&#x9;key</Key><Value>v</Value></Tag></TagSet></Tagging>";
    assert!(matches!(
        parse_object_tagging_xml(control),
        Err(S3TaggingXmlError::InvalidTag(
            mediahub_core::S3TaggingError::InvalidKey
        ))
    ));
}

#[test]
fn tagging_header_uses_strict_form_decoding_and_never_silently_drops_errors() {
    let mut headers = HeaderMap::new();
    headers.insert(
        "x-amz-tagging",
        HeaderValue::from_static("project=%E4%B8%87%E8%B1%A1%E4%BB%93&symbol=%2B&space=a+b"),
    );
    let tags = parse_s3_tagging_header(&headers, "/assets/key", "request")
        .expect("header")
        .expect("tag set");
    let values = tags
        .iter()
        .map(|tag| (tag.key(), tag.value()))
        .collect::<Vec<_>>();
    assert_eq!(
        values,
        vec![
            ("project", "万象仓"),
            ("symbol", "+"),
            ("space", "a b")
        ]
    );

    for value in ["key=%", "key=%GG", "key=a=b", "=value", "key=bad%26value"] {
        headers.insert("x-amz-tagging", HeaderValue::from_str(value).expect("header"));
        assert_eq!(
            parse_s3_tagging_header(&headers, "/assets/key", "request")
                .expect_err("invalid tag header")
                .code,
            "InvalidTag"
        );
    }
}

#[test]
fn copy_tagging_directive_is_strict_and_part_copy_rejects_tagging() {
    let mut headers = HeaderMap::new();
    headers.insert("x-amz-tagging-directive", HeaderValue::from_static("REPLACE"));
    assert_eq!(
        parse_s3_tagging_directive(&headers, "/assets/copy", "request")
            .expect("directive"),
        S3TaggingDirective::Replace
    );
    assert_eq!(
        reject_unsupported_s3_copy_headers(&headers, true, "/assets/copy", "request")
            .expect_err("part tagging rejected")
            .code,
        "InvalidArgument"
    );
    headers.insert("x-amz-tagging-directive", HeaderValue::from_static("MERGE"));
    assert_eq!(
        parse_s3_tagging_directive(&headers, "/assets/copy", "request")
            .expect_err("unknown directive")
            .code,
        "InvalidArgument"
    );
}
