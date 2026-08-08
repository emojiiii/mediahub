use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{
    AbortStagedPutRequest, BeginPutObjectRequest, CompletePutObjectReceipt,
    CompletePutObjectRequest, CompletedS3MultipartPart, FixedClock, InMemoryObjectStore,
    NewS3MultipartPart, NewS3MultipartUpload, ObjectStore, PrepareClaimedUploadCommitRequest,
    RepositoryError, S3BucketRepository, S3LifecycleBatchCursor, S3LifecycleService,
    S3MultipartCompletionClaim, S3MultipartCompletionRelease, S3MultipartRepository,
    S3ObjectService, S3ObjectServiceError, S3UploadIntentRepository, StreamedObject,
};
use mediahub_core::{
    ApplicationId, BucketId, Checksum, EntityTag, OffsetDateTime, S3AbortIncompleteMultipartUpload,
    S3Bucket, S3Expiration, S3LifecycleConfiguration, S3LifecycleFilter, S3LifecycleRule,
    S3LifecycleRuleStatus, S3NoncurrentVersionExpiration, S3ObjectTagSet, SourceProtocol,
    UploadIntent, UserId, VersioningStatus,
};
use time::Duration;

type Service = S3ObjectService<
    PostgresRepository,
    PostgresRepository,
    PostgresRepository,
    InMemoryObjectStore,
    FixedClock,
>;

