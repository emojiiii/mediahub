use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{
    BeginPutObjectRequest, CompletePutObjectRequest, DeleteObjectRequest, FixedClock,
    InMemoryObjectStore, ObjectStore, PutObjectTaggingRequest, S3BucketRepository,
    S3ObjectRepository, S3ObjectRequest, S3ObjectService, S3ObjectServiceError, StreamedObject,
};
use mediahub_core::{
    ApplicationId, BucketId, OffsetDateTime, S3Bucket, S3ObjectTag, S3ObjectTagSet, SourceProtocol,
    UserId,
};

#[tokio::test]
async fn postgres_s3_object_tagging_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect Object Tagging database");
    repository
        .migrate()
        .await
        .expect("migrate Object Tagging database");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated Object Tagging database");

    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("whole-second timestamp");
    let application_id = ApplicationId::new();
    insert_application(&repository, application_id, "primary", now).await;
    let bucket = S3Bucket::new(
        BucketId::new(),
        application_id,
        "tagged-assets",
        "us-east-1",
        false,
        None,
        now,
    )
    .expect("versioned bucket");
    repository
        .create_s3_bucket(&bucket)
        .await
        .expect("create bucket");
    repository
        .set_s3_bucket_versioning(
            application_id,
            bucket.name(),
            mediahub_core::VersioningStatus::Enabled,
            now,
        )
        .await
        .expect("enable versioning");

    let store = InMemoryObjectStore::default();
    let service = S3ObjectService::new(
        repository.clone(),
        repository.clone(),
        repository.clone(),
        store.clone(),
        FixedClock::new(now),
    );
    let first_tags = tag_set(&[("stage", "draft"), ("项目", "万象仓")]);
    let first = put_version(
        &service,
        &store,
        application_id,
        bucket.name(),
        first_tags.clone(),
        "first",
    )
    .await;
    let second_tags = tag_set(&[("stage", "published")]);
    let second = put_version(
        &service,
        &store,
        application_id,
        bucket.name(),
        second_tags.clone(),
        "second",
    )
    .await;

    let current_request = S3ObjectRequest {
        application_id,
        bucket_name: bucket.name().to_owned(),
        object_key: "docs/tagged.txt".into(),
        version_id: None,
    };
    let current = service
        .get_object_tags(&current_request)
        .await
        .expect("get current tags");
    assert_eq!(current.version.id(), second.version.id());
    assert_eq!(current.tags, second_tags);
    let exact_first = service
        .get_object_tags(&S3ObjectRequest {
            version_id: Some(first.version.external_version_id().clone()),
            ..current_request.clone()
        })
        .await
        .expect("get exact first-version tags");
    assert_eq!(exact_first.tags, first_tags);

    let replacement = tag_set(&[("stage", "archived"), ("owner", "media")]);
    let replaced = service
        .put_object_tags(&PutObjectTaggingRequest {
            object: S3ObjectRequest {
                version_id: Some(first.version.external_version_id().clone()),
                ..current_request.clone()
            },
            tags: replacement.clone(),
        })
        .await
        .expect("replace first-version tags");
    assert_eq!(replaced.tags, replacement);
    assert_eq!(
        service
            .get_object_tags(&current_request)
            .await
            .expect("current remains isolated")
            .tags,
        second_tags
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM object_version_tags WHERE object_version_id = $1",
        )
        .bind(first.version.id().as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("count first-version tags"),
        2
    );
    let persisted_metadata: serde_json::Value =
        sqlx::query_scalar("SELECT user_metadata FROM object_versions WHERE id = $1")
            .bind(first.version.id().as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("read user metadata");
    assert_eq!(
        persisted_metadata,
        serde_json::json!({"origin": "contract"})
    );

    let cleared = service
        .delete_object_tags(&current_request)
        .await
        .expect("clear current tags");
    assert!(cleared.tags.is_empty());

    let other_application = ApplicationId::new();
    insert_application(&repository, other_application, "other", now).await;
    let other_bucket = S3Bucket::new(
        BucketId::new(),
        other_application,
        bucket.name(),
        "us-east-1",
        false,
        None,
        now,
    )
    .expect("other bucket");
    repository
        .create_s3_bucket(&other_bucket)
        .await
        .expect("create other bucket");
    assert!(
        repository
            .find_s3_object_version_tags(other_application, first.version.id())
            .await
            .expect("cross-application tag lookup")
            .is_none()
    );
    assert!(matches!(
        service
            .get_object_tags(&S3ObjectRequest {
                application_id: other_application,
                bucket_name: bucket.name().to_owned(),
                object_key: "docs/tagged.txt".into(),
                version_id: Some(first.version.external_version_id().clone()),
            })
            .await,
        Err(S3ObjectServiceError::VersionNotFound)
    ));

    service
        .delete(&DeleteObjectRequest {
            application_id,
            bucket_name: bucket.name().to_owned(),
            object_key: "docs/tagged.txt".into(),
            version_id: None,
            bypass_governance: false,
            deleted_by: "tagging-contract".into(),
        })
        .await
        .expect("create delete marker");
    assert!(matches!(
        service.get_object_tags(&current_request).await,
        Err(S3ObjectServiceError::DeleteMarker {
            is_current: true,
            ..
        })
    ));
}

