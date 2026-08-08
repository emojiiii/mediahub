use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{RepositoryError, S3BucketRepository};
use mediahub_core::{
    ApplicationId, BucketId, DefaultRetention, DefaultRetentionPeriod, OffsetDateTime,
    RetentionMode, S3Bucket, UserId, VersioningStatus,
};
use sqlx::Row;
use time::Duration;

#[tokio::test]
async fn postgres_s3_object_lock_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect Object Lock database");
    repository
        .migrate()
        .await
        .expect("migrate Object Lock database");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated Object Lock database");

    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("whole-second contract timestamp");
    let application_id = ApplicationId::new();
    insert_application(&repository, application_id, now).await;

    let created_locked = S3Bucket::new(
        BucketId::new(),
        application_id,
        "created-locked",
        "us-east-1",
        true,
        None,
        now,
    )
    .expect("Object Lock bucket");
    repository
        .create_s3_bucket(&created_locked)
        .await
        .expect("atomic Object Lock bucket creation");
    let created_configuration = repository
        .get_s3_bucket_configuration(application_id, created_locked.name())
        .await
        .expect("read created configuration")
        .expect("created bucket");
    assert!(created_configuration.object_lock_enabled());
    assert_eq!(
        created_configuration.versioning_status(),
        VersioningStatus::Enabled
    );
    assert_eq!(created_configuration.default_retention(), None);
    assert_eq!(created_configuration.revision(), 1);
    assert_eq!(created_configuration.updated_at(), now);

    let initially_unlocked = S3Bucket::new(
        BucketId::new(),
        application_id,
        "enable-later",
        "us-east-1",
        false,
        None,
        now,
    )
    .expect("ordinary bucket");
    repository
        .create_s3_bucket(&initially_unlocked)
        .await
        .expect("create ordinary bucket");
    let retention =
        DefaultRetention::new(RetentionMode::Governance, DefaultRetentionPeriod::Days(30))
            .expect("valid default retention");
    let enabled_at = now + Duration::seconds(1);
    let enabled = repository
        .replace_s3_bucket_object_lock(
            application_id,
            initially_unlocked.name(),
            Some(retention),
            enabled_at,
        )
        .await
        .expect("enable Object Lock transactionally");
    assert!(enabled.object_lock_enabled());
    assert_eq!(enabled.versioning_status(), VersioningStatus::Enabled);
    assert_eq!(enabled.default_retention(), Some(retention));
    assert_eq!(enabled.revision(), 2);
    assert_eq!(enabled.updated_at(), enabled_at);

    let idempotent_at = enabled_at + Duration::seconds(1);
    let unchanged = repository
        .replace_s3_bucket_object_lock(
            application_id,
            initially_unlocked.name(),
            Some(retention),
            idempotent_at,
        )
        .await
        .expect("idempotent Object Lock replacement");
    assert_eq!(unchanged.revision(), 2);
    assert_eq!(unchanged.updated_at(), enabled_at);

    let cleared_at = idempotent_at + Duration::seconds(1);
    let cleared = repository
        .replace_s3_bucket_object_lock(application_id, initially_unlocked.name(), None, cleared_at)
        .await
        .expect("clear only the default retention");
    assert!(cleared.object_lock_enabled());
    assert_eq!(cleared.versioning_status(), VersioningStatus::Enabled);
    assert_eq!(cleared.default_retention(), None);
    assert_eq!(cleared.revision(), 3);
    assert_eq!(cleared.updated_at(), cleared_at);

    assert!(matches!(
        repository
            .set_s3_bucket_versioning(
                application_id,
                initially_unlocked.name(),
                VersioningStatus::Suspended,
                cleared_at + Duration::seconds(1),
            )
            .await,
        Err(RepositoryError::Invariant(_))
    ));
    assert_eq!(
        repository
            .replace_s3_bucket_object_lock(application_id, "missing-bucket", None, cleared_at,)
            .await,
        Err(RepositoryError::NotFound)
    );

    let direct_disable = sqlx::query(
        "UPDATE buckets SET object_lock_enabled = FALSE WHERE application_id = $1 AND name = $2",
    )
    .bind(application_id.as_uuid())
    .bind(initially_unlocked.name())
    .execute(repository.pool())
    .await;
    assert!(
        direct_disable.is_err(),
        "database must reject Object Lock disablement"
    );

    let persisted = sqlx::query(
        "SELECT versioning_status, object_lock_enabled, default_retention,
                s3_configuration_revision, updated_at
         FROM buckets WHERE application_id = $1 AND name = $2",
    )
    .bind(application_id.as_uuid())
    .bind(initially_unlocked.name())
    .fetch_one(repository.pool())
    .await
    .expect("read persisted Object Lock state");
    assert_eq!(
        persisted
            .try_get::<String, _>("versioning_status")
            .expect("versioning status"),
        "enabled"
    );
    assert!(
        persisted
            .try_get::<bool, _>("object_lock_enabled")
            .expect("Object Lock flag")
    );
    assert!(
        persisted
            .try_get::<Option<serde_json::Value>, _>("default_retention")
            .expect("default retention JSON")
            .is_none()
    );
    assert_eq!(
        persisted
            .try_get::<i64, _>("s3_configuration_revision")
            .expect("configuration revision"),
        3
    );
    assert_eq!(
        persisted
            .try_get::<OffsetDateTime, _>("updated_at")
            .expect("configuration updated_at"),
        cleared_at
    );
}

async fn insert_application(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    now: OffsetDateTime,
) {
    let user_id = UserId::new();
    sqlx::query(
        "INSERT INTO users (id, email_normalized, password_hash, created_at, updated_at)
         VALUES ($1, $2, 'contract-hash', $3, $3)",
    )
    .bind(user_id.as_uuid())
    .bind(format!("object-lock-{user_id}@contract.invalid"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert Object Lock contract user");
    sqlx::query(
        "INSERT INTO applications
            (id, user_id, name, app_id, quota_bytes, created_at, updated_at)
         VALUES ($1, $2, 'Object Lock Contract', $3, 1073741824, $4, $4)",
    )
    .bind(application_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(format!("object-lock-{application_id}"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert Object Lock contract application");
}