#[tokio::test]
async fn postgres_s3_application_quota_vertical_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect S3 quota database");
    repository
        .migrate()
        .await
        .expect("migrate S3 quota database");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated S3 quota database");

    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("whole-second timestamp");
    let application_id = ApplicationId::new();
    insert_application(&repository, application_id, 128, now).await;
    let enabled = create_bucket(
        &repository,
        application_id,
        "quota-enabled",
        VersioningStatus::Enabled,
        now,
    )
    .await;
    let suspended = create_bucket(
        &repository,
        application_id,
        "quota-suspended",
        VersioningStatus::Suspended,
        now,
    )
    .await;
    let unversioned = create_bucket(
        &repository,
        application_id,
        "quota-unversioned",
        VersioningStatus::Unversioned,
        now,
    )
    .await;
    let store = InMemoryObjectStore::default();

    let service = service(&repository, &store, now);
    let reservation_probe = service
        .begin_put(&begin_request(
            application_id,
            enabled.name(),
            "objects/reservation-probe.bin",
            8,
            None,
        ))
        .await
        .expect("reserve regular PutObject");
    assert_eq!(quota(&repository, application_id).await, (0, 8));
    assert_eq!(
        repository
            .create_upload_intent(&reservation_probe.intent)
            .await,
        Err(RepositoryError::Conflict)
    );
    assert_eq!(quota(&repository, application_id).await, (0, 8));
    service
        .abort_staged_put(&AbortStagedPutRequest {
            application_id,
            intent_id: reservation_probe.intent.id(),
        })
        .await
        .expect("release reservation probe");
    assert_eq!(quota(&repository, application_id).await, (0, 0));

    let (first, first_request, first_intent) = begin_stage_complete(
        &service,
        &store,
        application_id,
        enabled.name(),
        "objects/main.bin",
        b"prismark",
    )
    .await;
    assert_eq!(quota(&repository, application_id).await, (8, 0));
    service
        .complete_put(&first_request)
        .await
        .expect("committed PutObject replay");
    assert_eq!(quota(&repository, application_id).await, (8, 0));

    let duplicate = repository.create_upload_intent(&first_intent).await;
    assert_eq!(duplicate, Err(RepositoryError::Conflict));
    assert_eq!(quota(&repository, application_id).await, (8, 0));

    let (_second, _, _) = begin_stage_complete(
        &service,
        &store,
        application_id,
        enabled.name(),
        "objects/main.bin",
        b"prismark",
    )
    .await;
    let (_copy, _, _) = begin_stage_complete(
        &service,
        &store,
        application_id,
        enabled.name(),
        "objects/copy-of-main.bin",
        b"prismark",
    )
    .await;
    assert_eq!(quota(&repository, application_id).await, (24, 0));

    set_quota(&repository, application_id, 24).await;
    let exceeded = service
        .begin_put(&begin_request(
            application_id,
            enabled.name(),
            "objects/quota-exceeded.bin",
            1,
            None,
        ))
        .await;
    assert!(matches!(
        exceeded,
        Err(S3ObjectServiceError::Repository(
            RepositoryError::QuotaExceeded
        ))
    ));
    assert_eq!(quota(&repository, application_id).await, (24, 0));
    set_quota(&repository, application_id, 128).await;

    let (_first_null, _, _) = begin_stage_complete(
        &service,
        &store,
        application_id,
        suspended.name(),
        "objects/null.bin",
        b"prismark",
    )
    .await;
    assert_eq!(quota(&repository, application_id).await, (32, 0));
    let (_second_null, _, _) = begin_stage_complete(
        &service,
        &store,
        application_id,
        suspended.name(),
        "objects/null.bin",
        b"retry",
    )
    .await;
    assert_eq!(quota(&repository, application_id).await, (29, 0));

    service
        .delete(&mediahub_app::DeleteObjectRequest {
            application_id,
            bucket_name: enabled.name().into(),
            object_key: "objects/main.bin".into(),
            version_id: Some(first.version.external_version_id().clone()),
            bypass_governance: false,
            deleted_by: "quota-contract".into(),
        })
        .await
        .expect("delete exact noncurrent data version");
    assert_eq!(quota(&repository, application_id).await, (21, 0));
    let replay = service
        .delete(&mediahub_app::DeleteObjectRequest {
            application_id,
            bucket_name: enabled.name().into(),
            object_key: "objects/main.bin".into(),
            version_id: Some(first.version.external_version_id().clone()),
            bypass_governance: false,
            deleted_by: "quota-contract".into(),
        })
        .await;
    assert!(matches!(replay, Err(S3ObjectServiceError::VersionNotFound)));
    assert_eq!(quota(&repository, application_id).await, (21, 0));

    let marker = service
        .delete(&mediahub_app::DeleteObjectRequest {
            application_id,
            bucket_name: enabled.name().into(),
            object_key: "objects/main.bin".into(),
            version_id: None,
            bypass_governance: false,
            deleted_by: "quota-contract".into(),
        })
        .await
        .expect("create enabled delete marker");
    assert_eq!(quota(&repository, application_id).await, (21, 0));
    service
        .delete(&mediahub_app::DeleteObjectRequest {
            application_id,
            bucket_name: enabled.name().into(),
            object_key: "objects/main.bin".into(),
            version_id: marker.version_id,
            bypass_governance: false,
            deleted_by: "quota-contract".into(),
        })
        .await
        .expect("delete exact marker without releasing bytes");
    assert_eq!(quota(&repository, application_id).await, (21, 0));

    service
        .delete(&mediahub_app::DeleteObjectRequest {
            application_id,
            bucket_name: suspended.name().into(),
            object_key: "objects/null.bin".into(),
            version_id: None,
            bypass_governance: false,
            deleted_by: "quota-contract".into(),
        })
        .await
        .expect("suspended null replacement delete");
    assert_eq!(quota(&repository, application_id).await, (16, 0));

    let (_unversioned_data, _, _) = begin_stage_complete(
        &service,
        &store,
        application_id,
        unversioned.name(),
        "objects/unversioned.bin",
        b"prismark",
    )
    .await;
    assert_eq!(quota(&repository, application_id).await, (24, 0));
    service
        .delete(&mediahub_app::DeleteObjectRequest {
            application_id,
            bucket_name: unversioned.name().into(),
            object_key: "objects/unversioned.bin".into(),
            version_id: None,
            bypass_governance: false,
            deleted_by: "quota-contract".into(),
        })
        .await
        .expect("unversioned permanent delete");
    assert_eq!(quota(&repository, application_id).await, (16, 0));

    let abort_intent = service
        .begin_put(&begin_request(
            application_id,
            enabled.name(),
            "objects/abort.bin",
            8,
            None,
        ))
        .await
        .expect("reserve aborted PutObject");
    assert_eq!(quota(&repository, application_id).await, (16, 8));
    let abort = AbortStagedPutRequest {
        application_id,
        intent_id: abort_intent.intent.id(),
    };
    service
        .abort_staged_put(&abort)
        .await
        .expect("abort PutObject");
    service
        .abort_staged_put(&abort)
        .await
        .expect("replay PutObject abort");
    assert_eq!(quota(&repository, application_id).await, (16, 0));

    let expiry = now + Duration::seconds(2);
    service
        .begin_put(&begin_request(
            application_id,
            enabled.name(),
            "objects/expire.bin",
            8,
            Some(now + Duration::seconds(1)),
        ))
        .await
        .expect("reserve expiring PutObject");
    assert_eq!(
        repository
            .expire_upload_intents(expiry, 100, 5)
            .await
            .expect("expire PutObject intent"),
        1
    );
    assert_eq!(
        repository
            .expire_upload_intents(expiry, 100, 5)
            .await
            .expect("replay PutObject expiry"),
        0
    );
    assert_eq!(quota(&repository, application_id).await, (16, 0));

    lifecycle_releases_only_permanently_removed_data(
        &repository,
        &store,
        application_id,
        &enabled,
        now,
    )
    .await;
    assert_eq!(quota(&repository, application_id).await, (16, 0));

    let attached = attach_multipart(
        &repository,
        &store,
        AttachMultipartInput {
            application_id,
            bucket: &enabled,
            upload_id: "multipart-complete",
            object_key: "objects/multipart-complete.bin",
            now,
            expires_at: now + Duration::hours(2),
        },
    )
    .await;
    assert_eq!(quota(&repository, application_id).await, (16, 8));
    attached
        .service
        .promote_claimed_upload_intent(&attached.intent, &attached.token)
        .await
        .expect("promote multipart payload");
    let prepared = attached
        .service
        .prepare_claimed_upload_commit(PrepareClaimedUploadCommitRequest {
            intent: &attached.intent,
            lease_token: &attached.token,
            entity_tag: &attached.entity_tag,
            checksum: &attached.checksum,
            size_bytes: 8,
            created_by: "quota-contract",
            source_protocol: SourceProtocol::S3,
        })
        .await
        .expect("prepare multipart object version");
    let multipart_commit = prepared.commit.clone();
    repository
        .commit_multipart_object_version(
            "multipart-complete",
            &attached.token,
            prepared.commit,
            &attached.entity_tag,
            &attached.checksum,
        )
        .await
        .expect("commit multipart object version");
    assert_eq!(quota(&repository, application_id).await, (24, 0));
    repository
        .commit_multipart_object_version(
            "multipart-complete",
            &attached.token,
            multipart_commit,
            &attached.entity_tag,
            &attached.checksum,
        )
        .await
        .expect("replay completed multipart");
    assert_eq!(quota(&repository, application_id).await, (24, 0));

    let released = attach_multipart(
        &repository,
        &store,
        AttachMultipartInput {
            application_id,
            bucket: &enabled,
            upload_id: "multipart-release",
            object_key: "objects/multipart-release.bin",
            now,
            expires_at: now + Duration::hours(4),
        },
    )
    .await;
    assert!(matches!(
        repository
            .release_multipart_completion(
                "multipart-release",
                &released.token,
                now + Duration::minutes(1),
            )
            .await
            .expect("release multipart completion"),
        S3MultipartCompletionRelease::Released(_)
    ));
    assert_eq!(quota(&repository, application_id).await, (24, 0));

    let expiring_multipart = attach_multipart(
        &repository,
        &store,
        AttachMultipartInput {
            application_id,
            bucket: &enabled,
            upload_id: "multipart-expiry",
            object_key: "objects/multipart-expiry.bin",
            now,
            expires_at: now + Duration::hours(2),
        },
    )
    .await;
    assert_eq!(quota(&repository, application_id).await, (24, 8));
    assert_eq!(
        repository
            .expire_multipart_uploads(now + Duration::hours(3), 100)
            .await
            .expect("expire attached multipart")
            .len(),
        1
    );
    assert_eq!(quota(&repository, application_id).await, (24, 0));
    assert_eq!(
        repository
            .find_upload_intent(expiring_multipart.intent.id())
            .await
            .expect("read expired attached intent")
            .expect("attached intent")
            .state(),
        mediahub_core::UploadIntentState::Aborted
    );

    assert_s3_quota_matches_metadata(&repository, application_id).await;
}

