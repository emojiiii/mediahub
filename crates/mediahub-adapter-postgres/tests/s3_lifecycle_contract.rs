use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{
    BeginPutObjectRequest, CompletePutObjectReceipt, CompletePutObjectRequest,
    ExecuteS3LifecycleCommand, FixedClock, InMemoryObjectStore, NewS3MultipartPart,
    NewS3MultipartUpload, ObjectStore, PutObjectRetentionRequest, S3BucketRepository,
    S3CurrentExpirationCandidate, S3LifecycleBatchCursor, S3LifecycleExecutionOutcome,
    S3LifecycleRepository, S3LifecycleService, S3LifecycleTarget, S3MultipartRepository,
    S3ObjectRequest, S3ObjectService, StreamedObject, lifecycle_action_time, lifecycle_days_cutoff,
};
use mediahub_core::{
    ApplicationId, BucketId, ObjectRetention, ObjectVersionId, OffsetDateTime, RetentionMode,
    S3AbortIncompleteMultipartUpload, S3Bucket, S3Expiration, S3LifecycleConfiguration,
    S3LifecycleFilter, S3LifecycleRule, S3LifecycleRuleStatus, S3NoncurrentVersionExpiration,
    S3ObjectTagSet, S3VersionId, SourceProtocol, StorageGcTaskId, UserId,
};
use std::time::Duration as StdDuration;
use time::Duration;

