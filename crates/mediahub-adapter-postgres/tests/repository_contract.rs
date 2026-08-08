use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{
    AccessKeyRepository, AdminBootstrapOutcome, AdminRepository, ApplicationRepository,
    AsyncJobCompletion, AsyncJobCreation, AsyncJobRepository, AuditEvent, AuditRepository,
    AuthRepository, CompletedIdempotencyResponse, IdempotencyClaim, IdempotencyContext,
    MediaDirectoryListQuery, MediaListCursor, MediaListQuery, MediaRepository, NewAccessKey,
    NewVariant, NewWebhookEndpoint, OneTimeTokenPurpose, OutboxEvent, RepositoryError,
    S3MediaListQuery, SecretKeyVersionRepository, UploadSessionCancellation,
    UploadSessionRepository, VariantClaim, VariantRepository, WebhookDeliveryFailureDisposition,
    WebhookDeliveryHistoryCursor, WebhookDeliveryHistoryQuery, WebhookDeliveryHistoryStatus,
    WebhookDeliveryRepository, WebhookEndpointRepository, WebhookEndpointUpdate,
};
use mediahub_core::{
    ApplicationId, AsyncJob, AsyncJobAction, AsyncJobId, AsyncJobItemResult, AsyncJobState, Bucket,
    BucketId, BucketPolicy, ClientMetadata, LifecycleRule, Media, MediaId, NewAsyncJob, NewMedia,
    NewUploadSession, OffsetDateTime, SystemMetadata, UploadSession, UploadSessionId, UserId,
    VariantFormat, VariantId, Visibility,
};
use serde_json::{Value, json};
use sqlx::{Row, types::Json};
use time::Duration;
use uuid::Uuid;

/// Runs the complete PostgreSQL persistence contract against a dedicated database.
#[tokio::test]
async fn postgres_repository_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect contract database");
    repository
        .migrate()
        .await
        .expect("migrate contract database");
    sqlx::query("TRUNCATE TABLE audit_logs, users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated contract database");
    sqlx::query(
        "INSERT INTO system_settings (singleton, download_bytes_per_second, updated_by, \
                updated_request_id, updated_at) VALUES (TRUE, 33554432, NULL, NULL, NOW())",
    )
    .execute(repository.pool())
    .await
    .expect("restore default system settings");

    control_plane_schema_contract(&repository).await;
    tenant_fk_schema_contract(&repository).await;
    control_plane_repository_contract(&repository).await;
    let fixture = Fixture::create(&repository).await;
    administration_and_webhook_contract(&repository, &fixture).await;
    bucket_and_idempotency_contract(&repository, &fixture).await;
    quota_and_media_contract(&repository, &fixture).await;
    media_query_and_lifecycle_contract(&repository, &fixture).await;
    upload_session_contract(&repository, &fixture).await;
    async_job_contract(&repository, &fixture).await;
    variant_contract(&repository, &fixture).await;
}

async fn tenant_fk_schema_contract(repository: &PostgresRepository) {
    let rows = sqlx::query(
        "SELECT conname, pg_get_constraintdef(oid) AS definition \
         FROM pg_constraint \
         WHERE conname IN (\
             'media_bucket_application_fkey',\
             'upload_sessions_bucket_application_fkey',\
             's3_multipart_uploads_bucket_application_fkey'\
         )",
    )
    .fetch_all(repository.pool())
    .await
    .expect("inspect tenant foreign keys");
    assert_eq!(rows.len(), 3, "all tenant foreign keys must be present");
    for row in rows {
        let definition: String = row.try_get("definition").expect("decode FK definition");
        assert!(
            definition.contains("(bucket_id, application_id)"),
            "tenant FK must bind bucket and application together: {definition}"
        );
    }
}

async fn bucket_and_idempotency_contract(repository: &PostgresRepository, fixture: &Fixture) {
    let policy = BucketPolicy::unrestricted(Visibility::Private);
    let bucket = Bucket::new(
        BucketId::new(),
        fixture.application_id,
        "managed-contract",
        policy,
        fixture.now,
    )
    .expect("create managed bucket");
    repository
        .create_bucket(&bucket)
        .await
        .expect("persist managed bucket");
    assert_eq!(
        repository
            .find_bucket_by_name(fixture.application_id, bucket.name())
            .await
            .expect("find bucket by name")
            .expect("managed bucket exists"),
        bucket
    );
    assert!(
        repository
            .list_buckets(fixture.application_id)
            .await
            .expect("list buckets")
            .iter()
            .any(|stored| stored.id() == bucket.id())
    );

    let lifecycle_policy = BucketPolicy::new(
        Visibility::Public,
        Some(3_600),
        Some(512),
        ["image/png".to_owned()],
    )
    .and_then(|policy| {
        policy.with_lifecycle_rules(vec![LifecycleRule::KeepLatest {
            id: "managed-latest".to_owned(),
            enabled: true,
            prefix: "managed/".to_owned(),
            count: 2,
        }])
    })
    .expect("valid managed policy");
    assert!(
        repository
            .update_bucket_policy(
                fixture.application_id,
                bucket.name(),
                &lifecycle_policy,
                fixture.now + Duration::seconds(1),
            )
            .await
            .expect("update managed bucket")
    );
    let updated = repository
        .find_bucket_by_name(fixture.application_id, bucket.name())
        .await
        .expect("reload updated bucket")
        .expect("updated bucket exists");
    assert_eq!(updated.policy(), &lifecycle_policy);
    assert!(
        repository
            .delete_empty_bucket(fixture.application_id, bucket.name())
            .await
            .expect("delete empty managed bucket")
    );
    assert!(
        !repository
            .delete_empty_bucket(fixture.application_id, bucket.name())
            .await
            .expect("missing bucket deletion is idempotent")
    );

    let expires_at = fixture.now + Duration::minutes(10);
    let basic_claim = repository
        .claim_idempotency_key(
            fixture.application_id,
            "contract.basic",
            "basic-key",
            &"a".repeat(64),
            expires_at,
            fixture.now,
        )
        .await
        .expect("claim idempotency key");
    let basic_token = match basic_claim {
        IdempotencyClaim::Claimed(token) => token,
        claim => panic!("expected claimed idempotency key, got {claim:?}"),
    };
    assert_eq!(
        repository
            .claim_idempotency_key(
                fixture.application_id,
                "contract.basic",
                "basic-key",
                &"a".repeat(64),
                expires_at,
                fixture.now,
            )
            .await
            .expect("reclaim in-progress key"),
        IdempotencyClaim::InProgress
    );
    assert_eq!(
        repository
            .claim_idempotency_key(
                fixture.application_id,
                "contract.basic",
                "basic-key",
                &"b".repeat(64),
                expires_at,
                fixture.now,
            )
            .await
            .expect("claim changed request"),
        IdempotencyClaim::Conflict
    );
    let response = CompletedIdempotencyResponse {
        status: 201,
        payload: "{\"created\":true}".to_owned(),
        resource_id: Some("basic-resource".to_owned()),
    };
    repository
        .complete_idempotency_key(
            fixture.application_id,
            "contract.basic",
            "basic-key",
            &"a".repeat(64),
            &basic_token,
            &response,
            fixture.now + Duration::seconds(2),
        )
        .await
        .expect("complete idempotency key");
    assert_eq!(
        repository
            .claim_idempotency_key(
                fixture.application_id,
                "contract.basic",
                "basic-key",
                &"a".repeat(64),
                expires_at,
                fixture.now,
            )
            .await
            .expect("replay completed key"),
        IdempotencyClaim::Completed(response)
    );

    let atomic_bucket = Bucket::new(
        BucketId::new(),
        fixture.application_id,
        "atomic-contract",
        BucketPolicy::unrestricted(Visibility::Private),
        fixture.now,
    )
    .expect("create atomic bucket");
    let mut context = IdempotencyContext {
        application_id: fixture.application_id,
        operation_scope: "contract.bucket.create".to_owned(),
        key: "atomic-bucket-key".to_owned(),
        request_hash: "c".repeat(64),
        claim_token: String::new(),
    };
    let claim = repository
        .claim_idempotency_key(
            context.application_id,
            &context.operation_scope,
            &context.key,
            &context.request_hash,
            expires_at,
            fixture.now,
        )
        .await
        .expect("claim atomic bucket key");
    context.claim_token = match claim {
        IdempotencyClaim::Claimed(token) => token,
        claim => panic!("expected claimed idempotency key, got {claim:?}"),
    };
    let response = CompletedIdempotencyResponse {
        status: 201,
        payload: "{\"bucket\":true}".to_owned(),
        resource_id: Some(atomic_bucket.id().to_string()),
    };
    repository
        .create_bucket_and_complete_idempotency(
            &atomic_bucket,
            &context,
            &response,
            fixture.now + Duration::seconds(3),
        )
        .await
        .expect("atomically create bucket and response");
    assert!(
        repository
            .find_bucket_by_name(fixture.application_id, atomic_bucket.name())
            .await
            .expect("find atomic bucket")
            .is_some()
    );
    assert_eq!(
        repository
            .claim_idempotency_key(
                context.application_id,
                &context.operation_scope,
                &context.key,
                &context.request_hash,
                expires_at,
                fixture.now,
            )
            .await
            .expect("replay atomic bucket response"),
        IdempotencyClaim::Completed(response)
    );

    let mut release_context = IdempotencyContext {
        application_id: fixture.application_id,
        operation_scope: "contract.release".to_owned(),
        key: "release-key".to_owned(),
        request_hash: "d".repeat(64),
        claim_token: String::new(),
    };
    let claim = repository
        .claim_idempotency_key(
            release_context.application_id,
            &release_context.operation_scope,
            &release_context.key,
            &release_context.request_hash,
            expires_at,
            fixture.now,
        )
        .await
        .expect("claim releasable key");
    release_context.claim_token = match claim {
        IdempotencyClaim::Claimed(token) => token,
        claim => panic!("expected claimed idempotency key, got {claim:?}"),
    };
    repository
        .release_idempotency_key(&release_context)
        .await
        .expect("release in-progress key");
    let reclaimed = repository
        .claim_idempotency_key(
            release_context.application_id,
            &release_context.operation_scope,
            &release_context.key,
            &release_context.request_hash,
            expires_at,
            fixture.now,
        )
        .await
        .expect("reclaim released key");
    assert!(matches!(reclaimed, IdempotencyClaim::Claimed(_)));
}

