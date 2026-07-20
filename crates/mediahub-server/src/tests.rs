use base64::engine::general_purpose::STANDARD;

use super::s3_http::{S3ApiError, s3_object_names};
use super::*;

#[test]
fn media_directory_cursor_round_trips_and_isolated_from_flat_cursor() {
    let cursor = MediaDirectoryListCursor {
        entry_key: "图片/头像/".to_owned(),
        is_prefix: true,
    };
    let encoded = encode_media_directory_cursor(cursor.clone());
    assert_eq!(
        decode_media_directory_cursor(&encoded).expect("directory cursor"),
        cursor
    );
    assert!(decode_media_cursor(&encoded).is_err());
    assert!(decode_media_directory_cursor("invalid").is_err());
}

#[test]
fn media_directory_delimiter_accepts_only_a_single_slash() {
    assert!(!parse_list_delimiter(None).expect("flat list"));
    assert!(parse_list_delimiter(Some("/")).expect("directory list"));
    assert!(parse_list_delimiter(Some("")).is_err());
    assert!(parse_list_delimiter(Some("\\")).is_err());
}

async fn auth_test_state(pool: sqlx::PgPool, registration_enabled: bool) -> Arc<AppState> {
    let repository = PostgresRepository::new(pool);
    let storage_root = std::env::temp_dir().join(format!(
        "mediahub-auth-test-{}",
        uuid::Uuid::now_v7().simple()
    ));
    let object_store =
        RuntimeObjectStore::local(LocalObjectStore::new(&storage_root).expect("object store"));
    let access_key_cipher = Arc::new(
        AccessKeyCipher::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", 1)
            .expect("access key cipher"),
    );
    let webdav = webdav::WebDavService::new(
        repository.clone(),
        object_store.clone(),
        Arc::clone(&access_key_cipher),
    );
    Arc::new(AppState {
        repository,
        object_store,
        webdav,
        access_key_cipher,
        media_url_signer: Arc::new(MediaUrlSigner::new(vec![7; 32])),
        cookie_config: CookieConfig {
            secure: false,
            same_site: "Lax",
        },
        cors_allowed_origins: Vec::new(),
        registration_enabled,
        expose_auth_tokens: true,
        email_provider: None,
        auth_rate_limiter: AuthRateLimiter::default(),
        variant_slots: Arc::new(tokio::sync::Semaphore::new(1)),
        http_metrics: HttpMetrics::default(),
        metrics_bearer_token: None,
    })
}

async fn authenticated_test_user(
    state: &AppState,
    email: &str,
    system_role: &str,
) -> (UserId, HeaderMap) {
    let now = OffsetDateTime::now_utc();
    let user_id = UserId::new();
    state
        .repository
        .create_user(user_id, email, "hashed", now)
        .await
        .expect("user");
    state
        .repository
        .create_application(
            ApplicationId::new(),
            user_id,
            "Default",
            &format!("app_{}", user_id.as_uuid().simple()),
            1024,
            now,
        )
        .await
        .expect("application");
    let verification_hash = token_hash(&format!("verify-{user_id}"));
    state
        .repository
        .create_one_time_token(
            user_id,
            OneTimeTokenPurpose::VerifyEmail,
            &verification_hash,
            now + time::Duration::minutes(5),
            now,
        )
        .await
        .expect("verification token");
    assert!(
        state
            .repository
            .consume_email_verification_token(&verification_hash, now)
            .await
            .expect("verify user")
    );
    match system_role {
        "user" => {}
        "admin" => assert_eq!(
            state
                .repository
                .bootstrap_admin(email, now)
                .await
                .expect("bootstrap admin"),
            AdminBootstrapOutcome::Completed(user_id)
        ),
        value => panic!("unsupported test system role {value}"),
    }
    let session_token = format!("session-{}", user_id.as_uuid().simple());
    let csrf_token = format!("csrf-{}", user_id.as_uuid().simple());
    state
        .repository
        .create_session(
            user_id,
            &token_hash(&session_token),
            &token_hash(&csrf_token),
            now + time::Duration::hours(1),
            now,
        )
        .await
        .expect("session");
    let mut headers = HeaderMap::new();
    headers.insert(
        "cookie",
        HeaderValue::from_str(&format!(
            "{SESSION_COOKIE}={session_token}; {CSRF_COOKIE}={csrf_token}"
        ))
        .expect("cookie header"),
    );
    headers.insert(
        "x-csrf-token",
        HeaderValue::from_str(&csrf_token).expect("csrf header"),
    );
    (user_id, headers)
}

#[tokio::test]
async fn download_body_is_chunked_and_uses_cumulative_pacing() {
    assert_eq!(
        download_target_elapsed(64 * 1024, 1024 * 1024),
        StdDuration::from_millis(62) + StdDuration::from_micros(500)
    );
    assert_eq!(
        download_target_elapsed(3 * 1024 * 1024 / 2, 1024 * 1024),
        StdDuration::from_millis(1500)
    );

    let expected_length = DOWNLOAD_BODY_CHUNK_BYTES * 2 + 17;
    let body = download_limited_body(Body::from(vec![7_u8; expected_length]), None);
    let mut stream = body.into_data_stream();
    let mut chunks = 0;
    let mut received = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.expect("download chunk");
        assert!(chunk.len() <= DOWNLOAD_BODY_CHUNK_BYTES);
        assert!(chunk.iter().all(|byte| *byte == 7));
        chunks += 1;
        received += chunk.len();
    }
    assert_eq!(chunks, 3);
    assert_eq!(received, expected_length);
}

async fn upload_test_media(
    state: &AppState,
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: &str,
    content: &[u8],
    visibility_override: Option<Visibility>,
) -> Media {
    upload_test_media_with_mime(
        state,
        application_id,
        bucket_id,
        object_key,
        content,
        "application/octet-stream",
        visibility_override,
    )
    .await
}

async fn upload_test_media_with_mime(
    state: &AppState,
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: &str,
    content: &[u8],
    mime: &str,
    visibility_override: Option<Visibility>,
) -> Media {
    UploadMediaService::new(
        state.object_store.clone(),
        state.repository.clone(),
        state.repository.clone(),
        SystemClock,
    )
    .upload(&UploadMediaRequest {
        application_id,
        bucket_id,
        object_key: object_key.into(),
        original_name: object_key.rsplit('/').next().map(str::to_owned),
        display_name: object_key.into(),
        extension: None,
        mime: mime.into(),
        content: content.to_vec(),
        visibility_override,
        expire_at: None,
        metadata: ClientMetadata::default(),
    })
    .await
    .expect("test media upload")
    .media
}

fn sign_s3_test_request(
    request: &mut http::Request<Vec<u8>>,
    access_key_id: &str,
    secret: &str,
    presign_expiry: Option<StdDuration>,
) {
    if presign_expiry.is_none() && !request.headers().contains_key("x-amz-content-sha256") {
        request.headers_mut().insert(
            http::header::HeaderName::from_static("x-amz-content-sha256"),
            http::header::HeaderValue::from_static("UNSIGNED-PAYLOAD"),
        );
    }
    let identity = aws_credential_types::Credentials::new(
        access_key_id,
        secret,
        None,
        None,
        "mediahub-s3-gateway-test",
    )
    .into();
    let mut settings = aws_sigv4::http_request::SigningSettings::default();
    settings.signature_location = if presign_expiry.is_some() {
        aws_sigv4::http_request::SignatureLocation::QueryParams
    } else {
        aws_sigv4::http_request::SignatureLocation::Headers
    };
    settings.expires_in = presign_expiry;
    settings.percent_encoding_mode = aws_sigv4::http_request::PercentEncodingMode::Single;
    settings.uri_path_normalization_mode =
        aws_sigv4::http_request::UriPathNormalizationMode::Disabled;
    settings.payload_checksum_kind = aws_sigv4::http_request::PayloadChecksumKind::NoHeader;
    let params = aws_sigv4::sign::v4::SigningParams::builder()
        .identity(&identity)
        .region("us-east-1")
        .name("s3")
        .time(std::time::SystemTime::now())
        .settings(settings)
        .build()
        .expect("S3 test signing params")
        .into();
    let signable = aws_sigv4::http_request::SignableRequest::new(
        request.method().as_str(),
        request.uri().to_string(),
        request.headers().iter().map(|(name, value)| {
            (
                name.as_str(),
                value.to_str().expect("S3 test request header"),
            )
        }),
        aws_sigv4::http_request::SignableBody::UnsignedPayload,
    )
    .expect("S3 test signable request");
    aws_sigv4::http_request::sign(signable, &params)
        .expect("S3 test signature")
        .into_parts()
        .0
        .apply_to_request_http1x(request);
}

async fn send_s3_test_request(
    client: &reqwest::Client,
    request: http::Request<Vec<u8>>,
) -> reqwest::Response {
    let (parts, body) = request.into_parts();
    client
        .request(parts.method, parts.uri.to_string())
        .headers(parts.headers)
        .body(body)
        .send()
        .await
        .expect("S3 test request")
}