#[tokio::test]
async fn postgres_standard_s3_lifecycle_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect lifecycle database");
    repository
        .migrate()
        .await
        .expect("migrate lifecycle database");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated lifecycle database");

    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("whole-second lifecycle timestamp");
    let old = now - Duration::days(10);
    let application_id = ApplicationId::new();
    insert_application(&repository, application_id, now).await;
    sqlx::query("UPDATE applications SET used_bytes = 12345, reserved_bytes = 6789 WHERE id = $1")
        .bind(application_id.as_uuid())
        .execute(repository.pool())
        .await
        .expect("seed quota sentinel values");
    let quota_baseline = quota_snapshot(&repository, application_id).await;
    let bucket = S3Bucket::new(
        BucketId::new(),
        application_id,
        "lifecycle-assets",
        "us-east-1",
        true,
        None,
        old,
    )
    .expect("Object Lock lifecycle bucket");
    repository
        .create_s3_bucket(&bucket)
        .await
        .expect("create lifecycle bucket");
    let rules = lifecycle_rules();
    repository
        .replace_s3_bucket_lifecycle(
            application_id,
            bucket.name(),
            Some(S3LifecycleConfiguration::new(rules.clone()).expect("lifecycle")),
            old,
        )
        .await
        .expect("configure lifecycle");

    let store = InMemoryObjectStore::default();
    let first = put_version(
        &repository,
        &store,
        application_id,
        bucket.name(),
        "data/history.bin",
        old,
        "first",
    )
    .await;
    let second = put_version(
        &repository,
        &store,
        application_id,
        bucket.name(),
        "data/history.bin",
        old + Duration::hours(1),
        "second",
    )
    .await;

    repository
        .create_multipart_upload(NewS3MultipartUpload {
            upload_id: "lifecycle-multipart".into(),
            application_id,
            bucket_id: bucket.id(),
            object_key: "data/incomplete.bin".into(),
            content_type: "application/octet-stream".into(),
            user_metadata: serde_json::json!({}),
            object_tags: S3ObjectTagSet::empty(),
            storage_backend: "filesystem".into(),
            expires_at: now + Duration::days(30),
            created_at: old,
        })
        .await
        .expect("create multipart upload");
    repository
        .put_multipart_part(
            "lifecycle-multipart",
            NewS3MultipartPart {
                part_number: 1,
                size: 4,
                sha256: "0".repeat(64),
                md5: "d41d8cd98f00b204e9800998ecf8427e".into(),
                etag: "d41d8cd98f00b204e9800998ecf8427e".into(),
                storage_key: "multipart/lifecycle-part-1".into(),
            },
            1024,
            old,
        )
        .await
        .expect("persist multipart part");

    let first_batch = S3LifecycleService::new(repository.clone(), FixedClock::new(now))
        .run_batch(S3LifecycleBatchCursor::default(), 100)
        .await
        .expect("run first lifecycle batch");
    assert_eq!(first_batch.applied, 3);
    assert_eq!(
        quota_snapshot(&repository, application_id).await,
        quota_baseline
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM object_versions WHERE id = $1")
            .bind(first.version.id().as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("first version state"),
        "deleting"
    );
    let head = sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT current_version_id FROM objects WHERE id = $1",
    )
    .bind(second.object.id().as_uuid())
    .fetch_one(repository.pool())
    .await
    .expect("current delete marker")
    .expect("marker id");
    assert_ne!(head, second.version.id().as_uuid());
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT is_delete_marker FROM object_versions WHERE id = $1")
            .bind(head)
            .fetch_one(repository.pool())
            .await
            .expect("marker flag")
    );
    let action_time = lifecycle_action_time(now);
    assert_eq!(
        sqlx::query_scalar::<_, OffsetDateTime>(
            "SELECT created_at FROM object_versions WHERE id = $1",
        )
        .bind(head)
        .fetch_one(repository.pool())
        .await
        .expect("Lifecycle marker action timestamp"),
        action_time
    );
    assert_eq!(
        sqlx::query_scalar::<_, OffsetDateTime>(
            "SELECT became_noncurrent_at FROM object_versions WHERE id = $1",
        )
        .bind(second.version.id().as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("Lifecycle noncurrent action timestamp"),
        action_time
    );
    assert_eq!(
        sqlx::query_scalar::<_, OffsetDateTime>("SELECT updated_at FROM objects WHERE id = $1")
            .bind(second.object.id().as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("Lifecycle object action timestamp"),
        action_time
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state FROM s3_multipart_uploads WHERE upload_id = 'lifecycle-multipart'",
        )
        .fetch_one(repository.pool())
        .await
        .expect("multipart state"),
        "aborted"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT reason FROM storage_gc_tasks WHERE object_version_id = $1",
        )
        .bind(first.version.id().as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("lifecycle GC reason"),
        "lifecycle_expiration"
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT reason FROM storage_gc_tasks
             WHERE multipart_upload_id = 'lifecycle-multipart' AND storage_key = $1",
        )
        .bind("multipart/lifecycle-part-1")
        .fetch_one(repository.pool())
        .await
        .expect("multipart cleanup reason"),
        "multipart_temporary"
    );

    let later = now + Duration::days(2);
    let history_cleanup = S3LifecycleService::new(repository.clone(), FixedClock::new(later))
        .run_batch(S3LifecycleBatchCursor::default(), 100)
        .await
        .expect("expire noncurrent head and sole marker");
    assert_eq!(history_cleanup.applied, 2);
    assert_eq!(
        sqlx::query_scalar::<_, Option<uuid::Uuid>>(
            "SELECT current_version_id FROM objects WHERE id = $1",
        )
        .bind(second.object.id().as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("empty logical object head"),
        None
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT reason FROM storage_gc_tasks WHERE object_version_id = $1",
        )
        .bind(second.version.id().as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("second lifecycle GC reason"),
        "lifecycle_expiration"
    );
    let idempotent = S3LifecycleService::new(repository.clone(), FixedClock::new(later))
        .run_batch(S3LifecycleBatchCursor::default(), 100)
        .await
        .expect("idempotent lifecycle rerun");
    assert_eq!(idempotent.applied, 0);
    assert_eq!(
        quota_snapshot(&repository, application_id).await,
        quota_baseline
    );

    configuration_revision_fence(
        &repository,
        &store,
        application_id,
        &bucket,
        &rules,
        old,
        later,
    )
    .await;
    current_head_fence(
        &repository,
        &store,
        application_id,
        &bucket,
        &rules[0],
        old,
        later,
    )
    .await;
    retained_current_creates_delete_marker(
        &repository,
        &store,
        application_id,
        &bucket,
        &rules[0],
        old,
        later,
    )
    .await;
    retention_extension_fence(
        &repository,
        &store,
        application_id,
        &bucket,
        &rules[0],
        old,
        later,
    )
    .await;
    multipart_abort_lock_order_contract(
        &repository,
        application_id,
        &bucket,
        &rules[0],
        old,
        later,
    )
    .await;
    multipart_intent_bucket_lock_order_contract(
        &repository,
        application_id,
        &bucket,
        &rules[0],
        old,
        later,
    )
    .await;
    exact_noncurrent_lock_scope_contract(
        &repository,
        &store,
        application_id,
        &bucket,
        &rules[0],
        old,
        later,
    )
    .await;
    escaped_prefix_contract(&repository, &store, application_id, &bucket, later).await;
    expiration_delete_marker_timing_contract(
        &repository,
        &store,
        application_id,
        &bucket,
        old,
        later,
    )
    .await;
    assert_eq!(
        quota_snapshot(&repository, application_id).await,
        quota_baseline
    );
}