async fn control_plane_repository_contract(repository: &PostgresRepository) {
    let now = postgres_now();
    let user_id = UserId::new();
    repository
        .create_user(
            user_id,
            "control-plane@contract.invalid",
            "initial-hash",
            now,
        )
        .await
        .expect("create pending user");
    let pending = repository
        .find_user_by_email("control-plane@contract.invalid")
        .await
        .expect("find pending user")
        .expect("pending user exists");
    assert_eq!(pending.status, "pending_verification");
    assert_eq!(
        repository
            .create_user(user_id, "control-plane@contract.invalid", "duplicate", now)
            .await,
        Err(RepositoryError::Conflict)
    );

    let default_settings = repository
        .admin_system_settings()
        .await
        .expect("read default system settings");
    assert_eq!(
        default_settings.download_bytes_per_second,
        Some(32 * 1024 * 1024)
    );
    let limited_settings = repository
        .update_admin_system_settings(
            user_id,
            Some(64 * 1024 * 1024),
            "req-settings-limited",
            now + Duration::seconds(1),
        )
        .await
        .expect("enable response throttling");
    assert_eq!(
        limited_settings.download_bytes_per_second,
        Some(64 * 1024 * 1024)
    );
    let unlimited_settings = repository
        .update_admin_system_settings(
            user_id,
            None,
            "req-settings-unlimited",
            now + Duration::seconds(2),
        )
        .await
        .expect("disable response throttling");
    assert_eq!(unlimited_settings.download_bytes_per_second, None);

    let application_id = ApplicationId::new();
    repository
        .create_application(
            application_id,
            user_id,
            "Control plane",
            "app_control_plane_contract",
            4096,
            now,
        )
        .await
        .expect("create application");
    let application = repository
        .default_application_for_user(user_id)
        .await
        .expect("default application")
        .expect("application exists");
    assert_eq!(application.id, application_id);
    assert_eq!(application.quota.quota_bytes, 4096);
    assert_eq!(
        repository
            .find_application_by_app_id("app_control_plane_contract")
            .await
            .expect("public application lookup")
            .expect("public application exists")
            .id,
        application_id
    );
    assert!(
        repository
            .find_application_by_app_id("app_missing")
            .await
            .expect("missing public application lookup")
            .is_none()
    );
    assert!(
        repository
            .application_for_user_by_id(UserId::new(), application_id)
            .await
            .expect("cross-owner lookup")
            .is_none()
    );
    assert!(
        repository
            .update_application_name_for_user(
                user_id,
                "app_control_plane_contract",
                "Renamed",
                now + Duration::seconds(1),
            )
            .await
            .expect("rename application")
    );
    assert_eq!(
        repository
            .list_applications_for_user(user_id)
            .await
            .expect("list applications")[0]
            .name,
        "Renamed"
    );

    repository
        .create_one_time_token(
            user_id,
            OneTimeTokenPurpose::VerifyEmail,
            "verify-token-hash",
            now + Duration::minutes(10),
            now,
        )
        .await
        .expect("create verification token");
    repository
        .create_one_time_token(
            user_id,
            OneTimeTokenPurpose::VerifyEmail,
            "verify-token-hash-2",
            now + Duration::minutes(11),
            now + Duration::seconds(1),
        )
        .await
        .expect("rotate verification token");
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM one_time_tokens \
             WHERE user_id = $1 AND purpose = 'verify_email' AND consumed_at IS NULL",
        )
        .bind(user_id.as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("count active verification tokens"),
        1
    );
    assert!(
        repository
            .consume_email_verification_token("verify-token-hash-2", now + Duration::seconds(2))
            .await
            .expect("consume verification token")
    );
    assert!(
        !repository
            .consume_email_verification_token("verify-token-hash", now + Duration::seconds(3))
            .await
            .expect("verification token is one-time")
    );

    repository
        .create_session_with_context(
            user_id,
            "current-session-hash",
            "current-csrf-hash",
            now + Duration::hours(2),
            now + Duration::seconds(2),
            Some("127.0.0.1"),
            Some("contract-agent"),
        )
        .await
        .expect("create current session");
    repository
        .create_session(
            user_id,
            "other-session-hash",
            "other-csrf-hash",
            now + Duration::hours(2),
            now + Duration::seconds(3),
        )
        .await
        .expect("create other session");
    assert!(
        repository
            .valid_session_csrf(
                "current-session-hash",
                "current-csrf-hash",
                now + Duration::minutes(1),
            )
            .await
            .expect("validate CSRF")
    );
    repository
        .record_user_login(user_id, now + Duration::seconds(4))
        .await
        .expect("record login");
    let sessions = repository
        .list_active_sessions(user_id, "current-session-hash", now + Duration::minutes(1))
        .await
        .expect("list sessions");
    assert_eq!(sessions.len(), 2);
    let current = sessions
        .iter()
        .find(|session| session.is_current)
        .expect("current session marked");
    assert_eq!(current.created_ip.as_deref(), Some("127.0.0.1"));
    assert_eq!(
        repository
            .revoke_session(
                user_id,
                &current.id,
                "current-session-hash",
                now + Duration::minutes(2),
            )
            .await
            .expect("revoke current session"),
        Some(true)
    );

    repository
        .create_one_time_token(
            user_id,
            OneTimeTokenPurpose::ResetPassword,
            "reset-token-hash",
            now + Duration::minutes(10),
            now + Duration::minutes(2),
        )
        .await
        .expect("create reset token");
    assert!(
        repository
            .consume_password_reset_token(
                "reset-token-hash",
                "replacement-hash",
                now + Duration::minutes(3),
            )
            .await
            .expect("reset password")
    );
    assert!(
        repository
            .list_active_sessions(user_id, "other-session-hash", now + Duration::minutes(4))
            .await
            .expect("password reset revokes sessions")
            .is_empty()
    );

    let access_key = NewAccessKey {
        id: Uuid::new_v4().to_string(),
        application_id,
        access_key_id: "mh_ak_control_contract".into(),
        secret_ciphertext: "contract-ciphertext".into(),
        secret_key_version: 1,
        secret_last_four: "last".into(),
        name: "Contract key".into(),
        permissions: vec!["media:read".into()],
        expires_at: Some(now + Duration::hours(1)),
        created_at: now,
    };
    repository
        .create_access_key(&access_key)
        .await
        .expect("create access key");
    assert_eq!(
        repository
            .list_access_keys(application_id)
            .await
            .expect("list access keys")
            .len(),
        1
    );
    assert!(
        repository
            .find_active_access_key(&access_key.access_key_id, now)
            .await
            .expect("find active key")
            .is_some()
    );
    assert!(
        repository
            .update_access_key(
                &access_key.access_key_id,
                application_id,
                "Updated key",
                &["media:read".into(), "media:list".into()],
                None,
            )
            .await
            .expect("update access key")
    );
    repository
        .record_replay_nonce(
            &access_key.access_key_id,
            "contract-nonce",
            now + Duration::minutes(5),
            now,
        )
        .await
        .expect("record nonce");
    assert_eq!(
        repository
            .record_replay_nonce(
                &access_key.access_key_id,
                "contract-nonce",
                now + Duration::minutes(5),
                now,
            )
            .await,
        Err(RepositoryError::Conflict)
    );
    assert!(
        repository
            .revoke_access_key(&access_key.access_key_id, application_id, now)
            .await
            .expect("revoke access key")
    );
    assert!(
        repository
            .find_active_access_key(&access_key.access_key_id, now)
            .await
            .expect("revoked key is inactive")
            .is_none()
    );
    repository
        .record_audit(&AuditEvent {
            id: "audit-application-delete-contract".to_owned(),
            application_id,
            actor_type: "user".to_owned(),
            actor_id: user_id.to_string(),
            action: "application.delete".to_owned(),
            target_type: "application".to_owned(),
            target_id: application_id.to_string(),
            request_id: "request-application-delete-contract".to_owned(),
            summary: json!({ "retained": true }),
            created_at: now,
        })
        .await
        .expect("record application audit event");
    assert!(
        repository
            .delete_application_for_user(user_id, "app_control_plane_contract")
            .await
            .expect("delete empty application")
    );
    assert!(
        repository
            .find_application_by_id(application_id)
            .await
            .expect("find deleted application")
            .is_none()
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audit_logs WHERE application_id = $1",)
            .bind(application_id.as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("count retained audit events"),
        1
    );
}