async fn put_version(
    service: &S3ObjectService<
        PostgresRepository,
        PostgresRepository,
        PostgresRepository,
        InMemoryObjectStore,
        FixedClock,
    >,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket_name: &str,
    tags: S3ObjectTagSet,
    creator: &str,
) -> mediahub_app::CompletePutObjectReceipt {
    let begun = service
        .begin_put(&BeginPutObjectRequest {
            application_id,
            bucket_name: bucket_name.to_owned(),
            object_key: "docs/tagged.txt".into(),
            expected_size_bytes: 8,
            content_type: Some("text/plain".into()),
            user_metadata: serde_json::json!({"origin": "contract"}),
            object_tags: tags,
            expires_at: None,
        })
        .await
        .expect("begin tagged upload");
    store
        .put_temporary(
            begun.intent.temporary_storage_key(),
            b"prismark",
            "text/plain",
        )
        .await
        .expect("stage tagged bytes");
    service
        .complete_put(&CompletePutObjectRequest {
            application_id,
            intent_id: begun.intent.id(),
            streamed: StreamedObject {
                size: 8,
                sha256: "83704837d7a78682ab7973e48edfeff3a8a222c63faa185ea4ce860220773116".into(),
                md5: "c89d43adb247379adc03e0f63806210a".into(),
            },
            created_by: creator.into(),
            source_protocol: SourceProtocol::S3,
        })
        .await
        .expect("complete tagged upload")
}

fn tag_set(values: &[(&str, &str)]) -> S3ObjectTagSet {
    S3ObjectTagSet::new(
        values
            .iter()
            .map(|(key, value)| S3ObjectTag::new(*key, *value).expect("valid tag"))
            .collect(),
    )
    .expect("valid tag set")
}

async fn insert_application(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    suffix: &str,
    now: OffsetDateTime,
) {
    let user_id = UserId::new();
    sqlx::query(
        "INSERT INTO users (id, email_normalized, password_hash, created_at, updated_at)
         VALUES ($1, $2, 'contract-hash', $3, $3)",
    )
    .bind(user_id.as_uuid())
    .bind(format!("tagging-{suffix}-{user_id}@contract.invalid"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert contract user");
    sqlx::query(
        "INSERT INTO applications
            (id, user_id, name, app_id, quota_bytes, created_at, updated_at)
         VALUES ($1, $2, 'Object Tagging Contract', $3, 1073741824, $4, $4)",
    )
    .bind(application_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(format!("tagging-{suffix}-{application_id}"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert contract application");
}