async fn multipart_abort_lock_order_contract(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    rule: &S3LifecycleRule,
    old: OffsetDateTime,
    now: OffsetDateTime,
) {
    let upload_id = "lifecycle-lock-order";
    repository
        .create_multipart_upload(NewS3MultipartUpload {
            upload_id: upload_id.into(),
            application_id,
            bucket_id: bucket.id(),
            object_key: "data/lock-order.bin".into(),
            content_type: "application/octet-stream".into(),
            user_metadata: serde_json::json!({}),
            object_tags: S3ObjectTagSet::empty(),
            storage_backend: "filesystem".into(),
            expires_at: now + Duration::days(30),
            created_at: old,
        })
        .await
        .expect("create lock-order multipart upload");
    let configuration = repository
        .get_s3_bucket_configuration(application_id, bucket.name())
        .await
        .expect("lock-order configuration")
        .expect("lock-order bucket");
    let candidate = repository
        .list_s3_multipart_lifecycle_candidates(
            application_id,
            bucket.id(),
            "data/lock-order.bin",
            lifecycle_days_cutoff(now, 1).expect("multipart cutoff"),
            now,
            10,
        )
        .await
        .expect("scan lock-order multipart")
        .into_iter()
        .find(|candidate| candidate.upload_id == upload_id)
        .expect("lock-order multipart candidate");
    let command = lifecycle_command(
        application_id,
        bucket.id(),
        configuration.revision(),
        rule.clone(),
        S3LifecycleTarget::AbortMultipart {
            upload_id: candidate.upload_id,
            object_key: candidate.object_key,
            expected_initiated_at: candidate.initiated_at,
        },
        now,
    );

    let mut completion = repository
        .pool()
        .begin()
        .await
        .expect("begin completion lock-order tx");
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *completion)
        .await
        .expect("bound completion bucket lock wait");
    sqlx::query("SELECT upload_id FROM s3_multipart_uploads WHERE upload_id = $1 FOR UPDATE")
        .bind(upload_id)
        .fetch_one(&mut *completion)
        .await
        .expect("completion owns upload row first");

    let lifecycle_repository = repository.clone();
    let lifecycle =
        tokio::spawn(async move { lifecycle_repository.execute_s3_lifecycle(&command).await });
    wait_until_lifecycle_waits_for_row(repository, "s3_multipart_uploads").await;

    sqlx::query("SELECT id FROM buckets WHERE id = $1 AND application_id = $2 FOR UPDATE")
        .bind(bucket.id().as_uuid())
        .bind(application_id.as_uuid())
        .fetch_one(&mut *completion)
        .await
        .expect("completion can lock bucket without Lifecycle ABBA");
    completion.commit().await.expect("release completion locks");

    let outcome = tokio::time::timeout(StdDuration::from_secs(5), lifecycle)
        .await
        .expect("Lifecycle resumes after completion lock release")
        .expect("Lifecycle task joins")
        .expect("Lifecycle abort succeeds");
    assert_eq!(outcome, S3LifecycleExecutionOutcome::Applied);
}

async fn multipart_intent_bucket_lock_order_contract(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    rule: &S3LifecycleRule,
    old: OffsetDateTime,
    now: OffsetDateTime,
) {
    let upload_id = "lifecycle-intent-lock-order";
    let intent_id = uuid::Uuid::now_v7();
    repository
        .create_multipart_upload(NewS3MultipartUpload {
            upload_id: upload_id.into(),
            application_id,
            bucket_id: bucket.id(),
            object_key: "data/intent-lock-order.bin".into(),
            content_type: "application/octet-stream".into(),
            user_metadata: serde_json::json!({}),
            object_tags: S3ObjectTagSet::empty(),
            storage_backend: "filesystem".into(),
            expires_at: now + Duration::days(30),
            created_at: old,
        })
        .await
        .expect("create intent-lock-order multipart upload");
    sqlx::query(
        "INSERT INTO s3_upload_intents (
             id, application_id, bucket_id, object_key, proposed_version_id, state,
             storage_backend, temporary_storage_key, final_storage_key,
             expected_size_bytes, content_type, user_metadata, object_tags,
             expires_at, created_at, updated_at
         ) VALUES (
             $1, $2, $3, 'data/intent-lock-order.bin', $4, 'staging',
             'filesystem', $5, $6, 4, 'application/octet-stream', '{}'::jsonb, '[]'::jsonb,
             $7, $8, $8
         )",
    )
    .bind(intent_id)
    .bind(application_id.as_uuid())
    .bind(bucket.id().as_uuid())
    .bind(uuid::Uuid::now_v7())
    .bind("multipart/intent-lock-order.tmp")
    .bind("objects/intent-lock-order.final")
    .bind(now + Duration::days(30))
    .bind(old)
    .execute(repository.pool())
    .await
    .expect("insert attached upload intent fixture");
    sqlx::query("UPDATE s3_multipart_uploads SET upload_intent_id = $1 WHERE upload_id = $2")
        .bind(intent_id)
        .bind(upload_id)
        .execute(repository.pool())
        .await
        .expect("attach upload intent fixture");

    let configuration = repository
        .get_s3_bucket_configuration(application_id, bucket.name())
        .await
        .expect("intent-lock-order configuration")
        .expect("intent-lock-order bucket");
    let candidate = repository
        .list_s3_multipart_lifecycle_candidates(
            application_id,
            bucket.id(),
            "data/intent-lock-order.bin",
            lifecycle_days_cutoff(now, 1).expect("intent multipart cutoff"),
            now,
            10,
        )
        .await
        .expect("scan intent-lock-order multipart")
        .into_iter()
        .find(|candidate| candidate.upload_id == upload_id)
        .expect("intent-lock-order multipart candidate");
    let command = lifecycle_command(
        application_id,
        bucket.id(),
        configuration.revision(),
        rule.clone(),
        S3LifecycleTarget::AbortMultipart {
            upload_id: candidate.upload_id,
            object_key: candidate.object_key,
            expected_initiated_at: candidate.initiated_at,
        },
        now,
    );

    let mut completion = repository
        .pool()
        .begin()
        .await
        .expect("begin intent lock-order tx");
    sqlx::query("SET LOCAL lock_timeout = '3s'")
        .execute(&mut *completion)
        .await
        .expect("bound intent/bucket lock wait");
    sqlx::query("SELECT id FROM s3_upload_intents WHERE id = $1 FOR UPDATE")
        .bind(intent_id)
        .fetch_one(&mut *completion)
        .await
        .expect("completion phase owns attached intent");

    let lifecycle_repository = repository.clone();
    let lifecycle =
        tokio::spawn(async move { lifecycle_repository.execute_s3_lifecycle(&command).await });
    wait_until_lifecycle_waits_for_row(repository, "s3_upload_intents").await;
    sqlx::query("SELECT id FROM buckets WHERE id = $1 AND application_id = $2 FOR UPDATE")
        .bind(bucket.id().as_uuid())
        .bind(application_id.as_uuid())
        .fetch_one(&mut *completion)
        .await
        .expect("intent owner can lock bucket without Lifecycle ABBA");
    completion
        .commit()
        .await
        .expect("release intent/bucket locks");

    let outcome = tokio::time::timeout(StdDuration::from_secs(5), lifecycle)
        .await
        .expect("Lifecycle resumes after intent release")
        .expect("intent-lock Lifecycle task joins")
        .expect("intent-lock Lifecycle abort succeeds");
    assert_eq!(outcome, S3LifecycleExecutionOutcome::Applied);
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM s3_upload_intents WHERE id = $1")
            .bind(intent_id)
            .fetch_one(repository.pool())
            .await
            .expect("attached intent aborted"),
        "aborted"
    );
}