async fn control_plane_schema_contract(repository: &PostgresRepository) {
    let row = sqlx::query(
        "SELECT \
            to_regclass('public.sessions')::text AS sessions, \
            to_regclass('public.one_time_tokens')::text AS one_time_tokens, \
            to_regclass('public.access_keys')::text AS access_keys, \
            to_regclass('public.replay_nonces')::text AS replay_nonces, \
            to_regclass('public.idempotency_keys')::text AS idempotency_keys, \
            to_regclass('public.audit_logs')::text AS audit_logs, \
            to_regclass('public.deployment_bootstrap')::text AS deployment_bootstrap, \
            to_regclass('public.sessions_active_user_idx')::text AS sessions_active_user_idx, \
            to_regclass('public.users_role_status_created_idx')::text AS users_role_status_created_idx",
    )
    .fetch_one(repository.pool())
    .await
    .expect("inspect control-plane schema");
    for field in [
        "sessions",
        "one_time_tokens",
        "access_keys",
        "replay_nonces",
        "idempotency_keys",
        "audit_logs",
        "deployment_bootstrap",
        "sessions_active_user_idx",
        "users_role_status_created_idx",
    ] {
        assert_eq!(
            row.try_get::<Option<String>, _>(field)
                .expect("decode schema object"),
            Some(field.to_owned()),
            "missing PostgreSQL control-plane object {field}"
        );
    }

    let jsonb_columns = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM information_schema.columns \
         WHERE table_schema = 'public' AND data_type = 'jsonb' AND ( \
            (table_name = 'access_keys' AND column_name = 'permissions') OR \
            (table_name = 'audit_logs' AND column_name = 'summary') \
         )",
    )
    .fetch_one(repository.pool())
    .await
    .expect("inspect control-plane JSONB columns");
    assert_eq!(jsonb_columns, 2);

    let now = postgres_now();
    sqlx::query(
        "INSERT INTO users \
         (id, email_normalized, password_hash, status, system_role, created_at, updated_at) \
         VALUES ($1, $2, 'contract-hash', 'pending_verification', 'user', $3, $3)",
    )
    .bind(Uuid::new_v4())
    .bind(format!("pending-{}@contract.invalid", Uuid::new_v4()))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert pending-verification user");

    let invalid_status = sqlx::query(
        "INSERT INTO users \
         (id, email_normalized, password_hash, status, system_role, created_at, updated_at) \
         VALUES ($1, $2, 'contract-hash', 'disabled', 'user', $3, $3)",
    )
    .bind(Uuid::new_v4())
    .bind(format!("invalid-{}@contract.invalid", Uuid::new_v4()))
    .bind(now)
    .execute(repository.pool())
    .await;
    assert!(
        invalid_status.is_err(),
        "invalid user status must be rejected"
    );
}