struct AttachedMultipart {
    service: Service,
    intent: UploadIntent,
    token: String,
    entity_tag: EntityTag,
    checksum: Checksum,
}

struct AttachMultipartInput<'a> {
    application_id: ApplicationId,
    bucket: &'a S3Bucket,
    upload_id: &'a str,
    object_key: &'a str,
    now: OffsetDateTime,
    expires_at: OffsetDateTime,
}

async fn attach_multipart(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    input: AttachMultipartInput<'_>,
) -> AttachedMultipart {
    let AttachMultipartInput {
        application_id,
        bucket,
        upload_id,
        object_key,
        now,
        expires_at,
    } = input;
    repository
        .create_multipart_upload(NewS3MultipartUpload {
            upload_id: upload_id.into(),
            application_id,
            bucket_id: bucket.id(),
            object_key: object_key.into(),
            content_type: "application/octet-stream".into(),
            user_metadata: serde_json::json!({}),
            object_tags: S3ObjectTagSet::empty(),
            storage_backend: store.backend_name().into(),
            expires_at,
            created_at: now,
        })
        .await
        .expect("create multipart upload");
    let part_key = format!("multipart/{upload_id}/1");
    store
        .put_temporary(&part_key, b"prismark", "application/octet-stream")
        .await
        .expect("stage multipart part");
    repository
        .put_multipart_part(
            upload_id,
            NewS3MultipartPart {
                part_number: 1,
                size: 8,
                sha256: "83704837d7a78682ab7973e48edfeff3a8a222c63faa185ea4ce860220773116".into(),
                md5: "c89d43adb247379adc03e0f63806210a".into(),
                etag: "c89d43adb247379adc03e0f63806210a".into(),
                storage_key: part_key.clone(),
            },
            1024,
            now,
        )
        .await
        .expect("persist multipart part");
    let token = format!("complete-{upload_id}");
    let lease_until = now + Duration::minutes(30);
    let manifest = match repository
        .claim_multipart_completion(
            upload_id,
            &[CompletedS3MultipartPart {
                part_number: 1,
                etag: "c89d43adb247379adc03e0f63806210a".into(),
            }],
            &token,
            lease_until,
            now,
        )
        .await
        .expect("claim multipart completion")
    {
        S3MultipartCompletionClaim::Claimed(manifest) => manifest,
        outcome => panic!("unexpected multipart claim: {outcome:?}"),
    };
    let service = service(repository, store, now);
    let begun = service
        .begin_put(&begin_request(
            application_id,
            bucket.name(),
            object_key,
            manifest.total_size,
            Some(expires_at),
        ))
        .await
        .expect("reserve multipart total size");
    let composed = store
        .compose_temporary(
            begun.intent.temporary_storage_key(),
            &[part_key],
            "application/octet-stream",
        )
        .await
        .expect("compose multipart payload");
    let entity_tag = EntityTag::new("093406b8f626fe65f3ed47f1cdea9681-1").expect("multipart ETag");
    let checksum = Checksum::sha256_hex(&composed.sha256).expect("multipart checksum");
    repository
        .complete_upload_intent_staging(
            begun.intent.id(),
            &entity_tag,
            &checksum,
            composed.size,
            now,
        )
        .await
        .expect("freeze multipart stream facts");
    let intent = repository
        .claim_upload_intent(begun.intent.id(), &token, lease_until, now)
        .await
        .expect("claim multipart intent");
    repository
        .attach_multipart_upload_intent(upload_id, &token, intent.id(), now)
        .await
        .expect("attach multipart intent");
    AttachedMultipart {
        service,
        intent,
        token,
        entity_tag,
        checksum,
    }
}