async fn wait_until_lifecycle_waits_for_row(repository: &PostgresRepository, table: &str) {
    for _ in 0..200 {
        let waiting = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                 SELECT 1 FROM pg_stat_activity
                 WHERE datname = current_database()
                   AND pid <> pg_backend_pid()
                   AND state = 'active'
                   AND wait_event_type = 'Lock'
                   AND query LIKE '%' || $1 || '%FOR UPDATE%'
             )",
        )
        .bind(table)
        .fetch_one(repository.pool())
        .await
        .expect("observe Lifecycle upload lock wait");
        if waiting {
            return;
        }
        tokio::time::sleep(StdDuration::from_millis(10)).await;
    }
    panic!("Lifecycle did not reach the expected row lock in time: {table}");
}

async fn exact_noncurrent_lock_scope_contract(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    rule: &S3LifecycleRule,
    old: OffsetDateTime,
    now: OffsetDateTime,
) {
    let first = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "data/exact-lock.bin",
        old,
        "exact-lock-first",
    )
    .await;
    let sibling = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "data/exact-lock.bin",
        old + Duration::hours(1),
        "exact-lock-sibling",
    )
    .await;
    let current = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "data/exact-lock.bin",
        old + Duration::hours(2),
        "exact-lock-current",
    )
    .await;
    let configuration = repository
        .get_s3_bucket_configuration(application_id, bucket.name())
        .await
        .expect("exact-lock configuration")
        .expect("exact-lock bucket");
    let candidate = repository
        .list_s3_noncurrent_expiration_candidates(
            application_id,
            bucket.id(),
            "data/exact-lock.bin",
            lifecycle_days_cutoff(now, 1).expect("exact-lock cutoff"),
            now,
            10,
        )
        .await
        .expect("scan exact-lock candidates")
        .into_iter()
        .find(|candidate| candidate.version_id == first.version.id())
        .expect("first noncurrent candidate");
    let command = lifecycle_command(
        application_id,
        bucket.id(),
        configuration.revision(),
        rule.clone(),
        S3LifecycleTarget::ExpireNoncurrent {
            object_id: candidate.object_id,
            object_key: candidate.object_key,
            version_id: candidate.version_id,
            expected_became_noncurrent_at: candidate.became_noncurrent_at,
        },
        now,
    );

    let mut sibling_lock = repository
        .pool()
        .begin()
        .await
        .expect("begin sibling version lock tx");
    sqlx::query("SELECT id FROM object_versions WHERE id = $1 FOR UPDATE")
        .bind(sibling.version.id().as_uuid())
        .fetch_one(&mut *sibling_lock)
        .await
        .expect("lock unrelated active sibling version");
    let lifecycle_repository = repository.clone();
    let mut lifecycle =
        tokio::spawn(async move { lifecycle_repository.execute_s3_lifecycle(&command).await });
    let completed = tokio::time::timeout(StdDuration::from_secs(3), &mut lifecycle).await;
    if completed.is_err() {
        sibling_lock
            .rollback()
            .await
            .expect("release sibling after timeout");
        let _ = lifecycle.await;
        panic!("exact noncurrent expiration waited on an unrelated active version");
    }
    let outcome = completed
        .expect("checked above")
        .expect("exact-lock task joins")
        .expect("exact-lock lifecycle succeeds");
    assert_eq!(outcome, S3LifecycleExecutionOutcome::Applied);
    sibling_lock
        .rollback()
        .await
        .expect("release unrelated sibling lock");
    assert_eq!(
        current_version_id(repository, first.object.id()).await,
        Some(current.version.id())
    );
}

