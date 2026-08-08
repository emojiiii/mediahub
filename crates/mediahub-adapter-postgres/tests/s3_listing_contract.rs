use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{
    NewS3MultipartUpload, RepositoryError, S3ListingRepository, S3MultipartRepository,
    S3MultipartUploadListQuery, S3ObjectRepository, S3ObjectVersionListQuery,
};
use mediahub_core::{
    ApplicationId, BucketId, Checksum, EntityTag, ObjectId, ObjectVersion, ObjectVersionId,
    ObjectVersionState, OffsetDateTime, S3Object, S3VersionId, SourceProtocol, StoredObjectVersion,
    UserId,
};
use serde_json::{json, value::Value};
use sqlx::types::Json;
use time::Duration;

#[tokio::test]
async fn postgres_s3_listing_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect listing database");
    repository
        .migrate()
        .await
        .expect("migrate listing database");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated listing database");

    let now = OffsetDateTime::now_utc();
    let application_id = ApplicationId::new();
    let bucket_id = BucketId::new();
    insert_tenant(&repository, application_id, bucket_id, now).await;

    let first_object_id = ObjectId::new();
    create_object_version(
        &repository,
        application_id,
        bucket_id,
        first_object_id,
        "a.txt",
        "null",
        1,
        true,
        now,
    )
    .await;
    repository
        .append_s3_object_version(
            first_object_id,
            1,
            ObjectVersion::new_delete_marker(
                ObjectVersionId::new(),
                first_object_id,
                application_id,
                bucket_id,
                S3VersionId::new("version-a-delete").expect("version id"),
                2,
                false,
                "listing-contract",
                SourceProtocol::S3,
                now + Duration::seconds(2),
            )
            .expect("delete marker"),
            now + Duration::seconds(2),
        )
        .await
        .expect("append delete marker");
    for (index, key) in ["b.txt", "folder/a.txt", "folder/deep/x.txt"]
        .into_iter()
        .enumerate()
    {
        create_object_version(
            &repository,
            application_id,
            bucket_id,
            ObjectId::new(),
            key,
            &format!("version-{index}"),
            1,
            false,
            now + Duration::seconds(i64::try_from(index + 3).expect("small index")),
        )
        .await;
    }

    let first_page = repository
        .list_s3_object_versions_page(
            application_id,
            &S3ObjectVersionListQuery {
                bucket_id,
                prefix: String::new(),
                key_marker: None,
                version_id_marker: None,
                delimiter: false,
                limit: 2,
            },
        )
        .await
        .expect("first version page");
    assert_eq!(first_page.items.len(), 2);
    assert_eq!(first_page.items[0].key, "a.txt");
    assert!(first_page.items[0].version.is_delete_marker());
    assert!(first_page.items[0].is_latest);
    assert_eq!(
        first_page.items[1].version.external_version_id().as_str(),
        "null"
    );
    assert!(!first_page.items[1].is_latest);
    assert_eq!(first_page.next_key_marker.as_deref(), Some("a.txt"));
    assert_eq!(
        first_page
            .next_version_id_marker
            .as_ref()
            .map(S3VersionId::as_str),
        Some("null")
    );

    let resumed = repository
        .list_s3_object_versions_page(
            application_id,
            &S3ObjectVersionListQuery {
                bucket_id,
                prefix: String::new(),
                key_marker: first_page.next_key_marker,
                version_id_marker: first_page.next_version_id_marker,
                delimiter: false,
                limit: 10,
            },
        )
        .await
        .expect("resumed version page");
    assert_eq!(
        resumed
            .items
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["b.txt", "folder/a.txt", "folder/deep/x.txt"]
    );

    let delimited = repository
        .list_s3_object_versions_page(
            application_id,
            &S3ObjectVersionListQuery {
                bucket_id,
                prefix: "folder/".to_owned(),
                key_marker: None,
                version_id_marker: None,
                delimiter: true,
                limit: 10,
            },
        )
        .await
        .expect("delimited version page");
    assert_eq!(
        delimited
            .items
            .iter()
            .map(|item| item.key.as_str())
            .collect::<Vec<_>>(),
        vec!["folder/a.txt"]
    );
    assert_eq!(delimited.common_prefixes, vec!["folder/deep/"]);

    assert_eq!(
        repository
            .list_s3_object_versions_page(
                application_id,
                &S3ObjectVersionListQuery {
                    bucket_id,
                    prefix: String::new(),
                    key_marker: Some("a.txt".to_owned()),
                    version_id_marker: Some(S3VersionId::new("missing-version").expect("marker"),),
                    delimiter: false,
                    limit: 10,
                },
            )
            .await,
        Err(RepositoryError::NotFound)
    );

    for (upload_id, key, expires_at) in [
        ("upload-2", "m/a.bin", now + Duration::hours(1)),
        ("upload-1", "m/a.bin", now + Duration::hours(1)),
        ("upload-3", "m/folder/x.bin", now + Duration::hours(1)),
        ("expired", "m/expired.bin", now + Duration::seconds(1)),
    ] {
        repository
            .create_multipart_upload(NewS3MultipartUpload {
                upload_id: upload_id.to_owned(),
                application_id,
                bucket_id,
                object_key: key.to_owned(),
                content_type: "application/octet-stream".to_owned(),
                user_metadata: Value::Object(Default::default()),
                object_tags: mediahub_core::S3ObjectTagSet::empty(),
                storage_backend: "filesystem".to_owned(),
                expires_at,
                created_at: now,
            })
            .await
            .expect("create multipart upload");
    }
    repository
        .create_multipart_upload(NewS3MultipartUpload {
            upload_id: "aborted".to_owned(),
            application_id,
            bucket_id,
            object_key: "m/aborted.bin".to_owned(),
            content_type: "application/octet-stream".to_owned(),
            user_metadata: json!({}),
            object_tags: mediahub_core::S3ObjectTagSet::empty(),
            storage_backend: "filesystem".to_owned(),
            expires_at: now + Duration::hours(1),
            created_at: now,
        })
        .await
        .expect("create upload to abort");
    repository
        .abort_multipart_upload("aborted", now + Duration::seconds(1))
        .await
        .expect("abort upload");

    let multipart_page = repository
        .list_s3_multipart_uploads_page(
            application_id,
            &S3MultipartUploadListQuery {
                bucket_id,
                prefix: "m/".to_owned(),
                key_marker: None,
                upload_id_marker: None,
                delimiter: false,
                limit: 2,
                as_of: now + Duration::seconds(2),
            },
        )
        .await
        .expect("first multipart page");
    assert_eq!(
        multipart_page
            .items
            .iter()
            .map(|item| (item.key.as_str(), item.upload_id.as_str()))
            .collect::<Vec<_>>(),
        vec![("m/a.bin", "upload-1"), ("m/a.bin", "upload-2")]
    );
    assert_eq!(multipart_page.next_key_marker.as_deref(), Some("m/a.bin"));
    assert_eq!(
        multipart_page.next_upload_id_marker.as_deref(),
        Some("upload-2")
    );

    let multipart_resumed = repository
        .list_s3_multipart_uploads_page(
            application_id,
            &S3MultipartUploadListQuery {
                bucket_id,
                prefix: "m/".to_owned(),
                key_marker: multipart_page.next_key_marker,
                upload_id_marker: multipart_page.next_upload_id_marker,
                delimiter: false,
                limit: 10,
                as_of: now + Duration::seconds(2),
            },
        )
        .await
        .expect("resumed multipart page");
    assert_eq!(multipart_resumed.items.len(), 1);
    assert_eq!(multipart_resumed.items[0].key, "m/folder/x.bin");

    let multipart_delimited = repository
        .list_s3_multipart_uploads_page(
            application_id,
            &S3MultipartUploadListQuery {
                bucket_id,
                prefix: "m/".to_owned(),
                key_marker: None,
                upload_id_marker: None,
                delimiter: true,
                limit: 10,
                as_of: now + Duration::seconds(2),
            },
        )
        .await
        .expect("delimited multipart page");
    assert_eq!(multipart_delimited.items.len(), 2);
    assert_eq!(multipart_delimited.common_prefixes, vec!["m/folder/"]);
}

