use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{S3BucketCorsRepository, S3BucketTaggingRepository};
use mediahub_core::{
    ApplicationId, BucketId, OffsetDateTime, S3BucketTagSet, S3CorsConfiguration, S3CorsMethod,
    S3CorsRule, S3ObjectTag, UserId,
};
use time::Duration;

#[tokio::test]
async fn postgres_s3_bucket_cors_and_tagging_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect Bucket configuration database");
    repository
        .migrate()
        .await
        .expect("migrate Bucket configuration database");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated Bucket configuration database");

    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("whole-second timestamp");
    let user_id = UserId::new();
    insert_user(&repository, user_id, now).await;
    let owner_application_id = ApplicationId::new();
    let other_application_id = ApplicationId::new();
    insert_application(&repository, user_id, owner_application_id, "owner", now).await;
    insert_application(&repository, user_id, other_application_id, "other", now).await;
    insert_bucket(
        &repository,
        owner_application_id,
        "configuration-contract",
        now,
    )
    .await;

    let initial_cors = repository
        .get_s3_bucket_cors(owner_application_id, "configuration-contract")
        .await
        .expect("read initial CORS")
        .expect("owner bucket");
    assert_eq!(initial_cors.revision, 0);
    assert!(initial_cors.configuration.is_none());
    assert!(initial_cors.updated_at.is_none());

    let cors = S3CorsConfiguration::new(vec![
        S3CorsRule::new(
            Some("browser-upload".into()),
            vec![S3CorsMethod::Get, S3CorsMethod::Put],
            vec!["https://app.example.com".into()],
            vec!["authorization".into(), "x-amz-*".into()],
            vec!["etag".into(), "x-amz-version-id".into()],
            Some(600),
        )
        .expect("CORS rule"),
    ])
    .expect("CORS configuration");
    let stored_cors = repository
        .put_s3_bucket_cors(
            owner_application_id,
            "configuration-contract",
            cors.clone(),
            now + Duration::seconds(1),
        )
        .await
        .expect("put CORS")
        .expect("owner bucket");
    assert_eq!(stored_cors.revision, 1);
    assert_eq!(stored_cors.configuration, Some(cors));
    assert!(
        repository
            .put_s3_bucket_cors(
                other_application_id,
                "configuration-contract",
                S3CorsConfiguration::new(vec![
                    S3CorsRule::new(
                        None,
                        vec![S3CorsMethod::Get],
                        vec!["*".into()],
                        vec![],
                        vec![],
                        None,
                    )
                    .expect("cross-tenant rule"),
                ])
                .expect("cross-tenant CORS"),
                now + Duration::seconds(2),
            )
            .await
            .expect("tenant-fenced CORS put")
            .is_none()
    );

    let tags = S3BucketTagSet::new(
        (0..50)
            .map(|index| {
                S3ObjectTag::new(format!("bucket-key-{index}"), "value").expect("Bucket tag")
            })
            .collect(),
    )
    .expect("50 Bucket tags");
    let stored_tags = repository
        .put_s3_bucket_tagging(
            owner_application_id,
            "configuration-contract",
            tags.clone(),
            now + Duration::seconds(3),
        )
        .await
        .expect("put Bucket tags")
        .expect("owner bucket");
    assert_eq!(stored_tags.revision, 1);
    assert_eq!(stored_tags.tags, Some(tags));
    assert!(
        repository
            .delete_s3_bucket_tagging(
                other_application_id,
                "configuration-contract",
                now + Duration::seconds(4),
            )
            .await
            .expect("tenant-fenced tag delete")
            .is_none()
    );

    let deleted_tags = repository
        .delete_s3_bucket_tagging(
            owner_application_id,
            "configuration-contract",
            now + Duration::seconds(5),
        )
        .await
        .expect("delete Bucket tags")
        .expect("owner bucket");
    assert_eq!(deleted_tags.revision, 2);
    assert!(deleted_tags.tags.is_none());
    assert!(deleted_tags.updated_at.is_some());

    let deleted_cors = repository
        .delete_s3_bucket_cors(
            owner_application_id,
            "configuration-contract",
            now + Duration::seconds(6),
        )
        .await
        .expect("delete CORS")
        .expect("owner bucket");
    assert_eq!(deleted_cors.revision, 2);
    assert!(deleted_cors.configuration.is_none());
    assert!(deleted_cors.updated_at.is_some());
}

async fn insert_user(repository: &PostgresRepository, user_id: UserId, now: OffsetDateTime) {
    sqlx::query(
        "INSERT INTO users (id, email_normalized, password_hash, created_at, updated_at)
         VALUES ($1, $2, 'bucket-configuration-contract-hash', $3, $3)",
    )
    .bind(user_id.as_uuid())
    .bind(format!("{user_id}@bucket-configuration.invalid"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert Bucket configuration user");
}

async fn insert_application(
    repository: &PostgresRepository,
    user_id: UserId,
    application_id: ApplicationId,
    suffix: &str,
    now: OffsetDateTime,
) {
    sqlx::query(
        "INSERT INTO applications (id, user_id, name, app_id, quota_bytes, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 1000, $5, $5)",
    )
    .bind(application_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(format!("bucket-configuration-{suffix}"))
    .bind(format!("app_bucket_configuration_{suffix}"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert Bucket configuration application");
}

async fn insert_bucket(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    name: &str,
    now: OffsetDateTime,
) {
    sqlx::query(
        "INSERT INTO buckets (id, application_id, name, visibility, created_at, updated_at)
         VALUES ($1, $2, $3, 'private', $4, $4)",
    )
    .bind(BucketId::new().as_uuid())
    .bind(application_id.as_uuid())
    .bind(name)
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert Bucket configuration bucket");
}