async fn escaped_prefix_contract(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    now: OffsetDateTime,
) {
    let key = "data/%_\\literal.bin";
    put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        key,
        now,
        "escaped-prefix",
    )
    .await;
    let candidates = repository
        .list_s3_current_expiration_candidates(
            application_id,
            bucket.id(),
            "data/%_\\literal",
            None,
            now,
            10,
        )
        .await
        .expect("scan escaped lifecycle prefix");
    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.object_key.as_str())
            .collect::<Vec<_>>(),
        vec![key]
    );
}

async fn expiration_delete_marker_timing_contract(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    old: OffsetDateTime,
    now: OffsetDateTime,
) {
    let action_day = lifecycle_action_time(now);
    let marker_creation_time = action_day + Duration::hours(12);
    let initial_rules = vec![
        marker_timing_rule(
            "marker-days-initial",
            "marker-days/",
            S3Expiration::Days(10),
            true,
        ),
        marker_timing_rule(
            "marker-date-initial",
            "marker-date/",
            S3Expiration::Days(10),
            true,
        ),
    ];
    repository
        .replace_s3_bucket_lifecycle(
            application_id,
            bucket.name(),
            Some(S3LifecycleConfiguration::new(initial_rules).expect("marker initial rules")),
            marker_creation_time,
        )
        .await
        .expect("install marker timing rules");
    let days_object = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "marker-days/object.bin",
        old,
        "marker-days",
    )
    .await;
    let date_object = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "marker-date/object.bin",
        old,
        "marker-date",
    )
    .await;
    let marker_creation =
        S3LifecycleService::new(repository.clone(), FixedClock::new(marker_creation_time))
            .run_batch(S3LifecycleBatchCursor::default(), 100)
            .await
            .expect("create Lifecycle timing markers");
    assert_eq!(marker_creation.applied, 2);
    let days_marker = current_version_id(repository, days_object.object.id())
        .await
        .expect("days marker head");
    let date_marker = current_version_id(repository, date_object.object.id())
        .await
        .expect("date marker head");
    assert_ne!(days_marker, days_object.version.id());
    assert_ne!(date_marker, date_object.version.id());

    let noncurrent_cleanup_time = action_day + Duration::days(3) + Duration::hours(12);
    let noncurrent_cleanup =
        S3LifecycleService::new(repository.clone(), FixedClock::new(noncurrent_cleanup_time))
            .run_batch(S3LifecycleBatchCursor::default(), 100)
            .await
            .expect("make timing markers sole without expiring them");
    assert_eq!(noncurrent_cleanup.applied, 2);
    assert_eq!(
        active_version_count(repository, days_object.object.id()).await,
        1
    );
    assert_eq!(
        active_version_count(repository, date_object.object.id()).await,
        1
    );
    assert_eq!(
        current_version_id(repository, days_object.object.id()).await,
        Some(days_marker)
    );
    assert_eq!(
        current_version_id(repository, date_object.object.id()).await,
        Some(date_marker)
    );

    let date_expiration = action_day + Duration::days(20);
    let final_rules = vec![
        marker_timing_rule(
            "marker-days-final",
            "marker-days/",
            S3Expiration::Days(10),
            false,
        ),
        marker_timing_rule(
            "marker-date-final",
            "marker-date/",
            S3Expiration::Date(date_expiration),
            false,
        ),
    ];
    repository
        .replace_s3_bucket_lifecycle(
            application_id,
            bucket.name(),
            Some(S3LifecycleConfiguration::new(final_rules).expect("marker final rules")),
            noncurrent_cleanup_time + Duration::seconds(1),
        )
        .await
        .expect("install Days/Date marker rules");

    let before_due = action_day + Duration::days(9) + Duration::hours(12);
    let before_due_batch = S3LifecycleService::new(repository.clone(), FixedClock::new(before_due))
        .run_batch(S3LifecycleBatchCursor::default(), 100)
        .await
        .expect("evaluate markers before Days/Date due");
    assert_eq!(before_due_batch.applied, 0);
    assert_eq!(
        current_version_id(repository, days_object.object.id()).await,
        Some(days_marker)
    );
    assert_eq!(
        current_version_id(repository, date_object.object.id()).await,
        Some(date_marker)
    );

    let days_due = action_day + Duration::days(11) + Duration::hours(12);
    let days_due_batch = S3LifecycleService::new(repository.clone(), FixedClock::new(days_due))
        .run_batch(S3LifecycleBatchCursor::default(), 100)
        .await
        .expect("remove Days-expired sole marker");
    assert_eq!(days_due_batch.applied, 1);
    assert_eq!(
        current_version_id(repository, days_object.object.id()).await,
        None
    );
    assert_eq!(
        current_version_id(repository, date_object.object.id()).await,
        Some(date_marker)
    );

    let date_due_batch =
        S3LifecycleService::new(repository.clone(), FixedClock::new(date_expiration))
            .run_batch(S3LifecycleBatchCursor::default(), 100)
            .await
            .expect("remove Date-expired sole marker");
    assert_eq!(date_due_batch.applied, 1);
    assert_eq!(
        current_version_id(repository, date_object.object.id()).await,
        None
    );
}