async fn lifecycle_releases_only_permanently_removed_data(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    now: OffsetDateTime,
) {
    let old = now - Duration::days(10);
    begin_stage_complete(
        &service(repository, store, old),
        store,
        application_id,
        bucket.name(),
        "lifecycle/old.bin",
        b"prismark",
    )
    .await;
    assert_eq!(quota(repository, application_id).await, (24, 0));
    let rules = vec![
        S3LifecycleRule {
            id: Some("expire-data".into()),
            status: S3LifecycleRuleStatus::Enabled,
            filter: S3LifecycleFilter::Prefix("lifecycle/".into()),
            expiration: Some(S3Expiration::Days(1)),
            noncurrent_version_expiration: Some(S3NoncurrentVersionExpiration {
                noncurrent_days: 1,
            }),
            abort_incomplete_multipart_upload: Some(S3AbortIncompleteMultipartUpload {
                days_after_initiation: 1,
            }),
        },
        S3LifecycleRule {
            id: Some("remove-marker".into()),
            status: S3LifecycleRuleStatus::Enabled,
            filter: S3LifecycleFilter::Prefix("lifecycle/".into()),
            expiration: Some(S3Expiration::ExpiredObjectDeleteMarker),
            noncurrent_version_expiration: None,
            abort_incomplete_multipart_upload: None,
        },
    ];
    repository
        .replace_s3_bucket_lifecycle(
            application_id,
            bucket.name(),
            Some(S3LifecycleConfiguration::new(rules).expect("lifecycle rules")),
            now,
        )
        .await
        .expect("configure lifecycle");
    S3LifecycleService::new(repository.clone(), FixedClock::new(now))
        .run_batch(S3LifecycleBatchCursor::default(), 100)
        .await
        .expect("create lifecycle marker");
    assert_eq!(quota(repository, application_id).await, (24, 0));
    let later = now + Duration::days(2);
    S3LifecycleService::new(repository.clone(), FixedClock::new(later))
        .run_batch(S3LifecycleBatchCursor::default(), 100)
        .await
        .expect("permanently expire lifecycle data");
    assert_eq!(quota(repository, application_id).await, (16, 0));
    S3LifecycleService::new(repository.clone(), FixedClock::new(later))
        .run_batch(S3LifecycleBatchCursor::default(), 100)
        .await
        .expect("replay lifecycle batch");
    assert_eq!(quota(repository, application_id).await, (16, 0));
}