async fn administration_and_webhook_contract(repository: &PostgresRepository, fixture: &Fixture) {
    let now = fixture.now + Duration::hours(1);
    let first_admin = insert_verified_user_with_app(repository, "admin-one", now).await;
    let second_admin = insert_verified_user_with_app(repository, "admin-two", now).await;
    assert_eq!(
        repository
            .bootstrap_admin(&first_admin.2, now)
            .await
            .expect("bootstrap initial admin"),
        AdminBootstrapOutcome::Completed(first_admin.0)
    );
    assert_eq!(
        repository
            .bootstrap_admin(&second_admin.2, now + Duration::seconds(1))
            .await
            .expect("bootstrap is one-time"),
        AdminBootstrapOutcome::AlreadyCompleted
    );
    repository
        .create_session(
            first_admin.0,
            "admin-contract-session",
            "admin-contract-csrf",
            now + Duration::hours(1),
            now,
        )
        .await
        .expect("create admin session");
    assert_eq!(
        repository
            .transition_user_status(
                first_admin.0,
                first_admin.0,
                "suspended",
                "admin-final-protection",
                now + Duration::seconds(2),
            )
            .await,
        Err(RepositoryError::Conflict)
    );
    sqlx::query("UPDATE users SET system_role = 'admin' WHERE id = $1")
        .bind(second_admin.0.as_uuid())
        .execute(repository.pool())
        .await
        .expect("promote second contract admin");
    let suspended = repository
        .transition_user_status(
            second_admin.0,
            first_admin.0,
            "suspended",
            "admin-suspend-contract",
            now + Duration::seconds(3),
        )
        .await
        .expect("suspend admin with peer protection");
    assert_eq!(suspended.status, "suspended");
    assert!(
        repository
            .list_active_sessions(
                first_admin.0,
                "admin-contract-session",
                now + Duration::seconds(4),
            )
            .await
            .expect("suspension revokes sessions")
            .is_empty()
    );
    assert_eq!(
        repository
            .transition_user_status(
                second_admin.0,
                first_admin.0,
                "active",
                "admin-reactivate-contract",
                now + Duration::seconds(5),
            )
            .await
            .expect("reactivate verified admin")
            .status,
        "active"
    );

    let audit = AuditEvent {
        id: format!("contract-audit:{}", Uuid::new_v4()),
        application_id: fixture.application_id,
        actor_type: "system".into(),
        actor_id: "contract".into(),
        action: "contract.audit".into(),
        target_type: "fixture".into(),
        target_id: "audit".into(),
        request_id: "contract-request".into(),
        summary: json!({"sensitive": false}),
        created_at: now,
    };
    repository
        .record_audit(&audit)
        .await
        .expect("append audit event");
    assert!(
        repository
            .list_audit(fixture.application_id, 10)
            .await
            .expect("list scoped audit")
            .iter()
            .any(|event| event.id == audit.id)
    );
    assert!(
        repository
            .list_admin_audit(100)
            .await
            .expect("list global audit")
            .iter()
            .any(|event| event.id == audit.id)
    );
    assert!(
        repository
            .list_admin_users(100)
            .await
            .expect("list admin users")
            .iter()
            .any(|user| user.id == first_admin.0)
    );
    assert!(
        repository
            .list_admin_applications(100)
            .await
            .expect("list admin applications")
            .iter()
            .any(|application| application.id == fixture.application_id)
    );
    let updated_quota = repository
        .update_application_quota(
            second_admin.0,
            first_admin.1,
            8192,
            "admin-quota-contract",
            now + Duration::seconds(6),
        )
        .await
        .expect("increase application quota");
    assert_eq!(updated_quota.quota.quota_bytes, 8192);
    repository
        .reserve_quota(first_admin.1, 1024)
        .await
        .expect("reserve quota for lower-bound check");
    assert_eq!(
        repository
            .update_application_quota(
                second_admin.0,
                first_admin.1,
                1023,
                "admin-quota-too-small",
                now + Duration::seconds(7),
            )
            .await,
        Err(RepositoryError::Conflict)
    );
    repository
        .release_quota(first_admin.1, 1024)
        .await
        .expect("release quota after lower-bound check");
    assert!(
        repository
            .list_admin_audit(100)
            .await
            .expect("list quota audit")
            .iter()
            .any(|event| {
                event.action == "application.quota_changed"
                    && event.target_id == first_admin.1.to_string()
            })
    );
    repository
        .admin_storage_summary()
        .await
        .expect("admin storage summary");
    repository
        .admin_metrics_snapshot()
        .await
        .expect("admin metrics snapshot");

    let endpoint_id = format!("wh_contract_{}", Uuid::new_v4().simple());
    let endpoint = NewWebhookEndpoint {
        id: endpoint_id.clone(),
        application_id: fixture.application_id,
        url: "https://webhook.contract.invalid/media".into(),
        secret_ciphertext: "contract-webhook-ciphertext".into(),
        secret_key_version: 7,
        subscribed_events: vec!["media.uploaded".into()],
        enabled: true,
        created_at: now,
    };
    repository
        .create_webhook_endpoint(&endpoint)
        .await
        .expect("create text-ID webhook endpoint");
    assert_eq!(
        repository
            .find_webhook_endpoint(fixture.application_id, &endpoint_id)
            .await
            .expect("find webhook endpoint")
            .expect("webhook endpoint exists")
            .secret_key_version,
        7
    );
    assert!(
        repository
            .referenced_secret_key_versions()
            .await
            .expect("list referenced key versions")
            .contains(&7)
    );

    let first_event = format!("contract.webhook:{}", Uuid::new_v4());
    let second_event = format!("contract.webhook:{}", Uuid::new_v4());
    insert_contract_outbox(
        repository,
        &first_event,
        fixture.application_id,
        "media.uploaded",
        now,
    )
    .await;
    insert_contract_outbox(
        repository,
        &second_event,
        fixture.application_id,
        "media.uploaded",
        now + Duration::microseconds(1),
    )
    .await;
    assert_eq!(
        repository
            .materialize_webhook_deliveries(&first_event)
            .await
            .expect("materialize first delivery"),
        1
    );
    assert_eq!(
        repository
            .materialize_webhook_deliveries(&first_event)
            .await
            .expect("materialization is idempotent"),
        0
    );
    repository
        .materialize_webhook_deliveries(&second_event)
        .await
        .expect("materialize second delivery");
    let first_page = repository
        .list_webhook_delivery_history(
            fixture.application_id,
            &endpoint_id,
            &WebhookDeliveryHistoryQuery {
                status: Some(WebhookDeliveryHistoryStatus::Pending),
                cursor: None,
                limit: 1,
            },
        )
        .await
        .expect("list first webhook history page");
    assert!(first_page.has_more);
    let cursor_item = first_page.items.last().expect("first page item");
    let second_page = repository
        .list_webhook_delivery_history(
            fixture.application_id,
            &endpoint_id,
            &WebhookDeliveryHistoryQuery {
                status: Some(WebhookDeliveryHistoryStatus::Pending),
                cursor: Some(WebhookDeliveryHistoryCursor {
                    row_id: cursor_item.row_id,
                }),
                limit: 1,
            },
        )
        .await
        .expect("list second webhook history page");
    assert_eq!(second_page.items.len(), 1);
    assert_ne!(second_page.items[0].event_id, cursor_item.event_id);

    let claim_at = now + Duration::seconds(1);
    let deliveries = repository
        .claim_webhook_deliveries(claim_at, claim_at + Duration::seconds(30), 10)
        .await
        .expect("claim webhook deliveries");
    assert_eq!(deliveries.len(), 2);
    let replay_target = deliveries[0].delivery.event.id.clone();
    for delivery in deliveries {
        assert!(
            repository
                .mark_webhook_delivery_delivered_with_status(
                    &delivery.delivery.event.id,
                    &endpoint_id,
                    &delivery.lease_token,
                    claim_at + Duration::seconds(1),
                    Some(204),
                )
                .await
                .expect("acknowledge webhook delivery")
        );
    }
    assert!(
        repository
            .replay_webhook_delivery(
                fixture.application_id,
                &endpoint_id,
                &replay_target,
                claim_at + Duration::seconds(2),
            )
            .await
            .expect("replay delivered webhook")
    );
    let replayed = repository
        .claim_webhook_deliveries(
            claim_at + Duration::seconds(3),
            claim_at + Duration::seconds(30),
            10,
        )
        .await
        .expect("claim replayed webhook");
    assert_eq!(replayed.len(), 1);
    let disposition = repository
        .record_webhook_delivery_failure_with_status(
            &replay_target,
            &endpoint_id,
            &replayed[0].lease_token,
            claim_at + Duration::seconds(4),
            claim_at + Duration::seconds(5),
            1,
            Some(503),
            "contract endpoint unavailable",
        )
        .await
        .expect("record terminal webhook failure");
    assert!(matches!(
        disposition,
        Some(WebhookDeliveryFailureDisposition::DeadLettered { .. })
    ));
    let dead_letters = repository
        .list_webhook_delivery_history(
            fixture.application_id,
            &endpoint_id,
            &WebhookDeliveryHistoryQuery {
                status: Some(WebhookDeliveryHistoryStatus::DeadLettered),
                cursor: None,
                limit: 10,
            },
        )
        .await
        .expect("list dead-letter history");
    assert_eq!(dead_letters.items.len(), 1);
    assert_eq!(dead_letters.items[0].event_id, replay_target);
    assert_eq!(dead_letters.items[0].last_response_status, Some(503));
    assert_eq!(dead_letters.items[0].replay_count, 1);
    assert_eq!(
        dead_letters.items[0].last_error.as_deref(),
        Some("contract endpoint unavailable")
    );

    let unsubscribed_event = format!("contract.unsubscribed:{}", Uuid::new_v4());
    insert_contract_outbox(
        repository,
        &unsubscribed_event,
        fixture.application_id,
        "contract.unsubscribed",
        now,
    )
    .await;
    assert_eq!(
        repository
            .materialize_webhook_deliveries(&unsubscribed_event)
            .await
            .expect("materialize unsubscribed event"),
        0
    );
    assert!(
        repository
            .finalize_unsubscribed_outbox_events(100)
            .await
            .expect("finalize unsubscribed outbox")
            >= 1
    );
    assert!(
        sqlx::query_scalar::<_, Option<OffsetDateTime>>(
            "SELECT delivered_at FROM outbox_events WHERE id = $1",
        )
        .bind(&unsubscribed_event)
        .fetch_one(repository.pool())
        .await
        .expect("read finalized outbox")
        .is_some()
    );

    let updated_at = now + Duration::minutes(1);
    assert!(
        repository
            .update_webhook_endpoint(
                fixture.application_id,
                &endpoint_id,
                &WebhookEndpointUpdate {
                    url: "https://webhook.contract.invalid/updated".into(),
                    secret_ciphertext: "rotated-contract-ciphertext".into(),
                    secret_key_version: 8,
                    subscribed_events: vec!["media.deleted".into()],
                    enabled: false,
                    updated_at,
                },
            )
            .await
            .expect("update webhook endpoint")
    );
    let updated_endpoint = repository
        .list_webhook_endpoints(fixture.application_id)
        .await
        .expect("list webhook endpoints")
        .into_iter()
        .find(|endpoint| endpoint.id == endpoint_id)
        .expect("updated endpoint is listed");
    assert_eq!(
        updated_endpoint.url,
        "https://webhook.contract.invalid/updated"
    );
    assert_eq!(updated_endpoint.secret_key_version, 8);
    assert!(!updated_endpoint.enabled);
    assert!(
        repository
            .delete_webhook_endpoint(fixture.application_id, &endpoint_id)
            .await
            .expect("delete webhook endpoint")
    );
}

