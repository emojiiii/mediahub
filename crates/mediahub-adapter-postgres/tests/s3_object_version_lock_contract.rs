use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{
    BeginPutObjectRequest, CompletePutObjectRequest, FixedClock, InMemoryObjectStore,
    NewS3ObjectLock, ObjectStore, PutObjectLegalHoldRequest, PutObjectRetentionRequest,
    S3BucketRepository, S3DeleteLockReason, S3ObjectRequest, S3ObjectService, S3ObjectServiceError,
    StreamedObject,
};
use mediahub_core::{
    ApplicationId, BucketId, DefaultRetention, DefaultRetentionPeriod, ObjectRetention,
    OffsetDateTime, RetentionMode, S3Bucket, SourceProtocol, UserId,
};
use time::Duration;

#[tokio::test]
async fn postgres_object_version_lock_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect ObjectVersion lock database");
    repository
        .migrate()
        .await
        .expect("migrate ObjectVersion lock database");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated ObjectVersion lock database");

    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("whole-second timestamp");
    let application_id = ApplicationId::new();
    insert_application(&repository, application_id, now).await;
    let bucket = S3Bucket::new(
        BucketId::new(),
        application_id,
        "locked-assets",
        "us-east-1",
        true,
        Some(
            DefaultRetention::new(RetentionMode::Governance, DefaultRetentionPeriod::Days(30))
                .expect("default retention"),
        ),
        now,
    )
    .expect("locked bucket");
    repository
        .create_s3_bucket(&bucket)
        .await
        .expect("create locked bucket");

    let store = InMemoryObjectStore::default();
    let service = S3ObjectService::new(
        repository.clone(),
        repository.clone(),
        repository.clone(),
        store.clone(),
        FixedClock::new(now),
    );
    let begun = service
        .begin_put(&BeginPutObjectRequest {
            application_id,
            bucket_name: bucket.name().to_owned(),
            object_key: "docs/locked.txt".into(),
            expected_size_bytes: 8,
            content_type: Some("text/plain".into()),
            user_metadata: serde_json::json!({}),
            object_tags: mediahub_core::S3ObjectTagSet::empty(),
            expires_at: None,
        })
        .await
        .expect("begin upload");
    store
        .put_temporary(
            begun.intent.temporary_storage_key(),
            b"prismark",
            "text/plain",
        )
        .await
        .expect("stage bytes");
    let completed = service
        .complete_put_with_object_lock(
            &CompletePutObjectRequest {
                application_id,
                intent_id: begun.intent.id(),
                streamed: StreamedObject {
                    size: 8,
                    sha256: "83704837d7a78682ab7973e48edfeff3a8a222c63faa185ea4ce860220773116"
                        .into(),
                    md5: "c89d43adb247379adc03e0f63806210a".into(),
                },
                created_by: "object-lock-contract".into(),
                source_protocol: SourceProtocol::S3,
            },
            NewS3ObjectLock {
                retention: None,
                legal_hold: Some(true),
            },
        )
        .await
        .expect("commit locked ObjectVersion");
    assert_eq!(
        completed.version.retention(),
        Some(ObjectRetention::new(
            RetentionMode::Governance,
            now + Duration::days(30),
        ))
    );
    assert!(completed.version.legal_hold());

    let object = S3ObjectRequest {
        application_id,
        bucket_name: bucket.name().to_owned(),
        object_key: "docs/locked.txt".into(),
        version_id: Some(completed.version.external_version_id().clone()),
    };
    assert!(matches!(
        service
            .put_object_retention(&PutObjectRetentionRequest {
                object: object.clone(),
                retention: ObjectRetention::new(
                    RetentionMode::Governance,
                    now + Duration::days(20),
                ),
                bypass_governance: false,
            })
            .await,
        Err(S3ObjectServiceError::RetentionUpdateLocked(
            S3DeleteLockReason::GovernanceRetention
        ))
    ));
    let compliance = service
        .put_object_retention(&PutObjectRetentionRequest {
            object: object.clone(),
            retention: ObjectRetention::new(RetentionMode::Compliance, now + Duration::days(40)),
            bypass_governance: false,
        })
        .await
        .expect("strengthen and extend retention");
    assert_eq!(
        compliance.retention().expect("retention").mode(),
        RetentionMode::Compliance
    );
    assert!(matches!(
        service
            .put_object_retention(&PutObjectRetentionRequest {
                object: object.clone(),
                retention: ObjectRetention::new(
                    RetentionMode::Compliance,
                    now + Duration::days(35),
                ),
                bypass_governance: true,
            })
            .await,
        Err(S3ObjectServiceError::RetentionUpdateLocked(
            S3DeleteLockReason::ComplianceRetention
        ))
    ));
    let released = service
        .put_object_legal_hold(&PutObjectLegalHoldRequest {
            object: object.clone(),
            legal_hold: false,
        })
        .await
        .expect("release legal hold");
    assert!(!released.legal_hold());

    let wrong_application = S3ObjectRequest {
        application_id: ApplicationId::new(),
        ..object
    };
    assert!(matches!(
        service.get_object_lock(&wrong_application).await,
        Err(S3ObjectServiceError::BucketNotFound)
    ));
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
    .bind(format!("version-lock-{user_id}@contract.invalid"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert contract user");
    sqlx::query(
        "INSERT INTO applications
            (id, user_id, name, app_id, quota_bytes, created_at, updated_at)
         VALUES ($1, $2, 'ObjectVersion Lock Contract', $3, 1073741824, $4, $4)",
    )
    .bind(application_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(format!("version-lock-{application_id}"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert contract application");
}