async fn begin_stage_complete(
    service: &Service,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket_name: &str,
    object_key: &str,
    content: &[u8],
) -> (
    CompletePutObjectReceipt,
    CompletePutObjectRequest,
    UploadIntent,
) {
    let begun = service
        .begin_put(&begin_request(
            application_id,
            bucket_name,
            object_key,
            content.len() as u64,
            None,
        ))
        .await
        .expect("begin S3 upload");
    store
        .put_temporary(
            begun.intent.temporary_storage_key(),
            content,
            "application/octet-stream",
        )
        .await
        .expect("stage S3 upload");
    let request = CompletePutObjectRequest {
        application_id,
        intent_id: begun.intent.id(),
        streamed: streamed(content),
        created_by: "quota-contract".into(),
        source_protocol: SourceProtocol::S3,
    };
    let receipt = service
        .complete_put(&request)
        .await
        .expect("complete S3 upload");
    (receipt, request, begun.intent)
}

fn begin_request(
    application_id: ApplicationId,
    bucket_name: &str,
    object_key: &str,
    expected_size_bytes: u64,
    expires_at: Option<OffsetDateTime>,
) -> BeginPutObjectRequest {
    BeginPutObjectRequest {
        application_id,
        bucket_name: bucket_name.into(),
        object_key: object_key.into(),
        expected_size_bytes,
        content_type: Some("application/octet-stream".into()),
        user_metadata: serde_json::json!({}),
        object_tags: S3ObjectTagSet::empty(),
        expires_at,
    }
}