async fn insert_verified_user_with_app(
    repository: &PostgresRepository,
    label: &str,
    now: OffsetDateTime,
) -> (UserId, ApplicationId, String) {
    let user_id = UserId::new();
    let application_id = ApplicationId::new();
    let email = format!("{label}-{}@contract.invalid", Uuid::new_v4());
    sqlx::query(
        "INSERT INTO users \
         (id, email_normalized, password_hash, email_verified_at, status, created_at, updated_at) \
         VALUES ($1, $2, 'contract-hash', $3, 'active', $3, $3)",
    )
    .bind(user_id.as_uuid())
    .bind(&email)
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert verified contract user");
    sqlx::query(
        "INSERT INTO applications \
         (id, user_id, name, app_id, quota_bytes, created_at, updated_at) \
         VALUES ($1, $2, $3, $4, 4096, $5, $5)",
    )
    .bind(application_id.as_uuid())
    .bind(user_id.as_uuid())
    .bind(label)
    .bind(format!("app_{application_id}"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert admin contract application");
    (user_id, application_id, email)
}

async fn insert_contract_outbox(
    repository: &PostgresRepository,
    event_id: &str,
    application_id: ApplicationId,
    event_type: &str,
    created_at: OffsetDateTime,
) {
    sqlx::query(
        "INSERT INTO outbox_events \
         (id, application_id, event_type, aggregate_id, payload, attempts, available_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, 0, $6, $6)",
    )
    .bind(event_id)
    .bind(application_id.as_uuid())
    .bind(event_type)
    .bind(event_id)
    .bind(Json(json!({"event_id": event_id})))
    .bind(created_at)
    .execute(repository.pool())
    .await
    .expect("insert contract outbox event");
}

struct Fixture {
    application_id: ApplicationId,
    bucket_id: BucketId,
    now: OffsetDateTime,
}

impl Fixture {
    async fn create(repository: &PostgresRepository) -> Self {
        let now = postgres_now();
        let user_id = UserId::new();
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        sqlx::query(
            "INSERT INTO users (id, email_normalized, password_hash, created_at, updated_at) \
             VALUES ($1, $2, 'contract-hash', $3, $3)",
        )
        .bind(user_id.as_uuid())
        .bind(format!("{}@contract.invalid", user_id))
        .bind(now)
        .execute(repository.pool())
        .await
        .expect("insert fixture user");
        sqlx::query(
            "INSERT INTO applications (id, user_id, name, app_id, quota_bytes, created_at, updated_at) \
             VALUES ($1, $2, 'contract', $3, 1000, $4, $4)",
        )
        .bind(application_id.as_uuid())
        .bind(user_id.as_uuid())
        .bind(format!("app_{application_id}"))
        .bind(now)
        .execute(repository.pool())
        .await
        .expect("insert fixture application");
        sqlx::query(
            "INSERT INTO buckets (id, application_id, name, visibility, allowed_mime_types, \
             created_at, updated_at) VALUES ($1, $2, 'contract', 'private', $3, $4, $4)",
        )
        .bind(bucket_id.as_uuid())
        .bind(application_id.as_uuid())
        .bind(Json(json!([])))
        .bind(now)
        .execute(repository.pool())
        .await
        .expect("insert fixture bucket");
        Self {
            application_id,
            bucket_id,
            now,
        }
    }
}

async fn quota_and_media_contract(repository: &PostgresRepository, fixture: &Fixture) {
    let mut media = fixture.media("objects/first.bin", 4);
    repository
        .reserve_quota(fixture.application_id, media.size())
        .await
        .expect("reserve quota");
    let lease_token = create_leased_upload(repository, &media).await;
    assert!(matches!(
        repository
            .create_uploading(
                media.clone(),
                &format!("temporary/{}", media.id()),
                &lease_token,
                fixture.now + Duration::hours(1),
            )
            .await,
        Err(RepositoryError::Conflict)
    ));

    let endpoint_id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO webhook_endpoints (id, application_id, url, secret_ciphertext, \
         secret_key_version, subscribed_events, created_at, updated_at) \
         VALUES ($1, $2, 'https://contract.invalid/hook', 'ciphertext', 1, $3, $4, $4)",
    )
    .bind(&endpoint_id)
    .bind(fixture.application_id.as_uuid())
    .bind(Json(json!(["media.uploaded", "media.metadata_updated"])))
    .bind(fixture.now)
    .execute(repository.pool())
    .await
    .expect("insert webhook endpoint");

    let committed_at = fixture.now + Duration::seconds(1);
    let event = OutboxEvent::media_uploaded(&media, committed_at);
    media = repository
        .commit_upload(media.id(), &lease_token, committed_at, event.clone())
        .await
        .expect("commit upload");
    let replay = repository
        .commit_upload(media.id(), &lease_token, committed_at, event)
        .await
        .expect("idempotent commit replay");
    assert_eq!(replay, media);
    let quota = sqlx::query("SELECT used_bytes, reserved_bytes FROM applications WHERE id = $1")
        .bind(fixture.application_id.as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("read quota");
    assert_eq!(quota.get::<i64, _>("used_bytes"), 4);
    assert_eq!(quota.get::<i64, _>("reserved_bytes"), 0);

    let expected_revision = media.revision();
    media
        .set_display_name(
            "updated",
            expected_revision,
            committed_at + Duration::seconds(1),
        )
        .expect("mutate media");
    let update_event = OutboxEvent::media_metadata_updated(&media, media.updated_at());
    repository
        .update_media(media.clone(), expected_revision, update_event.clone())
        .await
        .expect("conditional update");
    assert!(matches!(
        repository
            .update_media(media, expected_revision, update_event)
            .await,
        Err(RepositoryError::Conflict)
    ));

    let now = fixture.now + Duration::seconds(3);
    let first_claim = repository
        .claim_webhook_deliveries(now, now + Duration::seconds(30), 10)
        .await
        .expect("claim webhook deliveries");
    assert_eq!(first_claim.len(), 2);
    for delivery in first_claim {
        let disposition = repository
            .record_webhook_delivery_failure(
                &delivery.delivery.event.id,
                &delivery.delivery.endpoint.id,
                &delivery.lease_token,
                now + Duration::seconds(1),
                now + Duration::seconds(10),
                1,
                "contract failure",
            )
            .await
            .expect("record delivery failure")
            .expect("lease owns delivery");
        assert!(matches!(
            disposition,
            WebhookDeliveryFailureDisposition::DeadLettered { .. }
        ));
    }
}

async fn media_query_and_lifecycle_contract(repository: &PostgresRepository, fixture: &Fixture) {
    let prefix = format!("lifecycle-{}/", Uuid::new_v4());
    let first = activate_media(
        repository,
        fixture.media_at(
            &format!("{prefix}first.bin"),
            3,
            fixture.now + Duration::seconds(40),
            None,
        ),
        fixture.now + Duration::seconds(43),
    )
    .await;
    let second = activate_media(
        repository,
        fixture.media_at(
            &format!("{prefix}second.bin"),
            5,
            fixture.now + Duration::seconds(41),
            Some(fixture.now + Duration::seconds(50)),
        ),
        fixture.now + Duration::seconds(44),
    )
    .await;
    let third = activate_media(
        repository,
        fixture.media_at(
            &format!("{prefix}third.bin"),
            7,
            fixture.now + Duration::seconds(42),
            None,
        ),
        fixture.now + Duration::seconds(45),
    )
    .await;

    assert_eq!(
        repository
            .find_media_by_id(second.id())
            .await
            .expect("find media by id")
            .expect("media exists"),
        second
    );
    assert!(
        repository
            .list_media(fixture.application_id, 100)
            .await
            .expect("list media")
            .iter()
            .any(|media| media.id() == third.id())
    );

    let first_page = repository
        .list_media_page(
            fixture.application_id,
            &MediaListQuery {
                bucket_id: Some(fixture.bucket_id),
                state: Some(mediahub_core::MediaState::Active),
                mime: Some("application/octet-stream".to_owned()),
                created_from: Some(fixture.now + Duration::seconds(39)),
                created_before: Some(fixture.now + Duration::seconds(43)),
                object_key_prefix: Some(prefix.clone()),
                cursor: None,
                limit: 2,
            },
        )
        .await
        .expect("list filtered first page");
    assert!(first_page.has_more);
    assert_eq!(
        first_page.items.iter().map(Media::id).collect::<Vec<_>>(),
        vec![third.id(), second.id()]
    );
    let cursor_media = first_page.items.last().expect("first page cursor");
    let second_page = repository
        .list_media_page(
            fixture.application_id,
            &MediaListQuery {
                bucket_id: Some(fixture.bucket_id),
                state: Some(mediahub_core::MediaState::Active),
                mime: Some("application/octet-stream".to_owned()),
                created_from: Some(fixture.now + Duration::seconds(39)),
                created_before: Some(fixture.now + Duration::seconds(43)),
                object_key_prefix: Some(prefix.clone()),
                cursor: Some(MediaListCursor {
                    created_at: cursor_media.created_at(),
                    id: cursor_media.id(),
                }),
                limit: 2,
            },
        )
        .await
        .expect("list filtered second page");
    assert!(!second_page.has_more);
    assert_eq!(second_page.items.len(), 1);
    assert_eq!(second_page.items[0].id(), first.id());

    let directory_prefix = format!("directory-{}/", Uuid::new_v4());
    let direct_first = activate_media(
        repository,
        fixture.media_at(
            &format!("{directory_prefix}a.txt"),
            1,
            fixture.now + Duration::seconds(60),
            None,
        ),
        fixture.now + Duration::seconds(61),
    )
    .await;
    activate_media(
        repository,
        fixture.media_at(
            &format!("{directory_prefix}images/avatar/photo.jpg"),
            2,
            fixture.now + Duration::seconds(62),
            None,
        ),
        fixture.now + Duration::seconds(63),
    )
    .await;
    activate_media(
        repository,
        fixture.media_at(
            &format!("{directory_prefix}images/banner.jpg"),
            3,
            fixture.now + Duration::seconds(64),
            None,
        ),
        fixture.now + Duration::seconds(65),
    )
    .await;
    activate_media(
        repository,
        fixture.media_at(
            &format!("{directory_prefix}uploads/archive.7z"),
            4,
            fixture.now + Duration::seconds(66),
            None,
        ),
        fixture.now + Duration::seconds(67),
    )
    .await;
    let direct_last = activate_media(
        repository,
        fixture.media_at(
            &format!("{directory_prefix}z.txt"),
            5,
            fixture.now + Duration::seconds(68),
            None,
        ),
        fixture.now + Duration::seconds(69),
    )
    .await;

    let directory_first_page = repository
        .list_media_directory_page(
            fixture.application_id,
            &MediaDirectoryListQuery {
                bucket_id: fixture.bucket_id,
                state: Some(mediahub_core::MediaState::Active),
                mime: Some("application/octet-stream".to_owned()),
                created_from: None,
                created_before: None,
                object_key_prefix: directory_prefix.clone(),
                cursor: None,
                limit: 2,
            },
        )
        .await
        .expect("list first directory page");
    assert!(directory_first_page.items.is_empty());
    assert_eq!(
        directory_first_page.common_prefixes,
        vec![
            format!("{directory_prefix}images/"),
            format!("{directory_prefix}uploads/")
        ]
    );
    let directory_cursor = directory_first_page
        .next_cursor
        .expect("directory first page has cursor");

    let directory_second_page = repository
        .list_media_directory_page(
            fixture.application_id,
            &MediaDirectoryListQuery {
                bucket_id: fixture.bucket_id,
                state: Some(mediahub_core::MediaState::Active),
                mime: Some("application/octet-stream".to_owned()),
                created_from: None,
                created_before: None,
                object_key_prefix: directory_prefix.clone(),
                cursor: Some(directory_cursor),
                limit: 2,
            },
        )
        .await
        .expect("list second directory page");
    assert_eq!(
        directory_second_page.items,
        vec![direct_first.clone(), direct_last.clone()]
    );
    assert!(directory_second_page.common_prefixes.is_empty());
    assert!(directory_second_page.next_cursor.is_none());

    let nested_directory = repository
        .list_media_directory_page(
            fixture.application_id,
            &MediaDirectoryListQuery {
                bucket_id: fixture.bucket_id,
                state: Some(mediahub_core::MediaState::Active),
                mime: Some("application/octet-stream".to_owned()),
                created_from: None,
                created_before: None,
                object_key_prefix: format!("{directory_prefix}images/"),
                cursor: None,
                limit: 100,
            },
        )
        .await
        .expect("list nested directory");
    assert_eq!(
        nested_directory
            .items
            .iter()
            .map(|media| media.object_key().to_owned())
            .collect::<Vec<_>>(),
        vec![format!("{directory_prefix}images/banner.jpg")]
    );
    assert_eq!(
        nested_directory.common_prefixes,
        vec![format!("{directory_prefix}images/avatar/")]
    );
    assert!(nested_directory.next_cursor.is_none());

    let s3_first_page = repository
        .list_s3_media_page(
            fixture.application_id,
            &S3MediaListQuery {
                bucket_id: fixture.bucket_id,
                object_key_prefix: directory_prefix.clone(),
                start_after: None,
                delimiter: true,
                limit: 2,
            },
        )
        .await
        .expect("list first S3 delimiter page");
    assert_eq!(s3_first_page.items, vec![direct_first.clone()]);
    assert_eq!(
        s3_first_page.common_prefixes,
        vec![format!("{directory_prefix}images/")]
    );
    let s3_cursor = s3_first_page.next_cursor.expect("S3 page cursor");
    assert_eq!(s3_cursor, format!("{directory_prefix}images/"));

    let s3_second_page = repository
        .list_s3_media_page(
            fixture.application_id,
            &S3MediaListQuery {
                bucket_id: fixture.bucket_id,
                object_key_prefix: directory_prefix.clone(),
                start_after: Some(s3_cursor),
                delimiter: true,
                limit: 2,
            },
        )
        .await
        .expect("list second S3 delimiter page");
    assert_eq!(s3_second_page.items, vec![direct_last.clone()]);
    assert_eq!(
        s3_second_page.common_prefixes,
        vec![format!("{directory_prefix}uploads/")]
    );
    assert!(s3_second_page.next_cursor.is_none());

    let s3_flat_page = repository
        .list_s3_media_page(
            fixture.application_id,
            &S3MediaListQuery {
                bucket_id: fixture.bucket_id,
                object_key_prefix: directory_prefix.clone(),
                start_after: Some(direct_first.object_key().to_owned()),
                delimiter: false,
                limit: 100,
            },
        )
        .await
        .expect("list flat S3 page");
    assert_eq!(s3_flat_page.items.len(), 4);
    assert!(s3_flat_page.common_prefixes.is_empty());
    assert!(s3_flat_page.next_cursor.is_none());
    assert!(
        s3_flat_page
            .items
            .windows(2)
            .all(|pair| pair[0].object_key() < pair[1].object_key())
    );

    let unicode_prefix = format!("s3-unicode-{}/", Uuid::new_v4());
    let unicode_keys =
        ["A.bin", "z.bin", "é.bin", "中.bin"].map(|suffix| format!("{unicode_prefix}{suffix}"));
    for (index, key) in unicode_keys.iter().enumerate() {
        activate_media(
            repository,
            fixture.media(key, 1),
            fixture.now + Duration::seconds(60 + i64::try_from(index).expect("small index")),
        )
        .await;
    }
    let unicode_first = repository
        .list_s3_media_page(
            fixture.application_id,
            &S3MediaListQuery {
                bucket_id: fixture.bucket_id,
                object_key_prefix: unicode_prefix.clone(),
                start_after: None,
                delimiter: false,
                limit: 2,
            },
        )
        .await
        .expect("list first UTF-8 byte-order S3 page");
    assert_eq!(
        unicode_first
            .items
            .iter()
            .map(|media| media.object_key())
            .collect::<Vec<_>>(),
        vec![unicode_keys[0].as_str(), unicode_keys[1].as_str()]
    );
    assert_eq!(
        unicode_first.next_cursor.as_deref(),
        Some(unicode_keys[1].as_str())
    );
    let unicode_second = repository
        .list_s3_media_page(
            fixture.application_id,
            &S3MediaListQuery {
                bucket_id: fixture.bucket_id,
                object_key_prefix: unicode_prefix,
                start_after: unicode_first.next_cursor,
                delimiter: false,
                limit: 2,
            },
        )
        .await
        .expect("list second UTF-8 byte-order S3 page");
    assert_eq!(
        unicode_second
            .items
            .iter()
            .map(|media| media.object_key())
            .collect::<Vec<_>>(),
        vec![unicode_keys[2].as_str(), unicode_keys[3].as_str()]
    );
    assert!(unicode_second.next_cursor.is_none());

    assert!(
        repository
            .list_expired_media(fixture.now + Duration::seconds(51), 100)
            .await
            .expect("list expired media")
            .iter()
            .any(|media| media.id() == second.id())
    );
    let policy = BucketPolicy::unrestricted(Visibility::Private)
        .with_lifecycle_rules(vec![
            LifecycleRule::KeepLatest {
                id: "contract-latest".to_owned(),
                enabled: true,
                prefix: prefix.clone(),
                count: 1,
            },
            LifecycleRule::ExpireAfter {
                id: "contract-expiry".to_owned(),
                enabled: true,
                prefix: prefix.clone(),
                duration_seconds: 60,
            },
        ])
        .expect("valid lifecycle rules");
    assert!(
        repository
            .update_bucket_policy(
                fixture.application_id,
                "contract",
                &policy,
                fixture.now + Duration::seconds(52),
            )
            .await
            .expect("persist lifecycle policy")
    );
    assert!(
        repository
            .list_lifecycle_buckets()
            .await
            .expect("list lifecycle buckets")
            .iter()
            .any(|bucket| bucket.id() == fixture.bucket_id)
    );
    let surplus = repository
        .list_keep_latest_surplus(fixture.application_id, fixture.bucket_id, &prefix, 1, 100)
        .await
        .expect("list keep-latest surplus");
    assert_eq!(
        surplus.iter().map(Media::id).collect::<Vec<_>>(),
        vec![first.id(), second.id()]
    );
    let expire_after = repository
        .list_expire_after_due(
            fixture.application_id,
            fixture.bucket_id,
            &prefix,
            fixture.now + Duration::seconds(42),
            100,
        )
        .await
        .expect("list expire-after candidates");
    assert_eq!(
        expire_after.iter().map(Media::id).collect::<Vec<_>>(),
        vec![first.id(), second.id(), third.id()]
    );

    let scheduled_at = fixture.now + Duration::seconds(53);
    let event = OutboxEvent::media_delete_scheduled(&first, scheduled_at, "contract");
    let pending = repository
        .schedule_delete(first.id(), scheduled_at, event)
        .await
        .expect("schedule media deletion");
    assert_eq!(pending.state(), mediahub_core::MediaState::DeletePending);
    assert!(
        repository
            .list_variant_storage_keys(first.id())
            .await
            .expect("list variant keys")
            .is_empty()
    );
    let used_before =
        sqlx::query_scalar::<_, i64>("SELECT used_bytes FROM applications WHERE id = $1")
            .bind(fixture.application_id.as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("read quota before deletion");
    let tombstone = repository
        .finalize_delete(first.id(), fixture.now + Duration::seconds(54))
        .await
        .expect("finalize media deletion");
    assert_eq!(tombstone.state(), mediahub_core::MediaState::Deleted);
    assert_eq!(tombstone.original_name(), None);
    assert_eq!(tombstone.display_name(), "deleted");
    assert_eq!(tombstone.extension(), None);
    assert!(tombstone.metadata().user().is_empty());
    assert!(tombstone.metadata().ai().is_empty());
    let used_after =
        sqlx::query_scalar::<_, i64>("SELECT used_bytes FROM applications WHERE id = $1")
            .bind(fixture.application_id.as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("read quota after deletion");
    assert_eq!(used_after, used_before - 3);
    assert_eq!(
        repository
            .finalize_delete(first.id(), fixture.now + Duration::seconds(55))
            .await
            .expect("replay finalized deletion")
            .state(),
        mediahub_core::MediaState::Deleted
    );
}

async fn activate_media(
    repository: &PostgresRepository,
    media: Media,
    committed_at: OffsetDateTime,
) -> Media {
    repository
        .reserve_quota(media.application_id(), media.size())
        .await
        .expect("reserve media quota");
    let lease_token = create_leased_upload(repository, &media).await;
    repository
        .commit_upload(
            media.id(),
            &lease_token,
            committed_at,
            OutboxEvent::media_uploaded(&media, committed_at),
        )
        .await
        .expect("activate media")
}

async fn upload_session_contract(repository: &PostgresRepository, fixture: &Fixture) {
    let session = UploadSession::new(
        NewUploadSession {
            id: UploadSessionId::new(),
            media_id: MediaId::new(),
            application_id: fixture.application_id,
            bucket_id: fixture.bucket_id,
            object_key: "sessions/cancelled.bin".to_owned(),
            original_name: None,
            display_name: "cancelled".to_owned(),
            extension: Some("bin".to_owned()),
            expected_size: 5,
            expected_mime: "application/octet-stream".to_owned(),
            storage_backend: "s3".to_owned(),
            storage_key: format!("contract/{}", Uuid::new_v4()),
            visibility_override: None,
            media_expires_at: None,
            client_metadata: ClientMetadata::default(),
            session_expires_at: fixture.now + Duration::minutes(15),
        },
        fixture.now,
    )
    .expect("create domain session");
    let mut idempotency = IdempotencyContext {
        application_id: fixture.application_id,
        operation_scope: "contract.upload.create".to_owned(),
        key: "atomic-upload-key".to_owned(),
        request_hash: "e".repeat(64),
        claim_token: String::new(),
    };
    let claim = repository
        .claim_idempotency_key(
            idempotency.application_id,
            &idempotency.operation_scope,
            &idempotency.key,
            &idempotency.request_hash,
            fixture.now + Duration::minutes(10),
            fixture.now,
        )
        .await
        .expect("claim upload idempotency key");
    idempotency.claim_token = match claim {
        IdempotencyClaim::Claimed(token) => token,
        claim => panic!("expected claimed idempotency key, got {claim:?}"),
    };
    let response = CompletedIdempotencyResponse {
        status: 201,
        payload: "{\"upload\":true}".to_owned(),
        resource_id: Some(session.id().to_string()),
    };
    repository
        .create_upload_session_and_complete_idempotency(
            &session,
            &idempotency,
            &response,
            fixture.now + Duration::seconds(1),
        )
        .await
        .expect("atomically persist upload session");
    assert_eq!(
        repository
            .claim_idempotency_key(
                idempotency.application_id,
                &idempotency.operation_scope,
                &idempotency.key,
                &idempotency.request_hash,
                fixture.now + Duration::minutes(10),
                fixture.now,
            )
            .await
            .expect("replay upload idempotency key"),
        IdempotencyClaim::Completed(response)
    );
    let first = repository
        .cancel_upload_session(session.id(), fixture.now + Duration::seconds(1))
        .await
        .expect("cancel session");
    let replay = repository
        .cancel_upload_session(session.id(), fixture.now + Duration::seconds(2))
        .await
        .expect("replay cancellation");
    assert!(matches!(first, UploadSessionCancellation::Cancelled(_)));
    assert!(matches!(
        replay,
        UploadSessionCancellation::AlreadyCancelled(_)
    ));
    let reserved =
        sqlx::query_scalar::<_, i64>("SELECT reserved_bytes FROM applications WHERE id = $1")
            .bind(fixture.application_id.as_uuid())
            .fetch_one(repository.pool())
            .await
            .expect("read reservation");
    assert_eq!(reserved, 0);
    assert!(
        repository
            .expire_upload_sessions(fixture.now + Duration::seconds(3), 10)
            .await
            .expect("do not clean a still-replayable cancelled target")
            .is_empty()
    );
    let cleanup_at = session.session_expires_at();
    let cleanup_candidates = repository
        .expire_upload_sessions(cleanup_at, 10)
        .await
        .expect("list cancelled session cleanup");
    assert_eq!(
        cleanup_candidates
            .iter()
            .map(UploadSession::id)
            .collect::<Vec<_>>(),
        vec![session.id()]
    );
    assert!(
        repository
            .complete_upload_session_cleanup(session.id())
            .await
            .expect("mark upload cleanup complete")
    );
    assert!(
        repository
            .expire_upload_sessions(cleanup_at + Duration::seconds(1), 10)
            .await
            .expect("relist cleaned sessions")
            .is_empty()
    );
    assert!(
        !repository
            .complete_upload_session_cleanup(session.id())
            .await
            .expect("replay cleanup acknowledgement")
    );
}

async fn async_job_contract(repository: &PostgresRepository, fixture: &Fixture) {
    let media = fixture.media("jobs/target.bin", 1);
    repository
        .reserve_quota(fixture.application_id, media.size())
        .await
        .expect("reserve job target quota");
    let lease_token = create_leased_upload(repository, &media).await;
    let committed_at = fixture.now + Duration::seconds(20);
    let media = repository
        .commit_upload(
            media.id(),
            &lease_token,
            committed_at,
            OutboxEvent::media_uploaded(&media, committed_at),
        )
        .await
        .expect("commit job target");
    let job = new_job(fixture, media.id(), "contract-job", "c");
    assert!(matches!(
        repository
            .create_async_job(job.clone(), &[media.id()])
            .await
            .expect("create async job"),
        AsyncJobCreation::Created(_)
    ));
    assert!(matches!(
        repository
            .create_async_job(job.clone(), &[media.id()])
            .await
            .expect("replay async job"),
        AsyncJobCreation::Existing(_)
    ));
    let changed = new_job(fixture, media.id(), "contract-job", "d");
    assert_eq!(
        repository
            .create_async_job(changed, &[media.id()])
            .await
            .expect("changed async request"),
        AsyncJobCreation::IdempotencyConflict
    );

    let claim_at = fixture.now + Duration::seconds(21);
    let mut claimed = repository
        .claim_async_jobs(claim_at, claim_at + Duration::seconds(30), 1)
        .await
        .expect("claim async job");
    assert_eq!(claimed.len(), 1);
    let leased = claimed.pop().expect("leased job");
    assert_eq!(leased.pending_media_ids, vec![media.id()]);
    let completed_at = claim_at + Duration::seconds(1);
    let result = AsyncJobItemResult::succeeded(
        leased.job.id(),
        fixture.application_id,
        media.id(),
        0,
        leased.job.attempt_count(),
        Some(json!({"deleted": true})),
        claim_at,
        completed_at,
    )
    .expect("item result");
    let completed = repository
        .complete_async_job(
            leased.job.id(),
            &leased.lease_token,
            std::slice::from_ref(&result),
            completed_at,
        )
        .await
        .expect("complete async job");
    assert!(matches!(completed, AsyncJobCompletion::Completed(_)));
    let replay = repository
        .complete_async_job(
            leased.job.id(),
            &leased.lease_token,
            &[result],
            completed_at + Duration::seconds(1),
        )
        .await
        .expect("replay async completion");
    assert!(matches!(replay, AsyncJobCompletion::AlreadyCompleted(_)));
    let stored = repository
        .find_async_job(fixture.application_id, leased.job.id())
        .await
        .expect("find async job")
        .expect("job exists");
    assert_eq!(stored.state(), AsyncJobState::Completed);
    let items = repository
        .list_async_job_items(fixture.application_id, leased.job.id())
        .await
        .expect("list async item results");
    assert_eq!(items.len(), 1);
}

fn new_job(fixture: &Fixture, _media_id: MediaId, key: &str, hash_char: &str) -> AsyncJob {
    AsyncJob::new(
        NewAsyncJob {
            id: AsyncJobId::new(),
            application_id: fixture.application_id,
            operation_scope: "media.batch".to_owned(),
            idempotency_key: key.to_owned(),
            request_hash: hash_char.repeat(64),
            request_id: Some("contract-request".to_owned()),
            action: AsyncJobAction::Delete,
            total_items: 1,
            max_attempts: 2,
        },
        fixture.now,
    )
    .expect("new async job")
}

async fn variant_contract(repository: &PostgresRepository, fixture: &Fixture) {
    let media = fixture.media("variants/source.png", 8);
    repository
        .reserve_quota(fixture.application_id, media.size())
        .await
        .expect("reserve variant source quota");
    let upload_lease_token = create_leased_upload(repository, &media).await;
    let committed_at = fixture.now + Duration::seconds(30);
    let media = repository
        .commit_upload(
            media.id(),
            &upload_lease_token,
            committed_at,
            OutboxEvent::media_uploaded(&media, committed_at),
        )
        .await
        .expect("commit variant source");
    let variant = NewVariant {
        id: VariantId::new(),
        media_id: media.id(),
        transform_key: format!("contract-{}", Uuid::new_v4()),
        parameters_json: "{\"width\":32}".to_owned(),
        processor_version: "contract-v1".to_owned(),
        format: VariantFormat::Webp,
        storage_backend: "s3".to_owned(),
        storage_key: format!("variants/{}.webp", Uuid::new_v4()),
        created_at: committed_at,
    };
    let lease_token = Uuid::new_v4().to_string();
    let claim = repository
        .claim_variant(
            variant.clone(),
            &lease_token,
            committed_at + Duration::seconds(60),
        )
        .await
        .expect("claim variant");
    assert!(matches!(claim, VariantClaim::Generate { .. }));
    let ready = repository
        .complete_variant(
            variant.id,
            &lease_token,
            32,
            32,
            128,
            committed_at + Duration::seconds(1),
        )
        .await
        .expect("complete variant")
        .expect("lease owns variant");
    assert_eq!(ready.width, Some(32));
    assert_eq!(
        repository
            .list_variant_storage_keys(media.id())
            .await
            .expect("list ready variant key"),
        vec![variant.storage_key.clone()]
    );
    let replay = repository
        .claim_variant(
            variant,
            &Uuid::new_v4().to_string(),
            committed_at + Duration::seconds(120),
        )
        .await
        .expect("claim ready variant");
    assert!(matches!(replay, VariantClaim::Ready(_)));
    let deleted_at = committed_at + Duration::seconds(121);
    repository
        .schedule_delete(
            media.id(),
            deleted_at,
            OutboxEvent::media_delete_scheduled(&media, deleted_at, "variant-contract"),
        )
        .await
        .expect("schedule variant source deletion");
    repository
        .finalize_delete(media.id(), deleted_at + Duration::seconds(1))
        .await
        .expect("finalize variant source deletion");
    assert!(
        repository
            .list_variant_storage_keys(media.id())
            .await
            .expect("list deleted variant rows")
            .is_empty()
    );
}

impl Fixture {
    fn media(&self, object_key: &str, size: u64) -> Media {
        self.media_at(object_key, size, self.now, None)
    }

    fn media_at(
        &self,
        object_key: &str,
        size: u64,
        created_at: OffsetDateTime,
        expire_at: Option<OffsetDateTime>,
    ) -> Media {
        Media::new(
            NewMedia {
                id: MediaId::new(),
                application_id: self.application_id,
                bucket_id: self.bucket_id,
                object_key: object_key.to_owned(),
                original_name: None,
                display_name: object_key.to_owned(),
                extension: Some("bin".to_owned()),
                storage_backend: "s3".to_owned(),
                storage_key: format!("contract/{}", Uuid::new_v4()),
                visibility_override: None,
                expire_at,
                system_metadata: SystemMetadata::new(
                    "application/octet-stream",
                    size,
                    None,
                    None,
                    None,
                    "a".repeat(64),
                )
                .expect("system metadata"),
                client_metadata: ClientMetadata::from_value(Value::Object(Default::default()))
                    .expect("client metadata"),
            },
            created_at,
        )
        .expect("media fixture")
    }
}

async fn create_leased_upload(repository: &PostgresRepository, media: &Media) -> String {
    let lease_token = Uuid::new_v4().to_string();
    repository
        .create_uploading(
            media.clone(),
            &format!("temporary/{}", media.id()),
            &lease_token,
            media.created_at() + Duration::hours(1),
        )
        .await
        .expect("create leased uploading media");
    lease_token
}

fn postgres_now() -> OffsetDateTime {
    let now = OffsetDateTime::now_utc();
    now.replace_nanosecond(now.nanosecond() / 1_000 * 1_000)
        .expect("microsecond precision is a valid nanosecond value")
}