fn s3_test_xml_value(xml: &str, element: &str) -> Option<String> {
    let start = format!("<{element}>");
    let end = format!("</{element}>");
    let value = xml.split_once(&start)?.1.split_once(&end)?.0;
    Some(
        value
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&amp;", "&"),
    )
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn s3_gateway_persists_media_and_serves_presigned_get_and_head(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (user_id, _) = authenticated_test_user(&state, "s3-gateway@example.com", "user").await;
    let application = state
        .repository
        .default_application_for_user(user_id)
        .await
        .expect("application lookup")
        .expect("application");
    state
        .repository
        .update_application_quota(
            user_id,
            application.id,
            16 * 1024 * 1024,
            "req_s3_gateway_test_quota",
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("raise S3 gateway test quota");
    let bucket = Bucket::new(
        BucketId::new(),
        application.id,
        "generated-images",
        BucketPolicy::unrestricted(Visibility::Private),
        OffsetDateTime::now_utc(),
    )
    .expect("bucket");
    state
        .repository
        .create_bucket(&bucket)
        .await
        .expect("persist bucket");
    let access_key_id = "mh_ak_sub2api_test";
    let access_key_secret = "sub2api-test-secret";
    state
        .repository
        .create_access_key(&NewAccessKey {
            id: uuid::Uuid::now_v7().to_string(),
            application_id: application.id,
            access_key_id: access_key_id.into(),
            secret_ciphertext: state
                .access_key_cipher
                .encrypt(access_key_secret.as_bytes())
                .expect("encrypt access key"),
            secret_key_version: state.access_key_cipher.version(),
            secret_last_four: "cret".into(),
            name: "sub2api S3".into(),
            permissions: vec![
                "media:upload".into(),
                "media:read".into(),
                "media:list".into(),
                "media:update".into(),
                "media:delete".into(),
            ],
            expires_at: None,
            created_at: OffsetDateTime::now_utc(),
        })
        .await
        .expect("persist access key");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn({
        let application = router((*state).clone());
        async move {
            axum::serve(listener, application)
                .await
                .expect("S3 gateway test server");
        }
    });
    let client = reqwest::Client::new();
    let object_key = "images/imgtask_test-0.png";
    let url = format!("http://{address}/s3/{}/{}", bucket.name(), object_key);
    let content = b"generated-image-png".to_vec();

    let mut put = http::Request::builder()
        .method(Method::PUT)
        .uri(&url)
        .header("host", address.to_string())
        .header(CONTENT_TYPE, "image/png")
        .header("x-amz-content-sha256", "UNSIGNED-PAYLOAD")
        .body(content.clone())
        .expect("PUT request");
    sign_s3_test_request(&mut put, access_key_id, access_key_secret, None);
    let put_response = send_s3_test_request(&client, put).await;
    assert_eq!(put_response.status(), StatusCode::OK);
    assert!(put_response.headers().contains_key(ETAG));
    assert!(put_response.headers().contains_key("x-amz-request-id"));

    let media = state
        .repository
        .find_by_object_key(application.id, bucket.id(), object_key)
        .await
        .expect("media lookup")
        .expect("MediaHub media record");
    assert_eq!(media.mime(), "image/png");
    assert_eq!(media.size(), content.len() as u64);

    let mut get = http::Request::builder()
        .method(Method::GET)
        .uri(format!("{url}?x-id=GetObject"))
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("GET request");
    sign_s3_test_request(
        &mut get,
        access_key_id,
        access_key_secret,
        Some(StdDuration::from_secs(24 * 60 * 60)),
    );
    let get_response = send_s3_test_request(&client, get).await;
    assert_eq!(get_response.status(), StatusCode::OK);
    assert_eq!(get_response.headers()[CONTENT_TYPE], "image/png");
    assert_eq!(
        get_response.bytes().await.expect("GET body").as_ref(),
        content
    );

    let mut versioned_get = http::Request::builder()
        .method(Method::GET)
        .uri(format!("{url}?versionId=1"))
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("versioned GET request");
    sign_s3_test_request(&mut versioned_get, access_key_id, access_key_secret, None);
    let versioned_get = send_s3_test_request(&client, versioned_get).await;
    assert_eq!(versioned_get.status(), StatusCode::NOT_IMPLEMENTED);
    assert!(
        versioned_get
            .text()
            .await
            .expect("Versioning error XML")
            .contains("<Code>NotImplemented</Code>")
    );

    let mut head = http::Request::builder()
        .method(Method::HEAD)
        .uri(&url)
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("HEAD request");
    sign_s3_test_request(
        &mut head,
        access_key_id,
        access_key_secret,
        Some(StdDuration::from_secs(24 * 60 * 60)),
    );
    let head_response = send_s3_test_request(&client, head).await;
    assert_eq!(head_response.status(), StatusCode::OK);
    assert_eq!(
        head_response.headers()[CONTENT_LENGTH],
        content.len().to_string()
    );
    assert!(head_response.bytes().await.expect("HEAD body").is_empty());

    let mut put_acl = http::Request::builder()
        .method(Method::PUT)
        .uri(format!("{url}?acl"))
        .header("host", address.to_string())
        .header("x-amz-acl", "public-read")
        .body(Vec::new())
        .expect("PutObjectAcl request");
    sign_s3_test_request(&mut put_acl, access_key_id, access_key_secret, None);
    let put_acl = send_s3_test_request(&client, put_acl).await;
    let put_acl_status = put_acl.status();
    let put_acl_body = put_acl.text().await.expect("PutObjectAcl response");
    assert_eq!(
        put_acl_status,
        StatusCode::OK,
        "PutObjectAcl response: {put_acl_body}"
    );

    let mut get_acl = http::Request::builder()
        .method(Method::GET)
        .uri(format!("{url}?acl"))
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("GetObjectAcl request");
    sign_s3_test_request(&mut get_acl, access_key_id, access_key_secret, None);
    let get_acl = send_s3_test_request(&client, get_acl).await;
    assert_eq!(get_acl.status(), StatusCode::OK);
    let acl_xml = get_acl.text().await.expect("ACL XML");
    assert!(acl_xml.contains("groups/global/AllUsers"));
    assert!(acl_xml.contains("<Permission>READ</Permission>"));

    let mut unsupported_acl = http::Request::builder()
        .method(Method::PUT)
        .uri(format!("{url}?acl"))
        .header("host", address.to_string())
        .header("x-amz-acl", "authenticated-read")
        .body(Vec::new())
        .expect("unsupported ACL request");
    sign_s3_test_request(&mut unsupported_acl, access_key_id, access_key_secret, None);
    let unsupported_acl = send_s3_test_request(&client, unsupported_acl).await;
    assert_eq!(unsupported_acl.status(), StatusCode::BAD_REQUEST);
    assert!(
        unsupported_acl
            .text()
            .await
            .expect("ACL error XML")
            .contains("<Code>AccessControlListNotSupported</Code>")
    );

    let multipart_key = "images/multipart-result.bin";
    let multipart_url = format!("http://{address}/s3/{}/{}", bucket.name(), multipart_key);
    let mut create_multipart = http::Request::builder()
        .method(Method::POST)
        .uri(format!("{multipart_url}?uploads"))
        .header("host", address.to_string())
        .header(CONTENT_TYPE, "application/octet-stream")
        .header("x-amz-acl", "private")
        .body(Vec::new())
        .expect("CreateMultipartUpload request");
    sign_s3_test_request(
        &mut create_multipart,
        access_key_id,
        access_key_secret,
        None,
    );
    let create_multipart = send_s3_test_request(&client, create_multipart).await;
    assert_eq!(create_multipart.status(), StatusCode::OK);
    let create_xml = create_multipart.text().await.expect("create multipart XML");
    let upload_id = s3_test_xml_value(&create_xml, "UploadId").expect("UploadId");

    let first_part = vec![b'a'; 5 * 1024 * 1024];
    let second_part = b"multipart-tail".to_vec();
    let mut part_etags = Vec::new();
    for (part_number, part) in [(1_u16, &first_part), (2_u16, &second_part)] {
        let mut upload_part = http::Request::builder()
            .method(Method::PUT)
            .uri(format!(
                "{multipart_url}?partNumber={part_number}&uploadId={upload_id}"
            ))
            .header("host", address.to_string())
            .body(part.clone())
            .expect("UploadPart request");
        sign_s3_test_request(&mut upload_part, access_key_id, access_key_secret, None);
        let upload_part = send_s3_test_request(&client, upload_part).await;
        assert_eq!(upload_part.status(), StatusCode::OK);
        part_etags.push(
            upload_part
                .headers()
                .get(ETAG)
                .expect("part ETag")
                .to_str()
                .expect("part ETag text")
                .to_owned(),
        );
    }

    let mut list_parts = http::Request::builder()
        .method(Method::GET)
        .uri(format!("{multipart_url}?uploadId={upload_id}&max-parts=1"))
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("ListParts request");
    sign_s3_test_request(&mut list_parts, access_key_id, access_key_secret, None);
    let list_parts = send_s3_test_request(&client, list_parts).await;
    assert_eq!(list_parts.status(), StatusCode::OK);
    let list_parts_xml = list_parts.text().await.expect("ListParts XML");
    assert!(list_parts_xml.contains("<IsTruncated>true</IsTruncated>"));
    assert!(list_parts_xml.contains("<NextPartNumberMarker>1</NextPartNumberMarker>"));

    let complete_body = format!(
            "<CompleteMultipartUpload><Part><PartNumber>1</PartNumber><ETag>{}</ETag></Part><Part><PartNumber>2</PartNumber><ETag>{}</ETag></Part></CompleteMultipartUpload>",
            part_etags[0], part_etags[1]
        )
        .into_bytes();
    let mut complete_multipart = http::Request::builder()
        .method(Method::POST)
        .uri(format!("{multipart_url}?uploadId={upload_id}"))
        .header("host", address.to_string())
        .header(CONTENT_TYPE, "application/xml")
        .body(complete_body)
        .expect("CompleteMultipartUpload request");
    sign_s3_test_request(
        &mut complete_multipart,
        access_key_id,
        access_key_secret,
        None,
    );
    let complete_multipart = send_s3_test_request(&client, complete_multipart).await;
    let complete_status = complete_multipart.status();
    let complete_xml = complete_multipart.text().await.expect("complete XML");
    assert_eq!(
        complete_status,
        StatusCode::OK,
        "CompleteMultipartUpload response: {complete_xml}"
    );
    assert!(complete_xml.contains("<CompleteMultipartUploadResult"));
    assert!(complete_xml.contains("<ETag>&quot;"));

    let multipart_media = state
        .repository
        .find_by_object_key(application.id, bucket.id(), multipart_key)
        .await
        .expect("multipart media lookup")
        .expect("multipart Media");
    assert_eq!(
        multipart_media.size(),
        (first_part.len() + second_part.len()) as u64
    );
    assert_eq!(
        multipart_media.visibility_override(),
        Some(Visibility::Private)
    );
    let mut expected_multipart = first_part.clone();
    expected_multipart.extend_from_slice(&second_part);
    assert_eq!(
        state
            .object_store
            .read(multipart_media.storage_key())
            .await
            .expect("read composed multipart"),
        expected_multipart
    );

    let aborted_key = "images/aborted-multipart.bin";
    let aborted_url = format!("http://{address}/s3/{}/{}", bucket.name(), aborted_key);
    let mut create_aborted = http::Request::builder()
        .method(Method::POST)
        .uri(format!("{aborted_url}?uploads"))
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("CreateMultipartUpload for abort request");
    sign_s3_test_request(&mut create_aborted, access_key_id, access_key_secret, None);
    let create_aborted = send_s3_test_request(&client, create_aborted).await;
    assert_eq!(create_aborted.status(), StatusCode::OK);
    let aborted_xml = create_aborted.text().await.expect("abort UploadId XML");
    let aborted_upload_id = s3_test_xml_value(&aborted_xml, "UploadId").expect("abort UploadId");
    let mut aborted_part = http::Request::builder()
        .method(Method::PUT)
        .uri(format!(
            "{aborted_url}?partNumber=1&uploadId={aborted_upload_id}"
        ))
        .header("host", address.to_string())
        .body(b"discarded-part".to_vec())
        .expect("UploadPart before abort request");
    sign_s3_test_request(&mut aborted_part, access_key_id, access_key_secret, None);
    assert_eq!(
        send_s3_test_request(&client, aborted_part).await.status(),
        StatusCode::OK
    );
    let mut abort = http::Request::builder()
        .method(Method::DELETE)
        .uri(format!("{aborted_url}?uploadId={aborted_upload_id}"))
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("AbortMultipartUpload request");
    sign_s3_test_request(&mut abort, access_key_id, access_key_secret, None);
    assert_eq!(
        send_s3_test_request(&client, abort).await.status(),
        StatusCode::NO_CONTENT
    );
    assert!(
        state
            .repository
            .list_multipart_parts(&aborted_upload_id)
            .await
            .expect("aborted multipart metadata")
            .is_empty()
    );
    assert!(
        state
            .object_store
            .list(
                &s3_multipart_storage::multipart_upload_prefix(&aborted_upload_id),
                None,
                1_000,
            )
            .await
            .expect("aborted multipart storage prefix")
            .objects
            .is_empty()
    );

    let bucket_url = format!("http://{address}/s3/{}", bucket.name());
    let mut first_list = http::Request::builder()
        .method(Method::GET)
        .uri(format!(
            "{bucket_url}?list-type=2&prefix=images%2F&max-keys=1"
        ))
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("ListObjectsV2 request");
    sign_s3_test_request(&mut first_list, access_key_id, access_key_secret, None);
    let first_list = send_s3_test_request(&client, first_list).await;
    assert_eq!(first_list.status(), StatusCode::OK);
    let first_list_xml = first_list.text().await.expect("first list XML");
    assert!(first_list_xml.contains("<KeyCount>1</KeyCount>"));
    assert!(first_list_xml.contains("<IsTruncated>true</IsTruncated>"));
    let continuation =
        s3_test_xml_value(&first_list_xml, "NextContinuationToken").expect("continuation token");
    let mut second_list = http::Request::builder()
        .method(Method::GET)
        .uri(format!(
            "{bucket_url}?list-type=2&prefix=images%2F&max-keys=1&continuation-token={continuation}"
        ))
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("continued ListObjectsV2 request");
    sign_s3_test_request(&mut second_list, access_key_id, access_key_secret, None);
    let second_list = send_s3_test_request(&client, second_list).await;
    assert_eq!(second_list.status(), StatusCode::OK);
    let second_list_xml = second_list.text().await.expect("second list XML");
    assert!(second_list_xml.contains(multipart_key));
    assert!(second_list_xml.contains("<IsTruncated>false</IsTruncated>"));

    let mut delete_object = http::Request::builder()
        .method(Method::DELETE)
        .uri(&url)
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("DeleteObject request");
    sign_s3_test_request(&mut delete_object, access_key_id, access_key_secret, None);
    assert_eq!(
        send_s3_test_request(&client, delete_object).await.status(),
        StatusCode::NO_CONTENT
    );
    let mut repeat_delete = http::Request::builder()
        .method(Method::DELETE)
        .uri(&url)
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("repeated DeleteObject request");
    sign_s3_test_request(&mut repeat_delete, access_key_id, access_key_secret, None);
    assert_eq!(
        send_s3_test_request(&client, repeat_delete).await.status(),
        StatusCode::NO_CONTENT
    );

    let delete_body = format!(
            "<Delete><Object><Key>{multipart_key}</Key></Object><Object><Key>images/missing.png</Key></Object></Delete>"
        )
        .into_bytes();
    let mut missing_delete_md5 = http::Request::builder()
        .method(Method::POST)
        .uri(format!("{bucket_url}?delete"))
        .header("host", address.to_string())
        .header(CONTENT_TYPE, "application/xml")
        .body(delete_body.clone())
        .expect("DeleteObjects request without Content-MD5");
    sign_s3_test_request(
        &mut missing_delete_md5,
        access_key_id,
        access_key_secret,
        None,
    );
    let missing_delete_md5 = send_s3_test_request(&client, missing_delete_md5).await;
    assert_eq!(missing_delete_md5.status(), StatusCode::BAD_REQUEST);
    assert!(
        missing_delete_md5
            .text()
            .await
            .expect("Content-MD5 error XML")
            .contains("<Code>InvalidDigest</Code>")
    );
    let delete_content_md5 = STANDARD.encode(md5::Md5::digest(&delete_body));
    let mut delete_objects = http::Request::builder()
        .method(Method::POST)
        .uri(format!("{bucket_url}?delete"))
        .header("host", address.to_string())
        .header(CONTENT_TYPE, "application/xml")
        .header("content-md5", delete_content_md5)
        .body(delete_body)
        .expect("DeleteObjects request");
    sign_s3_test_request(&mut delete_objects, access_key_id, access_key_secret, None);
    let delete_objects = send_s3_test_request(&client, delete_objects).await;
    assert_eq!(delete_objects.status(), StatusCode::OK);
    let delete_xml = delete_objects.text().await.expect("DeleteResult XML");
    assert!(delete_xml.contains(multipart_key));
    assert!(delete_xml.contains("images/missing.png"));

    let mut bad_get = http::Request::builder()
        .method(Method::GET)
        .uri(&url)
        .header("host", address.to_string())
        .body(Vec::new())
        .expect("invalid GET request");
    sign_s3_test_request(
        &mut bad_get,
        access_key_id,
        "wrong-secret",
        Some(StdDuration::from_secs(60)),
    );
    let bad_response = send_s3_test_request(&client, bad_get).await;
    assert_eq!(bad_response.status(), StatusCode::FORBIDDEN);
    assert_eq!(bad_response.headers()[CONTENT_TYPE], "application/xml");
    let error = bad_response.text().await.expect("S3 error body");
    assert!(error.contains("<Code>SignatureDoesNotMatch</Code>"));

    let audit = state
        .repository
        .list_audit(application.id, 20)
        .await
        .expect("S3 audit");
    assert!(audit.iter().any(|event| {
        event.action == "media.uploaded"
            && event.actor_id == access_key_id
            && event.summary["protocol"] == "s3"
    }));

    server.abort();
    let _ = server.await;
    drop(client);
    drop(state);
    std::fs::remove_dir_all(storage_root).expect("remove S3 gateway test storage");
}

#[test]
fn api_timestamps_are_human_readable_strings() {
    assert!(
        serde_json::to_value(OffsetDateTime::UNIX_EPOCH)
            .expect("timestamp serialization")
            .is_string()
    );
}

#[test]
fn cors_allows_application_context_and_security_headers() {
    let headers = cors_allowed_headers();
    for expected in [
        "authorization",
        "if-match",
        "idempotency-key",
        "x-csrf-token",
        "x-mediahub-access-key",
        "x-mediahub-app-id",
        "x-mediahub-content-sha256",
        "x-mediahub-date",
        "x-mediahub-nonce",
    ] {
        assert!(
            headers.iter().any(|header| header.as_str() == expected),
            "missing CORS request header {expected}"
        );
    }
}

#[test]
fn upload_size_validation_enforces_two_gib_limit() {
    assert!(validate_upload_expected_size(MAX_UPLOAD_OBJECT_BYTES).is_ok());
    let oversized = MAX_UPLOAD_OBJECT_BYTES + 1;
    assert_eq!(
        validate_upload_expected_size(oversized)
            .expect_err("object limit")
            .status,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let s3_error = S3ApiError::from_api(
        validate_upload_expected_size(oversized).expect_err("S3 object limit"),
        "/s3/media/oversized.bin",
        "request-id",
    );
    assert_eq!(s3_error.status, StatusCode::BAD_REQUEST);
    assert_eq!(s3_error.code, "EntityTooLarge");
    assert_eq!(
        validate_upload_expected_size(0)
            .expect_err("empty uploads are invalid")
            .status,
        StatusCode::BAD_REQUEST
    );
}

#[test]
fn s3_multipart_object_key_requires_a_final_name() {
    assert_eq!(
        s3_object_names(
            "images/final.png",
            "/s3/media/images/final.png",
            "request-id"
        )
        .expect("valid S3 object key"),
        ("final.png".to_owned(), Some("png".to_owned()))
    );
    let error = s3_object_names("images/", "/s3/media/images/", "request-id")
        .expect_err("directory marker cannot become a Media object");
    assert_eq!(error.status, StatusCode::BAD_REQUEST);
    assert_eq!(error.code, "InvalidArgument");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn storage_consistency_rejects_a_populated_database_with_an_empty_root(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    validate_storage_database_consistency(&state.repository, &state.object_store)
        .await
        .expect("an empty database accepts an empty storage root");

    let (user_id, _) = authenticated_test_user(&state, "storage-check@example.com", "user").await;
    let application = state
        .repository
        .default_application_for_user(user_id)
        .await
        .expect("application lookup")
        .expect("default application");
    let bucket = Bucket::new(
        BucketId::new(),
        application.id,
        "storage-check",
        BucketPolicy::unrestricted(Visibility::Private),
        OffsetDateTime::now_utc(),
    )
    .expect("bucket");
    state
        .repository
        .create_bucket(&bucket)
        .await
        .expect("persist bucket");
    upload_test_media(
        &state,
        application.id,
        bucket.id(),
        "protected.bin",
        b"protected",
        None,
    )
    .await;
    validate_storage_database_consistency(&state.repository, &state.object_store)
        .await
        .expect("the populated matching root remains healthy");

    let mismatched_root = std::env::temp_dir().join(format!(
        "mediahub-mismatched-storage-test-{}",
        uuid::Uuid::now_v7().simple()
    ));
    let mismatched_store = RuntimeObjectStore::local(
        LocalObjectStore::new(&mismatched_root).expect("mismatched object store"),
    );
    let mismatch = validate_storage_database_consistency(&state.repository, &mismatched_store)
        .await
        .expect_err("a populated database must reject an empty storage root");
    assert!(mismatch.contains("none of 1 sampled objects exists"));

    let mut mismatched_state = (*state).clone();
    mismatched_state.object_store = mismatched_store;
    let ready_error = readiness(State(Arc::new(mismatched_state)))
        .await
        .err()
        .expect("mismatched readiness");
    assert_eq!(ready_error.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(ready_error.code, "unavailable");

    std::fs::remove_dir_all(storage_root).expect("remove matching object store");
    std::fs::remove_dir_all(mismatched_root).expect("remove mismatched object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn admin_endpoints_distinguish_anonymous_and_non_admin_users(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let anonymous = admin_list_users(
        State(Arc::clone(&state)),
        HeaderMap::new(),
        Query(AdminListQuery::default()),
    )
    .await
    .expect_err("anonymous admin request");
    assert_eq!(anonymous.status, StatusCode::UNAUTHORIZED);

    let (user_id, headers) = authenticated_test_user(&state, "user@example.com", "user").await;
    let forbidden = admin_list_users(
        State(Arc::clone(&state)),
        headers.clone(),
        Query(AdminListQuery::default()),
    )
    .await
    .expect_err("non-admin request");
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    let forbidden_patch = admin_update_user_status(
        State(Arc::clone(&state)),
        headers.clone(),
        Extension(RequestId("req_admin_test".into())),
        Path(user_id.to_string()),
        Json(AdminUpdateUserStatusRequest {
            status: "suspended".into(),
        }),
    )
    .await
    .expect_err("non-admin status update");
    assert_eq!(forbidden_patch.status, StatusCode::FORBIDDEN);
    let forbidden_quota_patch = admin_update_application_quota(
        State(Arc::clone(&state)),
        headers,
        Extension(RequestId("req_admin_quota_test".into())),
        Path(ApplicationId::new().to_string()),
        Json(AdminUpdateApplicationQuotaRequest { quota_bytes: 2048 }),
    )
    .await
    .err()
    .expect("non-admin quota update");
    assert_eq!(forbidden_quota_patch.status, StatusCode::FORBIDDEN);
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn admin_settings_are_nullable_bounded_and_csrf_protected(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (_, admin_headers) =
        authenticated_test_user(&state, "settings-admin@example.com", "admin").await;

    let initial = admin_settings(State(Arc::clone(&state)), admin_headers.clone())
        .await
        .expect("initial settings");
    assert_eq!(
        initial.0.download_bytes_per_second,
        Some(mediahub_app::DEFAULT_DOWNLOAD_BYTES_PER_SECOND)
    );

    let mut missing_csrf = admin_headers.clone();
    missing_csrf.remove("x-csrf-token");
    let csrf_error = admin_update_settings(
        State(Arc::clone(&state)),
        missing_csrf,
        Extension(RequestId("req_settings_csrf".into())),
        Json(AdminUpdateSettingsRequest {
            download_bytes_per_second: Some(Some(8 * 1024 * 1024)),
        }),
    )
    .await
    .expect_err("settings PATCH requires CSRF");
    assert_eq!(csrf_error.status, StatusCode::FORBIDDEN);

    let updated = admin_update_settings(
        State(Arc::clone(&state)),
        admin_headers.clone(),
        Extension(RequestId("req_settings_update".into())),
        Json(AdminUpdateSettingsRequest {
            download_bytes_per_second: Some(Some(8 * 1024 * 1024)),
        }),
    )
    .await
    .expect("update settings");
    assert_eq!(updated.0.download_bytes_per_second, Some(8 * 1024 * 1024));
    let persisted = admin_settings(State(Arc::clone(&state)), admin_headers.clone())
        .await
        .expect("persisted settings");
    assert_eq!(
        persisted.0.download_bytes_per_second,
        updated.0.download_bytes_per_second
    );
    let audit = state
        .repository
        .list_admin_audit(100)
        .await
        .expect("settings audit");
    assert!(
        audit
            .iter()
            .any(|event| event.action == "system.settings_updated"
                && event.request_id == "req_settings_update")
    );

    let unlimited = admin_update_settings(
        State(Arc::clone(&state)),
        admin_headers.clone(),
        Extension(RequestId("req_settings_unlimited".into())),
        Json(AdminUpdateSettingsRequest {
            download_bytes_per_second: Some(None),
        }),
    )
    .await
    .expect("disable download limit");
    assert_eq!(unlimited.0.download_bytes_per_second, None);

    for request in [
        AdminUpdateSettingsRequest {
            download_bytes_per_second: None,
        },
        AdminUpdateSettingsRequest {
            download_bytes_per_second: Some(Some(MIN_DOWNLOAD_BYTES_PER_SECOND - 1)),
        },
        AdminUpdateSettingsRequest {
            download_bytes_per_second: Some(Some(MAX_DOWNLOAD_BYTES_PER_SECOND + 1)),
        },
    ] {
        let error = admin_update_settings(
            State(Arc::clone(&state)),
            admin_headers.clone(),
            Extension(RequestId("req_settings_invalid".into())),
            Json(request),
        )
        .await
        .expect_err("invalid settings update");
        assert_eq!(error.status, StatusCode::BAD_REQUEST);
    }

    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn head_and_not_modified_reads_do_not_query_download_settings(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (user_id, _) = authenticated_test_user(&state, "head@example.com", "user").await;
    let application = state
        .repository
        .default_application_for_user(user_id)
        .await
        .expect("application lookup")
        .expect("application");
    let bucket = Bucket::new(
        BucketId::new(),
        application.id,
        "head-test",
        BucketPolicy::unrestricted(Visibility::Public),
        OffsetDateTime::now_utc(),
    )
    .expect("bucket");
    state
        .repository
        .create_bucket(&bucket)
        .await
        .expect("create bucket");
    let media = upload_test_media(
        &state,
        application.id,
        bucket.id(),
        "read.txt",
        b"download settings query guard",
        None,
    )
    .await;

    let full = read_media_bytes(
        &state,
        &media,
        Visibility::Public,
        Method::GET,
        ReadMediaQuery::default(),
        HeaderMap::new(),
    )
    .await
    .expect("limited full response");
    assert_eq!(full.status(), StatusCode::OK);
    assert_eq!(
        full.headers().get(CONTENT_LENGTH),
        Some(&HeaderValue::from_str(&media.size().to_string()).expect("content length header"))
    );
    assert_eq!(
        to_bytes(
            full.into_body(),
            usize::try_from(media.size()).expect("media size")
        )
        .await
        .expect("full response body"),
        Bytes::from_static(b"download settings query guard")
    );

    let mut range_headers = HeaderMap::new();
    range_headers.insert(RANGE, HeaderValue::from_static("bytes=1-3"));
    let range = read_media_bytes(
        &state,
        &media,
        Visibility::Public,
        Method::GET,
        ReadMediaQuery::default(),
        range_headers,
    )
    .await
    .expect("limited Range response");
    assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        range.headers().get(CONTENT_LENGTH),
        Some(&HeaderValue::from_static("3"))
    );
    assert_eq!(
        range.headers().get(CONTENT_RANGE),
        Some(
            &HeaderValue::from_str(&format!("bytes 1-3/{}", media.size()))
                .expect("content range header")
        )
    );
    assert_eq!(
        to_bytes(range.into_body(), 3)
            .await
            .expect("Range response body"),
        Bytes::from_static(b"own")
    );

    sqlx::query("DROP TABLE system_settings")
        .execute(state.repository.pool())
        .await
        .expect("remove settings table in isolated test database");

    let head = read_media_bytes(
        &state,
        &media,
        Visibility::Public,
        Method::HEAD,
        ReadMediaQuery::default(),
        HeaderMap::new(),
    )
    .await
    .expect("HEAD must not query settings");
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(
        head.headers().get(CONTENT_LENGTH),
        Some(&HeaderValue::from_str(&media.size().to_string()).expect("content length header"))
    );
    assert!(
        to_bytes(head.into_body(), 1)
            .await
            .expect("HEAD body")
            .is_empty()
    );

    let mut conditional_headers = HeaderMap::new();
    conditional_headers.insert(IF_NONE_MATCH, entity_tag_header_value(media.etag()));
    let not_modified = read_media_bytes(
        &state,
        &media,
        Visibility::Public,
        Method::GET,
        ReadMediaQuery::default(),
        conditional_headers,
    )
    .await
    .expect("304 must not query settings");
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);

    let error = read_media_bytes(
        &state,
        &media,
        Visibility::Public,
        Method::GET,
        ReadMediaQuery::default(),
        HeaderMap::new(),
    )
    .await
    .expect_err("ordinary GET must query settings");
    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);

    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn admin_session_and_metrics_bearer_reach_protected_system_views(pool: sqlx::PgPool) {
    let mut state = auth_test_state(pool, true).await;
    Arc::get_mut(&mut state)
        .expect("exclusive state")
        .metrics_bearer_token = Some(Arc::from("m".repeat(32)));
    let storage_root = state.object_store.root().to_path_buf();
    let anonymous_metrics = metrics(State(Arc::clone(&state)), HeaderMap::new())
        .await
        .expect_err("anonymous metrics request");
    assert_eq!(anonymous_metrics.status, StatusCode::UNAUTHORIZED);
    let (_, admin_headers) = authenticated_test_user(&state, "admin@example.com", "admin").await;
    let users = admin_list_users(
        State(Arc::clone(&state)),
        admin_headers,
        Query(AdminListQuery::default()),
    )
    .await
    .expect("admin users");
    assert_eq!(users.0.len(), 1);

    let mut metric_headers = HeaderMap::new();
    metric_headers.insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", "m".repeat(32))).expect("authorization"),
    );
    let response = metrics(State(state), metric_headers)
        .await
        .expect("metrics response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get(CONTENT_TYPE),
        Some(&HeaderValue::from_static(
            "text/plain; version=0.0.4; charset=utf-8"
        ))
    );
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn cookie_application_context_is_owned_and_hmac_cannot_switch(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (user_id, mut headers) = authenticated_test_user(&state, "owner@example.com", "user").await;
    let default = state
        .repository
        .default_application_for_user(user_id)
        .await
        .expect("default lookup")
        .expect("default application");
    let selected_id = ApplicationId::new();
    state
        .repository
        .create_application(
            selected_id,
            user_id,
            "Selected",
            "app_selected",
            2048,
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("selected application");
    headers.insert(
        "x-mediahub-app-id",
        HeaderValue::from_static("app_selected"),
    );
    let selected = authenticated_application(&state, &headers, None)
        .await
        .expect("owned application context");
    assert_eq!(selected.application.id, selected_id);

    let switched = authenticated_application(
        &state,
        &headers,
        Some(HmacIdentity {
            application_id: default.id,
            access_key_id: "mh_ak_test".into(),
            permissions: vec!["application:read".into()],
        }),
    )
    .await
    .expect_err("HMAC context switch");
    assert_eq!(switched.status, StatusCode::FORBIDDEN);
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn canonical_object_paths_enforce_visibility_and_preserve_http_reads(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (user_id, auth_headers) =
        authenticated_test_user(&state, "public@example.com", "user").await;
    let application = state
        .repository
        .default_application_for_user(user_id)
        .await
        .expect("application lookup")
        .expect("application");
    let public_bucket = Bucket::new(
        BucketId::new(),
        application.id,
        "assets",
        BucketPolicy::unrestricted(Visibility::Public),
        OffsetDateTime::now_utc(),
    )
    .expect("public bucket");
    let private_bucket = Bucket::new(
        BucketId::new(),
        application.id,
        "private-assets",
        BucketPolicy::unrestricted(Visibility::Private),
        OffsetDateTime::now_utc(),
    )
    .expect("private bucket");
    state
        .repository
        .create_bucket(&public_bucket)
        .await
        .expect("persist public bucket");
    state
        .repository
        .create_bucket(&private_bucket)
        .await
        .expect("persist private bucket");

    let content = b"public-content";
    upload_test_media(
        &state,
        application.id,
        public_bucket.id(),
        "nested/path.bin",
        content,
        None,
    )
    .await;
    upload_test_media_with_mime(
        &state,
        application.id,
        public_bucket.id(),
        "preview.pdf",
        b"%PDF-1.7\n% MediaHub preview test",
        "application/pdf",
        None,
    )
    .await;

    let hidden_media = upload_test_media(
        &state,
        application.id,
        public_bucket.id(),
        "hidden.bin",
        b"hidden",
        Some(Visibility::Private),
    )
    .await;
    let signed_url = create_path_signed_url(
        State(Arc::clone(&state)),
        Path((
            application.app_id.clone(),
            public_bucket.name().to_owned(),
            hidden_media.object_key().to_owned(),
        )),
        auth_headers,
        None,
    )
    .await
    .expect("create signed URL")
    .0
    .url;
    assert!(signed_url.starts_with(&format!("/{}/assets/hidden.bin?token=", application.app_id)));
    upload_test_media(
        &state,
        application.id,
        private_bucket.id(),
        "published.bin",
        b"published",
        Some(Visibility::Public),
    )
    .await;

    let other_application_id = ApplicationId::new();
    state
        .repository
        .create_application(
            other_application_id,
            user_id,
            "Other",
            "app_public_path_other",
            1024,
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("other application");
    let other_bucket = Bucket::new(
        BucketId::new(),
        other_application_id,
        "assets",
        BucketPolicy::unrestricted(Visibility::Public),
        OffsetDateTime::now_utc(),
    )
    .expect("other bucket");
    state
        .repository
        .create_bucket(&other_bucket)
        .await
        .expect("persist other bucket");
    upload_test_media(
        &state,
        other_application_id,
        other_bucket.id(),
        "nested/path.bin",
        b"other-application",
        None,
    )
    .await;

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn({
        let application = router((*state).clone());
        async move {
            axum::serve(listener, application)
                .await
                .expect("public path test server");
        }
    });
    let client = reqwest::Client::new();
    let base = format!(
        "http://{address}/{}/{}",
        application.app_id,
        public_bucket.name()
    );
    let url = format!("{base}/nested/path.bin");

    let health = client
        .get(format!("http://{address}/health/live"))
        .send()
        .await
        .expect("static health route");
    assert_eq!(health.status(), StatusCode::OK);

    let response = client.get(&url).send().await.expect("public GET");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["accept-ranges"], "bytes");
    let etag = response.headers()["etag"]
        .to_str()
        .expect("ETag")
        .to_owned();
    assert_eq!(
        response.bytes().await.expect("public body").as_ref(),
        content
    );

    let pdf = client
        .head(format!("{base}/preview.pdf"))
        .send()
        .await
        .expect("PDF HEAD");
    assert_eq!(pdf.status(), StatusCode::OK);
    assert_eq!(pdf.headers()[CONTENT_TYPE], "application/pdf");
    assert!(
        pdf.headers()[CONTENT_DISPOSITION]
            .to_str()
            .expect("PDF content disposition")
            .starts_with("inline;")
    );
    assert_eq!(pdf.headers()[CONTENT_SECURITY_POLICY], "sandbox");
    assert_eq!(pdf.headers()[X_CONTENT_TYPE_OPTIONS], "nosniff");

    let range = client
        .get(&url)
        .header("Range", "bytes=1-3")
        .send()
        .await
        .expect("range GET");
    assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(range.headers()["content-range"], "bytes 1-3/14");
    assert_eq!(range.bytes().await.expect("range body").as_ref(), b"ubl");

    let not_modified = client
        .get(&url)
        .header("If-None-Match", &etag)
        .send()
        .await
        .expect("conditional GET");
    assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);

    let head = client.head(&url).send().await.expect("public HEAD");
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()["content-length"], content.len().to_string());
    assert!(head.bytes().await.expect("HEAD body").is_empty());

    let hidden = client
        .get(format!("{base}/hidden.bin"))
        .send()
        .await
        .expect("private override GET");
    assert_eq!(hidden.status(), StatusCode::NOT_FOUND);

    let legacy = client
        .get(format!("http://{address}/media/{}", hidden_media.id()))
        .send()
        .await
        .expect("removed legacy media route");
    assert_eq!(legacy.status(), StatusCode::NOT_FOUND);

    let legacy_public = client
        .get(format!(
            "http://{address}/public/{}/assets/nested/path.bin",
            application.app_id
        ))
        .send()
        .await
        .expect("removed public prefix route");
    assert_eq!(legacy_public.status(), StatusCode::NOT_FOUND);

    let legacy_signed = client
        .get(format!("http://{address}/signed/{}", hidden_media.id()))
        .send()
        .await
        .expect("removed signed prefix route");
    assert_eq!(legacy_signed.status(), StatusCode::NOT_FOUND);

    let signed = client
        .get(format!("http://{address}{signed_url}"))
        .send()
        .await
        .expect("signed private GET");
    assert_eq!(signed.status(), StatusCode::OK);
    assert_eq!(
        signed.bytes().await.expect("signed body").as_ref(),
        b"hidden"
    );

    let published = client
        .get(format!(
            "http://{address}/{}/private-assets/published.bin",
            application.app_id
        ))
        .send()
        .await
        .expect("public override GET");
    assert_eq!(published.status(), StatusCode::OK);
    assert_eq!(
        published.bytes().await.expect("published body").as_ref(),
        b"published"
    );

    let other = client
        .get(
            "http://".to_owned()
                + &address.to_string()
                + "/app_public_path_other/assets/nested/path.bin",
        )
        .send()
        .await
        .expect("other application GET");
    assert_eq!(other.status(), StatusCode::OK);
    assert_eq!(
        other.bytes().await.expect("other body").as_ref(),
        b"other-application"
    );

    server.abort();
    let _ = server.await;
    drop(client);
    drop(state);
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn webdav_and_path_object_api_share_the_durable_object_model(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (user_id, session_headers) =
        authenticated_test_user(&state, "dav@example.com", "user").await;
    let application = state
        .repository
        .default_application_for_user(user_id)
        .await
        .expect("application lookup")
        .expect("application");
    let bucket = Bucket::new(
        BucketId::new(),
        application.id,
        "documents",
        BucketPolicy::unrestricted(Visibility::Private),
        OffsetDateTime::now_utc(),
    )
    .expect("bucket");
    state
        .repository
        .create_bucket(&bucket)
        .await
        .expect("persist bucket");
    upload_test_media(
        &state,
        application.id,
        bucket.id(),
        "nested/source.txt",
        b"source-content",
        None,
    )
    .await;

    let access_key_id = "mh_ak_webdav";
    let access_key_secret = "webdav-secret";
    state
        .repository
        .create_access_key(&NewAccessKey {
            id: uuid::Uuid::now_v7().to_string(),
            application_id: application.id,
            access_key_id: access_key_id.into(),
            secret_ciphertext: state
                .access_key_cipher
                .encrypt(access_key_secret.as_bytes())
                .expect("encrypt access key"),
            secret_key_version: state.access_key_cipher.version(),
            secret_last_four: "cret".into(),
            name: "WebDAV".into(),
            permissions: vec![
                "application:read".into(),
                "bucket:list".into(),
                "bucket:manage".into(),
                "media:list".into(),
                "media:read".into(),
                "media:upload".into(),
                "media:delete".into(),
            ],
            expires_at: None,
            created_at: OffsetDateTime::now_utc(),
        })
        .await
        .expect("persist WebDAV access key");

    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn({
        let application = router((*state).clone());
        async move {
            axum::serve(listener, application)
                .await
                .expect("WebDAV test server");
        }
    });
    let client = reqwest::Client::new();
    let dav_root = format!("http://{address}/dav/");
    let dav_application = format!("{dav_root}{}/", application.app_id);
    let dav_bucket = format!("{dav_application}{}/", bucket.name());

    let anonymous = client
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").expect("PROPFIND"),
            &dav_root,
        )
        .send()
        .await
        .expect("anonymous DAV request");
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        anonymous.headers()["www-authenticate"],
        "Basic realm=\"MediaHub WebDAV\", charset=\"UTF-8\""
    );

    let options = client
        .request(reqwest::Method::OPTIONS, &dav_root)
        .basic_auth(access_key_id, Some(access_key_secret))
        .send()
        .await
        .expect("DAV OPTIONS");
    assert_eq!(options.status(), StatusCode::OK);
    assert!(
        options.headers().contains_key("dav"),
        "OPTIONS headers: {:?}",
        options.headers()
    );

    let root_listing = client
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").expect("PROPFIND"),
            &dav_root,
        )
        .basic_auth(access_key_id, Some(access_key_secret))
        .header("Depth", "1")
        .send()
        .await
        .expect("root PROPFIND");
    assert_eq!(root_listing.status(), StatusCode::MULTI_STATUS);
    assert!(
        root_listing
            .text()
            .await
            .expect("root PROPFIND body")
            .contains(&application.app_id)
    );

    let bucket_listing = client
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").expect("PROPFIND"),
            &dav_bucket,
        )
        .basic_auth(access_key_id, Some(access_key_secret))
        .header("Depth", "1")
        .send()
        .await
        .expect("bucket PROPFIND");
    assert_eq!(bucket_listing.status(), StatusCode::MULTI_STATUS);
    let bucket_listing_body = bucket_listing.text().await.expect("bucket PROPFIND body");
    assert!(
        bucket_listing_body.contains("nested"),
        "bucket PROPFIND body: {bucket_listing_body}"
    );

    let range = client
        .get(format!("{dav_bucket}nested/source.txt"))
        .basic_auth(access_key_id, Some(access_key_secret))
        .header("Range", "bytes=1-3")
        .send()
        .await
        .expect("DAV range GET");
    assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(
        range.bytes().await.expect("DAV range body").as_ref(),
        b"our"
    );

    let put_url = format!("{dav_bucket}nested/uploaded.txt");
    let put = client
        .put(&put_url)
        .basic_auth(access_key_id, Some(access_key_secret))
        .header(CONTENT_TYPE, "text/plain")
        .body("uploaded-content")
        .send()
        .await
        .expect("DAV PUT");
    assert_eq!(put.status(), StatusCode::CREATED);
    let duplicate = client
        .put(&put_url)
        .basic_auth(access_key_id, Some(access_key_secret))
        .header(CONTENT_TYPE, "text/plain")
        .body("replacement")
        .send()
        .await
        .expect("duplicate DAV PUT");
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);

    let new_bucket_url = format!("{dav_application}dav-created/");
    let mkcol = client
        .request(
            reqwest::Method::from_bytes(b"MKCOL").expect("MKCOL"),
            &new_bucket_url,
        )
        .basic_auth(access_key_id, Some(access_key_secret))
        .send()
        .await
        .expect("DAV MKCOL");
    assert_eq!(mkcol.status(), StatusCode::CREATED);

    let copy_url = format!("{dav_bucket}nested/copied.txt");
    let copy = client
        .request(
            reqwest::Method::from_bytes(b"COPY").expect("COPY"),
            format!("{dav_bucket}nested/source.txt"),
        )
        .basic_auth(access_key_id, Some(access_key_secret))
        .header("Destination", &copy_url)
        .send()
        .await
        .expect("DAV COPY");
    assert_eq!(copy.status(), StatusCode::CREATED);

    let moved_url = format!("{dav_bucket}nested/moved.txt");
    let moved = client
        .request(
            reqwest::Method::from_bytes(b"MOVE").expect("MOVE"),
            &copy_url,
        )
        .basic_auth(access_key_id, Some(access_key_secret))
        .header("Destination", &moved_url)
        .send()
        .await
        .expect("DAV MOVE");
    assert_eq!(moved.status(), StatusCode::CREATED);
    let removed_source = client
        .get(&copy_url)
        .basic_auth(access_key_id, Some(access_key_secret))
        .send()
        .await
        .expect("moved source GET");
    assert_eq!(removed_source.status(), StatusCode::NOT_FOUND);
    let delete = client
        .delete(&moved_url)
        .basic_auth(access_key_id, Some(access_key_secret))
        .send()
        .await
        .expect("DAV DELETE");
    assert_eq!(delete.status(), StatusCode::NO_CONTENT);

    let renamed_directory = format!("{dav_bucket}renamed/");
    let directory_move = client
        .request(
            reqwest::Method::from_bytes(b"MOVE").expect("MOVE"),
            format!("{dav_bucket}nested/"),
        )
        .basic_auth(access_key_id, Some(access_key_secret))
        .header("Destination", &renamed_directory)
        .send()
        .await
        .expect("DAV directory MOVE");
    assert_eq!(directory_move.status(), StatusCode::CREATED);
    let renamed_source = client
        .get(format!("{renamed_directory}source.txt"))
        .basic_auth(access_key_id, Some(access_key_secret))
        .send()
        .await
        .expect("renamed directory object GET");
    assert_eq!(renamed_source.status(), StatusCode::OK);
    assert_eq!(
        renamed_source
            .bytes()
            .await
            .expect("renamed directory object body")
            .as_ref(),
        b"source-content"
    );
    let old_directory_source = client
        .get(format!("{dav_bucket}nested/source.txt"))
        .basic_auth(access_key_id, Some(access_key_secret))
        .send()
        .await
        .expect("old directory object GET");
    assert_eq!(old_directory_source.status(), StatusCode::NOT_FOUND);

    let isolated = client
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").expect("PROPFIND"),
            format!("{dav_root}app_other/documents/"),
        )
        .basic_auth(access_key_id, Some(access_key_secret))
        .header("Depth", "0")
        .send()
        .await
        .expect("isolated PROPFIND");
    assert_eq!(isolated.status(), StatusCode::NOT_FOUND);

    let cookie = session_headers["cookie"].to_str().expect("cookie");
    let csrf = session_headers["x-csrf-token"].to_str().expect("csrf");
    let path_object_url = format!(
        "http://{address}/{}/{}/path-api.txt",
        application.app_id,
        bucket.name()
    );
    let path_put = client
        .put(&path_object_url)
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .header("X-MediaHub-App-Id", &application.app_id)
        .header(CONTENT_TYPE, "text/plain")
        .body("path-content")
        .send()
        .await
        .expect("path object PUT");
    assert_eq!(path_put.status(), StatusCode::CREATED);
    assert!(path_put.headers().contains_key(ETAG));
    let path_media_id = path_put.headers()["x-mediahub-media-id"]
        .to_str()
        .expect("path Media ID")
        .to_owned();

    let path_patch = client
        .patch(&path_object_url)
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .header("X-MediaHub-App-Id", &application.app_id)
        .header("If-Match", "\"1\"")
        .json(&serde_json::json!({ "display_name": "Path API object" }))
        .send()
        .await
        .expect("path object PATCH");
    assert_eq!(path_patch.status(), StatusCode::OK);
    assert_eq!(
        path_patch
            .json::<serde_json::Value>()
            .await
            .expect("path PATCH JSON")["revision"],
        2
    );
    let signed_path = client
        .post(&path_object_url)
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .header("X-MediaHub-App-Id", &application.app_id)
        .send()
        .await
        .expect("path signed URL");
    assert_eq!(signed_path.status(), StatusCode::OK);
    assert!(
        signed_path
            .json::<serde_json::Value>()
            .await
            .expect("signed path JSON")["url"]
            .as_str()
            .is_some_and(|url| url.contains("/documents/path-api.txt?token="))
    );
    let removed_media_id_route = client
        .get(format!("http://{address}/api/v1/media/{path_media_id}"))
        .header("Cookie", cookie)
        .header("X-MediaHub-App-Id", &application.app_id)
        .send()
        .await
        .expect("removed media-ID item route");
    assert_eq!(removed_media_id_route.status(), StatusCode::NOT_FOUND);

    let private_read = client
        .get(&path_object_url)
        .send()
        .await
        .expect("anonymous private path GET");
    assert_eq!(private_read.status(), StatusCode::NOT_FOUND);
    let hmac_private_read = read_object_content(
        State(Arc::clone(&state)),
        Path((
            application.app_id.clone(),
            bucket.name().to_owned(),
            "path-api.txt".to_owned(),
        )),
        Method::GET,
        Ok(Query(ReadMediaQuery::default())),
        HeaderMap::new(),
        Some(Extension(HmacIdentity {
            application_id: application.id,
            access_key_id: access_key_id.to_owned(),
            permissions: vec!["media:read".into()],
        })),
    )
    .await
    .expect("HMAC private path GET");
    assert_eq!(hmac_private_read.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(hmac_private_read.into_body(), MAX_REQUEST_BYTES)
            .await
            .expect("HMAC private body"),
        Bytes::from_static(b"path-content")
    );
    let forbidden_hmac_read = read_object_content(
        State(Arc::clone(&state)),
        Path((
            application.app_id.clone(),
            bucket.name().to_owned(),
            "path-api.txt".to_owned(),
        )),
        Method::GET,
        Ok(Query(ReadMediaQuery::default())),
        HeaderMap::new(),
        Some(Extension(HmacIdentity {
            application_id: application.id,
            access_key_id: "mh_ak_without_read".into(),
            permissions: vec!["media:list".into()],
        })),
    )
    .await
    .expect_err("HMAC without media:read");
    assert_eq!(forbidden_hmac_read.status, StatusCode::NOT_FOUND);
    let path_list = client
        .get(format!(
            "http://{address}/{}/{}?prefix=path-",
            application.app_id,
            bucket.name()
        ))
        .header("Cookie", cookie)
        .header("X-MediaHub-App-Id", &application.app_id)
        .send()
        .await
        .expect("path object list");
    assert_eq!(path_list.status(), StatusCode::OK);
    assert_eq!(
        path_list
            .json::<serde_json::Value>()
            .await
            .expect("path list JSON")["items"][0]["object_key"],
        "path-api.txt"
    );
    let path_delete = client
        .delete(&path_object_url)
        .header("Cookie", cookie)
        .header("X-CSRF-Token", csrf)
        .header("X-MediaHub-App-Id", &application.app_id)
        .send()
        .await
        .expect("path object DELETE");
    assert_eq!(path_delete.status(), StatusCode::ACCEPTED);

    let audit = state
        .repository
        .list_audit(application.id, 100)
        .await
        .expect("WebDAV audit list");
    for action in [
        "bucket.created",
        "media.uploaded",
        "media.copied",
        "media.delete_scheduled",
    ] {
        assert!(
            audit.iter().any(|event| {
                event.action == action
                    && event.actor_type == "access_key"
                    && event.actor_id == access_key_id
            }),
            "missing WebDAV audit action {action}"
        );
    }

    server.abort();
    let _ = server.await;
    drop(client);
    drop(state);
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn upload_session_get_refreshes_target_without_leaking_storage_key(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (user_id, headers) = authenticated_test_user(&state, "uploader@example.com", "user").await;
    let application = state
        .repository
        .default_application_for_user(user_id)
        .await
        .expect("application lookup")
        .expect("application");
    let bucket = Bucket::new(
        BucketId::new(),
        application.id,
        "uploads",
        BucketPolicy::unrestricted(Visibility::Private),
        OffsetDateTime::now_utc(),
    )
    .expect("bucket");
    state
        .repository
        .create_bucket(&bucket)
        .await
        .expect("bucket insert");
    let receipt = upload_session_service(&state)
        .create(&CreateUploadSessionRequest {
            application_id: application.id,
            bucket_id: bucket.id(),
            object_key: "resume.bin".into(),
            original_name: Some("resume.bin".into()),
            display_name: "Resume".into(),
            extension: Some("bin".into()),
            expected_size: 16,
            expected_mime: "application/octet-stream".into(),
            visibility_override: None,
            media_expires_at: None,
            metadata: ClientMetadata::default(),
        })
        .await
        .expect("upload session");
    let response = get_upload_session(
        State(Arc::clone(&state)),
        Path(receipt.session.id().to_string()),
        headers,
        None,
    )
    .await
    .expect("get upload session")
    .0;
    assert_eq!(response.state, UploadSessionState::Pending);
    assert!(response.upload_target.is_some());
    let payload = serde_json::to_value(response).expect("serialize response");
    assert!(payload.get("storage_key").is_none());
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn local_upload_content_streams_beyond_the_general_request_limit(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let (user_id, _) = authenticated_test_user(&state, "large-uploader@example.com", "user").await;
    let application = state
        .repository
        .default_application_for_user(user_id)
        .await
        .expect("application lookup")
        .expect("application");
    let expected_size = MAX_REQUEST_BYTES as u64 + 1;
    state
        .repository
        .update_application_quota(
            user_id,
            application.id,
            expected_size * 2,
            "req_large_upload_test",
            OffsetDateTime::now_utc(),
        )
        .await
        .expect("raise test application quota");
    let bucket = Bucket::new(
        BucketId::new(),
        application.id,
        "large-uploads",
        BucketPolicy::unrestricted(Visibility::Private),
        OffsetDateTime::now_utc(),
    )
    .expect("bucket");
    state
        .repository
        .create_bucket(&bucket)
        .await
        .expect("bucket insert");
    let receipt = upload_session_service(&state)
        .create(&CreateUploadSessionRequest {
            application_id: application.id,
            bucket_id: bucket.id(),
            object_key: "streamed-large.bin".into(),
            original_name: Some("streamed-large.bin".into()),
            display_name: "Streamed large upload".into(),
            extension: Some("bin".into()),
            expected_size,
            expected_mime: "application/octet-stream".into(),
            visibility_override: None,
            media_expires_at: None,
            metadata: ClientMetadata::default(),
        })
        .await
        .expect("large upload session");
    let token = state
        .media_url_signer
        .sign_upload_content(receipt.session.id(), receipt.session.session_expires_at());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn({
        let application = router((*state).clone());
        async move {
            axum::serve(listener, application)
                .await
                .expect("large upload test server");
        }
    });
    let response = reqwest::Client::new()
        .put(format!(
            "http://{address}/api/v1/uploads/{}/content?token={token}",
            receipt.session.id()
        ))
        .header(CONTENT_TYPE, "application/octet-stream")
        .body(vec![
            0x5a;
            usize::try_from(expected_size).expect("test size")
        ])
        .send()
        .await
        .expect("streamed upload request");
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    let stored = state
        .object_store
        .inspect_upload(&receipt.session)
        .await
        .expect("inspect streamed upload");
    assert_eq!(stored.size, expected_size);
    assert_eq!(stored.mime, "application/octet-stream");
    state
        .object_store
        .abort_upload(&receipt.session)
        .await
        .expect("remove streamed upload");
    server.abort();
    let _ = server.await;
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[tokio::test]
async fn capabilities_report_the_readme_contract() {
    let response = capabilities().await.0;
    assert_eq!(response.deployment_profile, "docker");
    assert_eq!(response.storage, ["local", "s3"]);
    assert!(response.s3_gateway);
    assert!(response.image_processing);
    assert!(!response.video_processing);
    assert!(!response.resumable_upload);
    assert!(!response.archive_restore);
}

#[tokio::test]
async fn resend_receives_authenticated_rendered_email() {
    let captured = Arc::new(Mutex::new(None::<serde_json::Value>));
    let captured_request = Arc::clone(&captured);
    let provider_app = Router::new().route(
        "/emails",
        post(
            move |headers: HeaderMap, Json(mut payload): Json<serde_json::Value>| {
                let captured_request = Arc::clone(&captured_request);
                async move {
                    payload["authorization"] = serde_json::Value::String(
                        headers
                            .get(axum::http::header::AUTHORIZATION)
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default()
                            .to_owned(),
                    );
                    payload["idempotency_key"] = serde_json::Value::String(
                        headers
                            .get("idempotency-key")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default()
                            .to_owned(),
                    );
                    *captured_request.lock().expect("capture lock") = Some(payload);
                    (StatusCode::OK, Json(serde_json::json!({"id": "email_123"})))
                }
            },
        ),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        axum::serve(listener, provider_app)
            .await
            .expect("provider server");
    });
    let provider = ResendEmailProvider::new_with_endpoint(
        server_config::ResendConfig {
            api_key: "re_provider_secret".into(),
            from: "MediaHub <mediahub@example.com>".into(),
            web_url: Url::parse("https://console.example.com").expect("Web URL"),
        },
        Url::parse(&format!("http://{address}/emails")).expect("provider URL"),
    );
    let sent = provider
        .send_token(
            "owner@example.com",
            AuthEmailKind::VerifyEmail,
            "raw-token",
            OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(30),
        )
        .await;
    assert!(sent.is_ok());
    let payload = captured
        .lock()
        .expect("capture lock")
        .clone()
        .expect("captured request");
    assert_eq!(payload["from"], "MediaHub <mediahub@example.com>");
    assert_eq!(payload["to"][0], "owner@example.com");
    assert_eq!(payload["subject"], "Verify your MediaHub email");
    assert!(
        payload["html"]
            .as_str()
            .expect("HTML body")
            .contains("https://console.example.com/verify-email?token=raw-token")
    );
    assert!(
        payload["text"]
            .as_str()
            .expect("text body")
            .contains("raw-token")
    );
    assert_eq!(payload["authorization"], "Bearer re_provider_secret");
    assert!(
        payload["idempotency_key"]
            .as_str()
            .expect("idempotency key")
            .starts_with("mediahub-verify_email-")
    );
    assert!(
        !payload["idempotency_key"]
            .as_str()
            .expect("idempotency key")
            .contains("raw-token")
    );
    server.abort();
}

#[tokio::test]
async fn resend_rejection_is_reported_as_unavailable() {
    let provider_app = Router::new().route(
        "/emails",
        post(|| async {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "name": "validation_error",
                    "statusCode": 422,
                    "message": "sender is invalid"
                })),
            )
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("listener");
    let address = listener.local_addr().expect("address");
    let server = tokio::spawn(async move {
        axum::serve(listener, provider_app)
            .await
            .expect("provider server");
    });
    let provider = ResendEmailProvider::new_with_endpoint(
        server_config::ResendConfig {
            api_key: "re_provider_secret".into(),
            from: "MediaHub <mediahub@example.com>".into(),
            web_url: Url::parse("https://console.example.com").expect("Web URL"),
        },
        Url::parse(&format!("http://{address}/emails")).expect("provider URL"),
    );
    let error = provider
        .send_token(
            "owner@example.com",
            AuthEmailKind::ResetPassword,
            "raw-token",
            OffsetDateTime::UNIX_EPOCH + time::Duration::minutes(15),
        )
        .await
        .expect_err("Resend rejection must fail");
    assert_eq!(error.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.code, "unavailable");
    assert_eq!(error.message, "email delivery was rejected");
    server.abort();
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn disabled_public_registration_returns_a_uniform_error(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, false).await;
    let storage_root = state.object_store.root().to_path_buf();
    let connect_info = ConnectInfo("127.0.0.1:12345".parse().expect("address"));
    let Err(valid_input_error) = register(
        State(Arc::clone(&state)),
        connect_info,
        Json(RegisterRequest {
            email: "owner@example.com".into(),
            password: "valid-password-123".into(),
        }),
    )
    .await
    else {
        panic!("disabled registration must reject valid input");
    };
    let Err(invalid_input_error) = register(
        State(state),
        connect_info,
        Json(RegisterRequest {
            email: "not-an-email".into(),
            password: "short".into(),
        }),
    )
    .await
    else {
        panic!("disabled registration must reject invalid input identically");
    };

    assert_eq!(valid_input_error.status, StatusCode::FORBIDDEN);
    assert_eq!(valid_input_error.code, "registration_disabled");
    assert_eq!(valid_input_error.message, "public registration is disabled");
    assert_eq!(valid_input_error.status, invalid_input_error.status);
    assert_eq!(valid_input_error.code, invalid_input_error.code);
    assert_eq!(valid_input_error.message, invalid_input_error.message);
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn email_verification_and_password_reset_complete_the_auth_lifecycle(pool: sqlx::PgPool) {
    let state = auth_test_state(pool, true).await;
    let storage_root = state.object_store.root().to_path_buf();
    let connect_info = ConnectInfo("127.0.0.1:12345".parse().expect("address"));
    let original_password = "original-password-123";
    let new_password = "replacement-password-456";

    let Ok((status, Json(registration))) = register(
        State(Arc::clone(&state)),
        connect_info,
        Json(RegisterRequest {
            email: "Owner@Example.com".into(),
            password: original_password.into(),
        }),
    )
    .await
    else {
        panic!("registration should succeed");
    };
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(registration.status, "pending_verification");
    let verification_token = registration
        .verification_token
        .expect("development verification token");
    let Ok((resend_status, Json(resend))) = resend_verification(
        State(Arc::clone(&state)),
        connect_info,
        Json(ForgotPasswordRequest {
            email: "owner@example.com".into(),
        }),
    )
    .await
    else {
        panic!("verification resend should succeed");
    };
    assert_eq!(resend_status, StatusCode::ACCEPTED);
    let resent_token = resend
        .verification_token
        .expect("development verification token");

    let Err(error) = login(
        State(Arc::clone(&state)),
        connect_info,
        HeaderMap::new(),
        Json(LoginRequest {
            email: "owner@example.com".into(),
            password: original_password.into(),
        }),
    )
    .await
    else {
        panic!("pending user must not log in");
    };
    assert_eq!(error.code, "invalid_credentials");

    assert!(
        verify_email(
            State(Arc::clone(&state)),
            connect_info,
            Json(OneTimeTokenRequest {
                token: verification_token,
            }),
        )
        .await
        .is_err()
    );

    let Ok(Json(verified)) = verify_email(
        State(Arc::clone(&state)),
        connect_info,
        Json(OneTimeTokenRequest {
            token: resent_token,
        }),
    )
    .await
    else {
        panic!("verification should succeed");
    };
    assert_eq!(verified.status, "active");
    let login_response = login(
        State(Arc::clone(&state)),
        connect_info,
        HeaderMap::new(),
        Json(LoginRequest {
            email: "owner@example.com".into(),
            password: original_password.into(),
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("verified user should log in"));
    assert_eq!(login_response.status(), StatusCode::OK);

    let Ok((existing_status, Json(existing))) = forgot_password(
        State(Arc::clone(&state)),
        connect_info,
        Json(ForgotPasswordRequest {
            email: "owner@example.com".into(),
        }),
    )
    .await
    else {
        panic!("forgot password should succeed");
    };
    let Ok((missing_status, Json(missing))) = forgot_password(
        State(Arc::clone(&state)),
        connect_info,
        Json(ForgotPasswordRequest {
            email: "missing@example.com".into(),
        }),
    )
    .await
    else {
        panic!("unknown account should receive the generic response");
    };
    assert_eq!(existing_status, missing_status);
    assert_eq!(existing.message, missing.message);
    assert!(missing.reset_token.is_some());
    let reset_token = existing.reset_token.expect("development reset token");

    let reset_response = reset_password(
        State(Arc::clone(&state)),
        connect_info,
        Json(ResetPasswordRequest {
            token: reset_token,
            password: new_password.into(),
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("password reset should succeed"));
    assert_eq!(reset_response.status(), StatusCode::NO_CONTENT);
    let Err(error) = login(
        State(Arc::clone(&state)),
        connect_info,
        HeaderMap::new(),
        Json(LoginRequest {
            email: "owner@example.com".into(),
            password: original_password.into(),
        }),
    )
    .await
    else {
        panic!("old password must stop working");
    };
    assert_eq!(error.code, "invalid_credentials");
    assert!(
        login(
            State(state),
            connect_info,
            HeaderMap::new(),
            Json(LoginRequest {
                email: "owner@example.com".into(),
                password: new_password.into(),
            }),
        )
        .await
        .is_ok()
    );
    std::fs::remove_dir_all(storage_root).expect("remove temporary object store");
}

#[test]
fn authentication_tokens_are_url_safe_and_rate_limits_are_enforced() {
    let token = generate_auth_token();
    assert!(validate_one_time_token(&token).is_ok());
    assert!(validate_one_time_token("too-short").is_err());

    let limiter = AuthRateLimiter::default();
    let window = StdDuration::from_secs(60);
    assert!(limiter.check("login:test".into(), 2, window).is_ok());
    assert!(limiter.check("login:test".into(), 2, window).is_ok());
    let Err(error) = limiter.check("login:test".into(), 2, window) else {
        panic!("third attempt should be rate limited");
    };
    assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(error.code, "rate_limited");
}

#[test]
fn if_none_match_uses_weak_entity_tag_comparison() {
    let mut headers = HeaderMap::new();
    headers.insert(
        IF_NONE_MATCH,
        HeaderValue::from_static("\"other\", W/\"content-hash\""),
    );

    assert!(if_none_match_matches(&headers, "content-hash"));
    assert!(!if_none_match_matches(&headers, "different-hash"));
}

#[test]
fn if_none_match_accepts_wildcard() {
    let mut headers = HeaderMap::new();
    headers.insert(IF_NONE_MATCH, HeaderValue::from_static("*"));

    assert!(if_none_match_matches(&headers, "content-hash"));
}

#[test]
fn object_content_paths_encode_segments_but_preserve_key_hierarchy() {
    assert_eq!(
        object_content_path("app demo", "media assets", "nested/图片 1.png"),
        "/app%20demo/media%20assets/nested/%E5%9B%BE%E7%89%87%201.png"
    );
}

#[test]
fn pdf_content_is_inline_but_remains_sandboxed() {
    assert_eq!(content_disposition_type("application/pdf"), "inline");
    assert!(media_requires_sandbox("application/pdf"));
    assert_eq!(content_disposition_type("text/html"), "attachment");
    assert_eq!(content_disposition_type("image/svg+xml"), "attachment");
    assert_eq!(
        content_disposition_type("application/octet-stream"),
        "attachment"
    );
}

#[test]
fn read_query_accepts_format_only_original_size_transform() {
    let transform = ReadMediaQuery {
        format: Some(VariantFormat::Webp),
        ..ReadMediaQuery::default()
    }
    .transform()
    .expect("valid format-only query")
    .expect("transform");

    assert_eq!(transform.width(), None);
    assert_eq!(transform.height(), None);
    assert_eq!(transform.format(), VariantFormat::Webp);
}

#[test]
fn read_query_rejects_avif_variant_output() {
    let uri: http::Uri = "/app/bucket/object?format=avif".parse().expect("URI");
    let result: Result<axum::extract::Query<ReadMediaQuery>, _> =
        axum::extract::Query::try_from_uri(&uri);
    let error = match parse_read_media_query(result) {
        Ok(_) => panic!("AVIF query unexpectedly parsed"),
        Err(error) => error,
    };

    assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error.code, "invalid_request");
    assert_eq!(
        error.message,
        "media query is invalid; format must be jpeg, png, or webp"
    );
}

#[test]
fn upload_mime_is_detected_from_content_instead_of_multipart_metadata() {
    assert_eq!(
        detected_mime(b"\x89PNG\r\n\x1a\nnot-a-real-image"),
        "image/png"
    );
    assert_eq!(
        detected_mime(b"<html><script>1</script></html>"),
        "text/html"
    );
    assert_eq!(
        detected_mime(&[0, 159, 146, 150]),
        "application/octet-stream"
    );
}

#[test]
fn generated_object_key_uses_a_server_identifier_and_safe_extension() {
    let key = generated_object_key(Some("My Upload.PNG"));
    assert!(key.starts_with("uploads/"));
    assert!(key.ends_with(".png"));
    assert_ne!(key, generated_object_key(Some("My Upload.PNG")));
    assert!(!generated_object_key(Some("archive.invalid-extension!")).ends_with('!'));
}

#[test]
fn webhook_url_validation_rejects_local_and_private_destinations() {
    for value in [
        "http://localhost/hooks",
        "http://127.0.0.1/hooks",
        "http://10.0.0.8/hooks",
        "http://[::1]/hooks",
        "http://[::ffff:127.0.0.1]/hooks",
        "http://100.64.0.1/hooks",
        "http://198.18.0.1/hooks",
        "http://[2001:db8::1]/hooks",
        "ftp://example.com/hooks",
    ] {
        assert!(validate_webhook_url(value.to_owned()).is_err(), "{value}");
    }
    let Ok(url) = validate_webhook_url("https://hooks.example.com/events".to_owned()) else {
        panic!("public HTTPS URL should be accepted");
    };
    assert_eq!(url, "https://hooks.example.com/events");
    assert!(is_public_webhook_ip("8.8.8.8".parse().expect("IP")));
}

#[test]
fn webhook_events_must_be_known_and_are_normalized() {
    let Ok(events) = validate_webhook_events(vec!["media.uploaded".into(), "media.deleted".into()])
    else {
        panic!("known events should be accepted");
    };
    assert_eq!(events, vec!["media.deleted", "media.uploaded"]);
    assert!(validate_webhook_events(vec!["unknown.event".into()]).is_err());
    assert!(
        validate_webhook_events(vec!["media.uploaded".into(), "media.uploaded".into()]).is_ok()
    );
}

#[test]
fn signed_media_url_tokens_bind_the_media_and_expire() {
    let signer = MediaUrlSigner::new(vec![7; 32]);
    let now = OffsetDateTime::UNIX_EPOCH;
    let media_id = MediaId::new();
    let token = signer.sign(media_id, now + time::Duration::seconds(60));

    assert_eq!(signer.verify(&token, media_id, now), Ok(()));
    assert_eq!(
        signer.verify(&token, MediaId::new(), now),
        Err(SignedMediaUrlError::Invalid)
    );
    assert_eq!(
        signer.verify(&token, media_id, now + time::Duration::seconds(60)),
        Err(SignedMediaUrlError::Expired)
    );
}

#[test]
fn signed_upload_tokens_bind_the_session_method_and_expire() {
    let signer = MediaUrlSigner::new(vec![9; 32]);
    let now = OffsetDateTime::UNIX_EPOCH;
    let upload_session_id = UploadSessionId::new();
    let token = signer.sign_upload_content(upload_session_id, now + time::Duration::seconds(60));

    assert_eq!(
        signer.verify_upload_content(&token, upload_session_id, now),
        Ok(())
    );
    assert_eq!(
        signer.verify_upload_content(&token, UploadSessionId::new(), now),
        Err(SignedMediaUrlError::Invalid)
    );
    assert_eq!(
        signer.verify_upload_content(&token, upload_session_id, now + time::Duration::seconds(60)),
        Err(SignedMediaUrlError::Expired)
    );
}

#[test]
fn upload_content_type_is_normalized_and_validated() {
    let Ok(normalized) = normalized_mime(" Image/PNG ") else {
        panic!("content type should be valid");
    };
    assert_eq!(normalized, "image/png");
    assert!(normalized_mime("image png").is_err());
    assert!(normalized_mime("png").is_err());
}

#[test]
fn upload_session_request_accepts_only_canonical_bucket_and_content_type() {
    let canonical = serde_json::json!({
        "bucket": "images",
        "expected_size": 42,
        "content_type": "image/png"
    });
    assert!(serde_json::from_value::<CreateUploadSessionHttpRequest>(canonical).is_ok());

    for legacy_or_incomplete in [
        serde_json::json!({
            "bucket_id": BucketId::new().to_string(),
            "expected_size": 42,
            "content_type": "image/png"
        }),
        serde_json::json!({
            "bucket": "images",
            "expected_size": 42,
            "expected_mime": "image/png"
        }),
        serde_json::json!({
            "expected_size": 42,
            "content_type": "image/png"
        }),
        serde_json::json!({
            "bucket": "images",
            "expected_size": 42
        }),
    ] {
        assert!(
            serde_json::from_value::<CreateUploadSessionHttpRequest>(legacy_or_incomplete).is_err()
        );
    }
}

#[sqlx::test(migrator = "mediahub_adapter_postgres::MIGRATOR")]
async fn startup_rejects_database_key_versions_missing_from_the_keyring(pool: sqlx::PgPool) {
    let repository = PostgresRepository::new(pool);
    let now = OffsetDateTime::UNIX_EPOCH;
    let user_id = UserId::new();
    let application_id = ApplicationId::new();
    repository
        .create_user(user_id, "keyring@example.com", "hashed", now)
        .await
        .expect("user");
    repository
        .create_application(application_id, user_id, "Keyring", "app_keyring", 1024, now)
        .await
        .expect("application");
    repository
        .create_access_key(&NewAccessKey {
            id: uuid::Uuid::now_v7().to_string(),
            application_id,
            access_key_id: "mh_ak_keyring".into(),
            secret_ciphertext: "ciphertext".into(),
            secret_key_version: 2,
            secret_last_four: "last".into(),
            name: "Keyring".into(),
            permissions: vec!["media:read".into()],
            expires_at: None,
            created_at: now,
        })
        .await
        .expect("access key");
    let cipher = AccessKeyCipher::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", 1)
        .expect("cipher");

    let result = validate_referenced_key_versions(&repository, &cipher).await;
    let Err(error) = result else {
        panic!("missing keyring version should fail startup");
    };
    assert!(error.to_string().contains("version 2"));
}

#[test]
fn signed_media_url_tokens_reject_tampering() {
    let signer = MediaUrlSigner::new(vec![7; 32]);
    let now = OffsetDateTime::UNIX_EPOCH;
    let media_id = MediaId::new();
    let token = signer.sign(media_id, now + time::Duration::seconds(60));
    let (payload, signature) = token.split_once('.').expect("signed token format");
    let mut signature = URL_SAFE_NO_PAD
        .decode(signature)
        .expect("signature encoding");
    signature[0] ^= 1;
    let tampered = format!("{payload}.{}", URL_SAFE_NO_PAD.encode(signature));

    assert_eq!(
        signer.verify(&tampered, media_id, now),
        Err(SignedMediaUrlError::Invalid)
    );
}