fn streamed(content: &[u8]) -> StreamedObject {
    match content {
        b"prismark" => StreamedObject {
            size: 8,
            sha256: "83704837d7a78682ab7973e48edfeff3a8a222c63faa185ea4ce860220773116".into(),
            md5: "c89d43adb247379adc03e0f63806210a".into(),
        },
        b"retry" => StreamedObject {
            size: 5,
            sha256: "06b29bb318814108e94270528fe7994c096308b3692923723bf1ae6f98d50b4f".into(),
            md5: "165e6d21e0a2cc9ebb32ca05f90e0fa7".into(),
        },
        _ => panic!("missing S3 quota digest fixture"),
    }
}

fn service(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    now: OffsetDateTime,
) -> Service {
    S3ObjectService::new(
        repository.clone(),
        repository.clone(),
        repository.clone(),
        store.clone(),
        FixedClock::new(now),
    )
}

async fn create_bucket(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    name: &str,
    versioning: VersioningStatus,
    now: OffsetDateTime,
) -> S3Bucket {
    let bucket = S3Bucket::new(
        BucketId::new(),
        application_id,
        name,
        "us-east-1",
        false,
        None,
        now,
    )
    .expect("S3 quota bucket");
    repository
        .create_s3_bucket(&bucket)
        .await
        .expect("create S3 quota bucket");
    if versioning != VersioningStatus::Unversioned {
        repository
            .set_s3_bucket_versioning(application_id, name, VersioningStatus::Enabled, now)
            .await
            .expect("set S3 quota bucket versioning");
        if versioning == VersioningStatus::Suspended {
            repository
                .set_s3_bucket_versioning(application_id, name, VersioningStatus::Suspended, now)
                .await
                .expect("suspend S3 quota bucket versioning");
        }
    }
    repository
        .find_s3_bucket(application_id, name)
        .await
        .expect("read S3 quota bucket")
        .expect("S3 quota bucket exists")
}

async fn insert_application(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    quota_bytes: i64,
    now: OffsetDateTime,
) {
    let user_id = UserId::new();
    sqlx::query(
        "INSERT INTO users (id, email_normalized, password_hash, created_at, updated_at)
         VALUES ($1, $2, 'contract-hash', $3, $3)",
    )
    .bind(user_id.as_uuid())
    .bind(format!("s3-quota-{user_id}@contract.invalid"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert S3 quota user");
    sqlx::query(
        "INSERT INTO applications
            (id, user_id, name, app_id, quota_bytes, created_at, updated_at)
         VALUES ($1, $2, 'S3 Quota Contract', $3, $4, $5, $5)",
    )
    .bind(application_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(format!("s3-quota-{application_id}"))
    .bind(quota_bytes)
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert S3 quota application");
}

async fn set_quota(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    quota_bytes: i64,
) {
    sqlx::query("UPDATE applications SET quota_bytes = $1 WHERE id = $2")
        .bind(quota_bytes)
        .bind(application_id.as_uuid())
        .execute(repository.pool())
        .await
        .expect("set S3 quota");
}

async fn quota(repository: &PostgresRepository, application_id: ApplicationId) -> (i64, i64) {
    sqlx::query_as("SELECT used_bytes, reserved_bytes FROM applications WHERE id = $1")
        .bind(application_id.as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("read S3 quota")
}

async fn assert_s3_quota_matches_metadata(
    repository: &PostgresRepository,
    application_id: ApplicationId,
) {
    let used = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(size_bytes), 0)::BIGINT
         FROM object_versions
         WHERE application_id = $1 AND state = 'committed'
           AND superseded_at IS NULL AND NOT is_delete_marker",
    )
    .bind(application_id.as_uuid())
    .fetch_one(repository.pool())
    .await
    .expect("sum S3 used bytes");
    let reserved = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(SUM(expected_size_bytes), 0)::BIGINT
         FROM s3_upload_intents
         WHERE application_id = $1 AND state IN ('staging', 'ready', 'committing')",
    )
    .bind(application_id.as_uuid())
    .fetch_one(repository.pool())
    .await
    .expect("sum S3 reserved bytes");
    assert_eq!(quota(repository, application_id).await, (used, reserved));
}
