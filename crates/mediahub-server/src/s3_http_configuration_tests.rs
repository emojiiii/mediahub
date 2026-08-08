#[test]
fn s3_bucket_get_classifier_selects_configuration_before_authentication() {
    let request_id = "request-id";
    assert_eq!(
        classify_s3_bucket_get(
            &Uri::from_static("/s3/prismark-bucket?versioning"),
            request_id,
        )
        .expect("versioning operation"),
        S3BucketGetOperation::GetVersioning,
    );
    assert_eq!(
        classify_s3_bucket_get(
            &Uri::from_static("/s3/prismark-bucket?lifecycle=&x-id=GetBucketLifecycle"),
            request_id,
        )
        .expect("lifecycle operation"),
        S3BucketGetOperation::GetLifecycle,
    );
    assert_eq!(
        classify_s3_bucket_get(
            &Uri::from_static("/s3/prismark-bucket?location"),
            request_id,
        )
        .expect("location operation"),
        S3BucketGetOperation::GetLocation,
    );
    assert_eq!(
        classify_s3_bucket_get(
            &Uri::from_static("/s3/prismark-bucket?list-type=2&prefix=images%2F"),
            request_id,
        )
        .expect("list operation"),
        S3BucketGetOperation::ListObjects,
    );
}

#[test]
fn s3_bucket_get_classifier_rejects_ambiguous_subresources_and_list_parameters() {
    for uri in [
        "/s3/prismark-bucket?versioning&lifecycle",
        "/s3/prismark-bucket?location&versioning",
        "/s3/prismark-bucket?lifecycle&max-keys=10",
    ] {
        let error = classify_s3_bucket_get(&uri.parse().expect("valid URI"), "request-id")
            .expect_err("ambiguous operation must be rejected");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
        assert_eq!(error.code, "InvalidRequest");
    }
}

#[test]
fn s3_bucket_put_and_delete_classifiers_preserve_plain_bucket_operations() {
    let request_id = "request-id";
    assert_eq!(
        classify_s3_bucket_put(&Uri::from_static("/s3/prismark-bucket"), request_id)
            .expect("create bucket"),
        S3BucketPutOperation::CreateBucket,
    );
    assert_eq!(
        classify_s3_bucket_put(
            &Uri::from_static("/s3/prismark-bucket?versioning"),
            request_id,
        )
        .expect("put versioning"),
        S3BucketPutOperation::PutVersioning,
    );
    assert_eq!(
        classify_s3_bucket_put(
            &Uri::from_static("/s3/prismark-bucket?lifecycle"),
            request_id,
        )
        .expect("put lifecycle"),
        S3BucketPutOperation::PutLifecycle,
    );
    assert_eq!(
        classify_s3_bucket_delete(&Uri::from_static("/s3/prismark-bucket"), request_id)
            .expect("delete bucket"),
        S3BucketDeleteOperation::DeleteBucket,
    );
    assert_eq!(
        classify_s3_bucket_delete(
            &Uri::from_static("/s3/prismark-bucket?lifecycle"),
            request_id,
        )
        .expect("delete lifecycle"),
        S3BucketDeleteOperation::DeleteLifecycle,
    );
}