fn marker_timing_rule(
    id: &str,
    prefix: &str,
    expiration: S3Expiration,
    expire_noncurrent: bool,
) -> S3LifecycleRule {
    S3LifecycleRule {
        id: Some(id.into()),
        status: S3LifecycleRuleStatus::Enabled,
        filter: S3LifecycleFilter::Prefix(prefix.into()),
        expiration: Some(expiration),
        noncurrent_version_expiration: expire_noncurrent
            .then_some(S3NoncurrentVersionExpiration { noncurrent_days: 1 }),
        abort_incomplete_multipart_upload: None,
    }
}

async fn active_version_count(
    repository: &PostgresRepository,
    object_id: mediahub_core::ObjectId,
) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM object_versions
         WHERE object_id = $1 AND state = 'committed' AND superseded_at IS NULL",
    )
    .bind(object_id.as_uuid())
    .fetch_one(repository.pool())
    .await
    .expect("count active object versions")
}

fn lifecycle_command(
    application_id: ApplicationId,
    bucket_id: BucketId,
    revision: u64,
    rule: S3LifecycleRule,
    target: S3LifecycleTarget,
    now: OffsetDateTime,
) -> ExecuteS3LifecycleCommand {
    let marker_id = ObjectVersionId::new();
    ExecuteS3LifecycleCommand {
        application_id,
        bucket_id,
        expected_configuration_revision: revision,
        rule,
        target,
        evaluated_at: now,
        delete_marker_id: marker_id,
        delete_marker_version_id: S3VersionId::new(marker_id.to_string())
            .expect("opaque lifecycle marker version id"),
        gc_task_id: StorageGcTaskId::new(),
        gc_not_before: now,
        gc_max_attempts: 10,
    }
}

async fn quota_snapshot(
    repository: &PostgresRepository,
    application_id: ApplicationId,
) -> (i64, i64) {
    sqlx::query_as::<_, (i64, i64)>(
        "SELECT used_bytes, reserved_bytes FROM applications WHERE id = $1",
    )
    .bind(application_id.as_uuid())
    .fetch_one(repository.pool())
    .await
    .expect("read lifecycle quota snapshot")
}

async fn configuration_revision_fence(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    rules: &[S3LifecycleRule],
    old: OffsetDateTime,
    now: OffsetDateTime,
) {
    let receipt = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "data/config-race.bin",
        old,
        "config-race",
    )
    .await;
    let configuration = repository
        .get_s3_bucket_configuration(application_id, bucket.name())
        .await
        .expect("read configuration")
        .expect("bucket configuration");
    let candidate = current_candidate(
        repository,
        application_id,
        bucket,
        "data/config-race.bin",
        now,
    )
    .await;
    let command = current_command(
        application_id,
        bucket.id(),
        configuration.revision(),
        rules[0].clone(),
        candidate,
        now,
    );
    repository
        .replace_s3_bucket_lifecycle(application_id, bucket.name(), None, now)
        .await
        .expect("remove lifecycle after scan");
    assert_eq!(
        repository
            .execute_s3_lifecycle(&command)
            .await
            .expect("configuration-fenced execute"),
        S3LifecycleExecutionOutcome::ConfigurationChanged
    );
    assert_eq!(
        current_version_id(repository, receipt.object.id()).await,
        Some(receipt.version.id())
    );
    repository
        .replace_s3_bucket_lifecycle(
            application_id,
            bucket.name(),
            Some(S3LifecycleConfiguration::new(rules.to_vec()).expect("restore lifecycle")),
            now + Duration::seconds(1),
        )
        .await
        .expect("restore lifecycle after fence");
}

async fn current_head_fence(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    rule: &S3LifecycleRule,
    old: OffsetDateTime,
    now: OffsetDateTime,
) {
    let stale = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "data/head-race.bin",
        old,
        "stale-head",
    )
    .await;
    let configuration = repository
        .get_s3_bucket_configuration(application_id, bucket.name())
        .await
        .expect("configuration")
        .expect("bucket");
    let candidate = current_candidate(
        repository,
        application_id,
        bucket,
        "data/head-race.bin",
        now,
    )
    .await;
    let command = current_command(
        application_id,
        bucket.id(),
        configuration.revision(),
        rule.clone(),
        candidate,
        now,
    );
    let replacement = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "data/head-race.bin",
        now,
        "new-head",
    )
    .await;
    assert_eq!(
        repository
            .execute_s3_lifecycle(&command)
            .await
            .expect("head-fenced execute"),
        S3LifecycleExecutionOutcome::TargetChanged
    );
    assert_eq!(
        current_version_id(repository, stale.object.id()).await,
        Some(replacement.version.id())
    );
}