#[allow(clippy::too_many_arguments)]
async fn create_object_version(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_id: ObjectId,
    key: &str,
    external_version_id: &str,
    generation: u64,
    is_null_version: bool,
    created_at: OffsetDateTime,
) {
    let object = S3Object::new(object_id, application_id, bucket_id, key, created_at)
        .expect("logical object");
    let version_id = ObjectVersionId::new();
    let payload = StoredObjectVersion::new(
        "filesystem",
        format!("listing-contract/{version_id}"),
        None,
        None,
        EntityTag::new(format!("etag-{version_id}")).expect("etag"),
        1,
        Some("application/octet-stream".to_owned()),
        json!({}),
        Some(Checksum::sha256_hex("0".repeat(64)).expect("checksum")),
    )
    .expect("payload");
    let version = ObjectVersion::new_object(
        version_id,
        object_id,
        application_id,
        bucket_id,
        S3VersionId::new(external_version_id).expect("version id"),
        generation,
        is_null_version,
        ObjectVersionState::Committed,
        payload,
        None,
        false,
        "listing-contract",
        SourceProtocol::S3,
        created_at,
    )
    .expect("object version");
    repository
        .create_s3_object_with_version(object, version, created_at)
        .await
        .expect("persist object version");
}

async fn insert_tenant(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    bucket_id: BucketId,
    now: OffsetDateTime,
) {
    let user_id = UserId::new();
    sqlx::query(
        "INSERT INTO users (id, email_normalized, password_hash, created_at, updated_at) \
         VALUES ($1, $2, 'listing-contract-hash', $3, $3)",
    )
    .bind(user_id.as_uuid())
    .bind(format!("{user_id}@listing.invalid"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert listing user");
    sqlx::query(
        "INSERT INTO applications (id, user_id, name, app_id, quota_bytes, created_at, updated_at) \
         VALUES ($1, $2, 'listing-contract', $3, 1000, $4, $4)",
    )
    .bind(application_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(format!("app_{application_id}"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert listing application");
    sqlx::query(
        "INSERT INTO buckets (id, application_id, name, visibility, allowed_mime_types, \
         created_at, updated_at) VALUES ($1, $2, 'listing-contract', 'private', $3, $4, $4)",
    )
    .bind(bucket_id.as_uuid())
    .bind(application_id.as_uuid())
    .bind(Json(json!([])))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert listing bucket");
}