#[test]
fn s3_bucket_delete_versioning_is_a_standard_method_error() {
    let error = classify_s3_bucket_delete(
        &Uri::from_static("/s3/prismark-bucket?versioning"),
        "request-id",
    )
    .expect_err("versioning cannot be deleted");
    assert_eq!(error.status, StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(error.code, "MethodNotAllowed");
}

#[test]
fn s3_bucket_versioning_parser_accepts_only_enabled_or_suspended() {
    let enabled = br#"<?xml version="1.0" encoding="UTF-8"?>
        <VersioningConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
          <Status>Enabled</Status>
        </VersioningConfiguration>"#;
    assert_eq!(
        parse_s3_bucket_versioning_xml(enabled).expect("enabled versioning"),
        VersioningStatus::Enabled,
    );
    let suspended =
        br#"<VersioningConfiguration><Status>Suspended</Status></VersioningConfiguration>"#;
    assert_eq!(
        parse_s3_bucket_versioning_xml(suspended).expect("suspended versioning"),
        VersioningStatus::Suspended,
    );

    for invalid in [
        br#"<VersioningConfiguration></VersioningConfiguration>"#.as_slice(),
        br#"<VersioningConfiguration><Status>Unversioned</Status></VersioningConfiguration>"#
            .as_slice(),
        br#"<VersioningConfiguration><Status>Enabled</Status><Status>Suspended</Status></VersioningConfiguration>"#
            .as_slice(),
    ] {
        assert!(parse_s3_bucket_versioning_xml(invalid).is_err());
    }
}

#[test]
fn s3_bucket_versioning_parser_explicitly_rejects_mfa_delete() {
    let error = parse_s3_bucket_versioning_xml(
        br#"<VersioningConfiguration><Status>Enabled</Status><MFADelete>Enabled</MFADelete></VersioningConfiguration>"#,
    )
    .expect_err("MFA Delete is outside this slice");
    assert_eq!(
        error,
        S3BucketConfigurationXmlError::Unsupported("MFA Delete is not supported."),
    );
}

#[test]
fn s3_bucket_versioning_xml_omits_status_for_unversioned_bucket() {
    let unversioned = s3_bucket_versioning_xml(VersioningStatus::Unversioned);
    assert!(unversioned.contains("<VersioningConfiguration"));
    assert!(!unversioned.contains("<Status>"));
    assert!(
        s3_bucket_versioning_xml(VersioningStatus::Enabled).contains("<Status>Enabled</Status>")
    );
    assert!(
        s3_bucket_versioning_xml(VersioningStatus::Suspended)
            .contains("<Status>Suspended</Status>")
    );
}

#[test]
fn s3_lifecycle_parser_round_trips_every_supported_action() {
    let input = br#"<?xml version="1.0" encoding="UTF-8"?>
      <LifecycleConfiguration xmlns="http://s3.amazonaws.com/doc/2006-03-01/">
        <Rule>
          <ID>archive &amp; cleanup</ID>
          <Filter><Prefix>archive/&lt;raw&gt;/</Prefix></Filter>
          <Status>Enabled</Status>
          <Expiration><Days>30</Days></Expiration>
          <NoncurrentVersionExpiration><NoncurrentDays>7</NoncurrentDays></NoncurrentVersionExpiration>
          <AbortIncompleteMultipartUpload><DaysAfterInitiation>3</DaysAfterInitiation></AbortIncompleteMultipartUpload>
        </Rule>
        <Rule>
          <Filter></Filter>
          <Status>Disabled</Status>
          <Expiration><ExpiredObjectDeleteMarker>true</ExpiredObjectDeleteMarker></Expiration>
        </Rule>
      </LifecycleConfiguration>"#;
    let document = parse_s3_bucket_lifecycle_xml(input).expect("supported lifecycle subset");
    assert_eq!(document.schema_version, 1);
    assert_eq!(document.rules.len(), 2);
    assert_eq!(
        document.rules[0].filter,
        S3LifecycleFilter::Prefix {
            value: "archive/<raw>/".to_owned(),
        },
    );
    let core = s3_lifecycle_document_to_core(document.clone()).expect("domain lifecycle");
    let domain_round_trip =
        s3_lifecycle_document_from_core(&core).expect("domain lifecycle round trip");
    assert_eq!(domain_round_trip, document);    let persisted = serde_json::to_value(&document).expect("lifecycle JSON");
    let restored: S3LifecycleDocument =
        serde_json::from_value(persisted).expect("persisted lifecycle");
    restored.validate_persisted().expect("valid persisted form");
    let rendered = s3_bucket_lifecycle_xml(&restored);
    assert!(rendered.contains("archive &amp; cleanup"));
    assert!(rendered.contains("archive/&lt;raw&gt;/"));
    assert_eq!(
        parse_s3_bucket_lifecycle_xml(rendered.as_bytes()).expect("rendered lifecycle"),
        document,
    );
}

#[test]
fn s3_lifecycle_parser_accepts_midnight_utc_expiration_date() {
    let document = parse_s3_bucket_lifecycle_xml(
        br#"<LifecycleConfiguration><Rule><Filter/><Status>Enabled</Status><Expiration><Date>2030-01-02T00:00:00Z</Date></Expiration></Rule></LifecycleConfiguration>"#,
    )
    .expect("date expiration");
    assert_eq!(
        document.rules[0].expiration,
        Some(S3LifecycleExpiration::Date {
            date: "2030-01-02T00:00:00.000Z".to_owned(),
        }),
    );
}

#[test]
fn s3_lifecycle_parser_rejects_every_documented_unsupported_feature() {
    let unsupported_fragments = [
        "<Filter><Tag><Key>tier</Key><Value>cold</Value></Tag></Filter><Expiration><Days>1</Days></Expiration>",
        "<Filter><And><Prefix>a/</Prefix></And></Filter><Expiration><Days>1</Days></Expiration>",
        "<Filter><ObjectSizeGreaterThan>1</ObjectSizeGreaterThan></Filter><Expiration><Days>1</Days></Expiration>",
        "<Filter/><Transition><Days>1</Days><StorageClass>GLACIER</StorageClass></Transition>",
        "<Filter/><NoncurrentVersionTransition><NoncurrentDays>1</NoncurrentDays><StorageClass>GLACIER</StorageClass></NoncurrentVersionTransition>",
    ];
    for fragment in unsupported_fragments {
        let xml = format!(
            "<LifecycleConfiguration><Rule>{fragment}<Status>Enabled</Status></Rule></LifecycleConfiguration>"
        );
        assert!(matches!(
            parse_s3_bucket_lifecycle_xml(xml.as_bytes()),
            Err(S3BucketConfigurationXmlError::Unsupported(_)),
        ));
    }
}

#[test]
fn s3_lifecycle_parser_rejects_newer_noncurrent_versions() {
    let error = parse_s3_bucket_lifecycle_xml(
        br#"<LifecycleConfiguration><Rule><Filter/><Status>Enabled</Status><NoncurrentVersionExpiration><NoncurrentDays>30</NoncurrentDays><NewerNoncurrentVersions>2</NewerNoncurrentVersions></NoncurrentVersionExpiration></Rule></LifecycleConfiguration>"#,
    )
    .expect_err("NewerNoncurrentVersions is outside the documented subset");
    assert!(matches!(
        error,
        S3BucketConfigurationXmlError::Unsupported(_)
    ));
}

#[test]
fn s3_lifecycle_parser_rejects_invalid_or_ineffective_rules() {
    let invalid_documents = [
        "<LifecycleConfiguration></LifecycleConfiguration>",
        "<LifecycleConfiguration><Rule><Status>Enabled</Status><Expiration><Days>1</Days></Expiration></Rule></LifecycleConfiguration>",
        "<LifecycleConfiguration><Rule><Filter/><Expiration><Days>1</Days></Expiration></Rule></LifecycleConfiguration>",
        "<LifecycleConfiguration><Rule><Filter/><Status>Enabled</Status></Rule></LifecycleConfiguration>",
        "<LifecycleConfiguration><Rule><Filter/><Status>Enabled</Status><Expiration><Days>0</Days></Expiration></Rule></LifecycleConfiguration>",
        "<LifecycleConfiguration><Rule><Filter/><Status>Enabled</Status><Expiration><ExpiredObjectDeleteMarker>false</ExpiredObjectDeleteMarker></Expiration></Rule></LifecycleConfiguration>",
        "<LifecycleConfiguration><Rule><Filter/><Status>Enabled</Status><Expiration><Date>2030-01-02T08:00:00+08:00</Date></Expiration></Rule></LifecycleConfiguration>",
    ];
    for xml in invalid_documents {
        assert!(
            parse_s3_bucket_lifecycle_xml(xml.as_bytes()).is_err(),
            "{xml}"
        );
    }
}

#[test]
fn s3_lifecycle_parser_rejects_duplicate_rule_ids() {
    let error = parse_s3_bucket_lifecycle_xml(
        br#"<LifecycleConfiguration>
          <Rule><ID>same</ID><Filter/><Status>Enabled</Status><Expiration><Days>1</Days></Expiration></Rule>
          <Rule><ID>same</ID><Filter/><Status>Disabled</Status><Expiration><Days>2</Days></Expiration></Rule>
        </LifecycleConfiguration>"#,
    )
    .expect_err("duplicate IDs");
    assert!(matches!(error, S3BucketConfigurationXmlError::Invalid(_)));
}

#[test]
fn persisted_lifecycle_schema_is_strict_and_versioned() {
    let valid = serde_json::json!({
        "schema_version": 1,
        "rules": [{
            "status": "enabled",
            "filter": { "type": "empty" },
            "expiration": { "type": "days", "days": 1 }
        }]
    });
    let document: S3LifecycleDocument =
        serde_json::from_value(valid).expect("current lifecycle schema");
    document.validate_persisted().expect("valid document");

    for invalid in [
        serde_json::json!({"schema_version": 2, "rules": []}),
        serde_json::json!({"schema_version": 1, "rules": [], "unknown": true}),
    ] {
        let result = serde_json::from_value::<S3LifecycleDocument>(invalid)
            .map_err(|_| ())
            .and_then(|document| document.validate_persisted().map_err(|_| ()));
        assert!(result.is_err());
    }
}