async fn retained_current_creates_delete_marker(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    rule: &S3LifecycleRule,
    old: OffsetDateTime,
    now: OffsetDateTime,
) {
    let locked = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "data/retention-race.bin",
        old,
        "retention-race",
    )
    .await;
    let configuration = repository
        .get_s3_bucket_configuration(application_id, bucket.name())
        .await
        .expect("configuration")
        .expect("bucket");
    let candidate = current_candidate(
        repository,
        application_id,
        bucket,
        "data/retention-race.bin",
        now,
    )
    .await;
    let command = current_command(
        application_id,
        bucket.id(),
        configuration.revision(),
        rule.clone(),
        candidate,
        now,
    );
    let service = S3ObjectService::new(
        repository.clone(),
        repository.clone(),
        repository.clone(),
        store.clone(),
        FixedClock::new(now),
    );
    service
        .put_object_retention(&PutObjectRetentionRequest {
            object: S3ObjectRequest {
                application_id,
                bucket_name: bucket.name().into(),
                object_key: "data/retention-race.bin".into(),
                version_id: Some(locked.version.external_version_id().clone()),
            },
            retention: ObjectRetention::new(RetentionMode::Governance, now + Duration::days(30)),
            bypass_governance: false,
        })
        .await
        .expect("extend retention after scan");
    assert_eq!(
        repository
            .execute_s3_lifecycle(&command)
            .await
            .expect("retained current expiration"),
        S3LifecycleExecutionOutcome::Applied
    );
    let marker_id = current_version_id(repository, locked.object.id())
        .await
        .expect("Lifecycle delete marker");
    assert_ne!(marker_id, locked.version.id());
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT is_delete_marker FROM object_versions WHERE id = $1")
            .bind(marker_id.as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("retained current marker flag")
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM object_versions WHERE id = $1")
            .bind(locked.version.id().as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("retained version state"),
        "committed"
    );
}

async fn retention_extension_fence(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    rule: &S3LifecycleRule,
    old: OffsetDateTime,
    now: OffsetDateTime,
) {
    let locked = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "data/noncurrent-retention-race.bin",
        old,
        "locked-noncurrent",
    )
    .await;
    let replacement = put_version(
        repository,
        store,
        application_id,
        bucket.name(),
        "data/noncurrent-retention-race.bin",
        old + Duration::hours(1),
        "replacement-head",
    )
    .await;
    let configuration = repository
        .get_s3_bucket_configuration(application_id, bucket.name())
        .await
        .expect("configuration")
        .expect("bucket");
    let candidate = repository
        .list_s3_noncurrent_expiration_candidates(
            application_id,
            bucket.id(),
            "data/noncurrent-retention-race.bin",
            lifecycle_days_cutoff(now, 1).expect("noncurrent cutoff"),
            now,
            10,
        )
        .await
        .expect("scan noncurrent lifecycle candidate")
        .into_iter()
        .find(|candidate| candidate.version_id == locked.version.id())
        .expect("locked noncurrent candidate");
    let marker_id = ObjectVersionId::new();
    let command = ExecuteS3LifecycleCommand {
        application_id,
        bucket_id: bucket.id(),
        expected_configuration_revision: configuration.revision(),
        rule: rule.clone(),
        target: S3LifecycleTarget::ExpireNoncurrent {
            object_id: candidate.object_id,
            object_key: candidate.object_key,
            version_id: candidate.version_id,
            expected_became_noncurrent_at: candidate.became_noncurrent_at,
        },
        evaluated_at: now,
        delete_marker_id: marker_id,
        delete_marker_version_id: S3VersionId::new(marker_id.to_string())
            .expect("opaque marker version id"),
        gc_task_id: StorageGcTaskId::new(),
        gc_not_before: now,
        gc_max_attempts: 10,
    };
    let service = S3ObjectService::new(
        repository.clone(),
        repository.clone(),
        repository.clone(),
        store.clone(),
        FixedClock::new(now),
    );
    service
        .put_object_retention(&PutObjectRetentionRequest {
            object: S3ObjectRequest {
                application_id,
                bucket_name: bucket.name().into(),
                object_key: "data/noncurrent-retention-race.bin".into(),
                version_id: Some(locked.version.external_version_id().clone()),
            },
            retention: ObjectRetention::new(RetentionMode::Governance, now + Duration::days(30)),
            bypass_governance: false,
        })
        .await
        .expect("extend noncurrent retention after scan");
    assert_eq!(
        repository
            .execute_s3_lifecycle(&command)
            .await
            .expect("Object Lock-fenced noncurrent execute"),
        S3LifecycleExecutionOutcome::Locked
    );
    assert_eq!(
        current_version_id(repository, locked.object.id()).await,
        Some(replacement.version.id())
    );
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT state FROM object_versions WHERE id = $1")
            .bind(locked.version.id().as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("locked noncurrent version state"),
        "committed"
    );
}

async fn current_candidate(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    bucket: &S3Bucket,
    key: &str,
    now: OffsetDateTime,
) -> S3CurrentExpirationCandidate {
    repository
        .list_s3_current_expiration_candidates(
            application_id,
            bucket.id(),
            key,
            Some(lifecycle_days_cutoff(now, 1).expect("cutoff")),
            now,
            10,
        )
        .await
        .expect("scan current lifecycle candidate")
        .into_iter()
        .find(|candidate| candidate.object_key == key)
        .expect("target candidate")
}

fn current_command(
    application_id: ApplicationId,
    bucket_id: BucketId,
    revision: u64,
    rule: S3LifecycleRule,
    candidate: S3CurrentExpirationCandidate,
    now: OffsetDateTime,
) -> ExecuteS3LifecycleCommand {
    let marker_id = ObjectVersionId::new();
    ExecuteS3LifecycleCommand {
        application_id,
        bucket_id,
        expected_configuration_revision: revision,
        rule,
        target: S3LifecycleTarget::ExpireCurrent {
            object_id: candidate.object_id,
            object_key: candidate.object_key,
            expected_current_version_id: candidate.current_version_id,
            version_created_at: candidate.version_created_at,
        },
        evaluated_at: now,
        delete_marker_id: marker_id,
        delete_marker_version_id: S3VersionId::new(marker_id.to_string()).expect("version id"),
        gc_task_id: StorageGcTaskId::new(),
        gc_not_before: now,
        gc_max_attempts: 10,
    }
}

async fn current_version_id(
    repository: &PostgresRepository,
    object_id: mediahub_core::ObjectId,
) -> Option<ObjectVersionId> {
    sqlx::query_scalar::<_, Option<uuid::Uuid>>(
        "SELECT current_version_id FROM objects WHERE id = $1",
    )
    .bind(object_id.as_uuid())
    .fetch_one(repository.pool())
    .await
    .expect("current version id")
    .map(ObjectVersionId::from_uuid)
}

fn lifecycle_rules() -> Vec<S3LifecycleRule> {
    vec![
        S3LifecycleRule {
            id: Some("expire-data".into()),
            status: S3LifecycleRuleStatus::Enabled,
            filter: S3LifecycleFilter::Prefix("data/".into()),
            expiration: Some(S3Expiration::Days(1)),
            noncurrent_version_expiration: Some(S3NoncurrentVersionExpiration {
                noncurrent_days: 1,
            }),
            abort_incomplete_multipart_upload: Some(S3AbortIncompleteMultipartUpload {
                days_after_initiation: 1,
            }),
        },
        S3LifecycleRule {
            id: Some("remove-expired-markers".into()),
            status: S3LifecycleRuleStatus::Enabled,
            filter: S3LifecycleFilter::Prefix("data/".into()),
            expiration: Some(S3Expiration::ExpiredObjectDeleteMarker),
            noncurrent_version_expiration: None,
            abort_incomplete_multipart_upload: None,
        },
    ]
}

async fn put_version(
    repository: &PostgresRepository,
    store: &InMemoryObjectStore,
    application_id: ApplicationId,
    bucket_name: &str,
    object_key: &str,
    at: OffsetDateTime,
    creator: &str,
) -> CompletePutObjectReceipt {
    let service = S3ObjectService::new(
        repository.clone(),
        repository.clone(),
        repository.clone(),
        store.clone(),
        FixedClock::new(at),
    );
    let begun = service
        .begin_put(&BeginPutObjectRequest {
            application_id,
            bucket_name: bucket_name.into(),
            object_key: object_key.into(),
            expected_size_bytes: 8,
            content_type: Some("application/octet-stream".into()),
            user_metadata: serde_json::json!({}),
            object_tags: S3ObjectTagSet::empty(),
            expires_at: None,
        })
        .await
        .expect("begin lifecycle object");
    store
        .put_temporary(
            begun.intent.temporary_storage_key(),
            b"prismark",
            "application/octet-stream",
        )
        .await
        .expect("stage lifecycle object");
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
        .expect("complete lifecycle object")
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
    .bind(format!("lifecycle-{user_id}@contract.invalid"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert lifecycle contract user");
    sqlx::query(
        "INSERT INTO applications
            (id, user_id, name, app_id, quota_bytes, created_at, updated_at)
         VALUES ($1, $2, 'Lifecycle Contract', $3, 1073741824, $4, $4)",
    )
    .bind(application_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(format!("lifecycle-{application_id}"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert lifecycle contract application");
}
