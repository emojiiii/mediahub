use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{
    DeleteS3IdentityPolicy, PutS3IdentityPolicy, S3IdentityPolicyDocument,
    S3IdentityPolicyRepository,
};
use mediahub_core::{ApplicationId, BucketId, OffsetDateTime, UserId};
use sqlx::error::DatabaseError;
use time::Duration;
use uuid::Uuid;

#[tokio::test]
async fn postgres_s3_identity_policy_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect identity policy database");
    repository
        .migrate()
        .await
        .expect("migrate identity policy database");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated identity policy database");

    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .expect("whole-second timestamp");
    let user_id = UserId::new();
    insert_user(&repository, user_id, now).await;
    let owner_application_id = ApplicationId::new();
    let other_application_id = ApplicationId::new();
    insert_application(&repository, user_id, owner_application_id, "owner", now).await;
    insert_application(&repository, user_id, other_application_id, "other", now).await;
    let owner_account_id = account_id(&repository, owner_application_id).await;
    insert_target_bucket(&repository, other_application_id, now).await;
    let target_bucket_owner_account_id = account_id(&repository, other_application_id).await;
    assert_ne!(owner_account_id, target_bucket_owner_account_id);
    let owner_key = "mh_ak_identity_owner";
    let other_key = "mh_ak_identity_other";
    insert_access_key(&repository, owner_application_id, owner_key, now).await;
    insert_access_key(&repository, other_application_id, other_key, now).await;

    let initial = repository
        .get_s3_identity_policy(owner_key)
        .await
        .expect("read initial policy")
        .expect("owner access key");
    assert_eq!(initial.identity.application_id, owner_application_id);
    assert_eq!(
        initial.identity.account_id.as_str(),
        owner_account_id.to_string()
    );
    assert_ne!(
        initial.identity.account_id.as_str(),
        target_bucket_owner_account_id.to_string(),
        "the caller account comes from the signing access key application, not the target bucket owner"
    );
    assert_eq!(initial.identity.access_key_id, owner_key);
    assert_eq!(initial.revision, 0);
    assert!(initial.policy.is_none());
    assert!(initial.updated_at.is_none());
    assert_eq!(
        legacy_permissions(&repository, owner_key).await,
        vec!["bucket:list", "object:read"],
        "legacy permissions remain separate and must not create a policy fallback"
    );

    let first_document = policy("s3:GetObject", "arn:aws:s3:::photos/*");
    let first_hash = first_document.sha256().to_owned();
    let first = repository
        .put_s3_identity_policy(&PutS3IdentityPolicy {
            application_id: owner_application_id,
            access_key_id: owner_key.into(),
            policy: first_document,
            updated_at: now + Duration::seconds(1),
        })
        .await
        .expect("put first policy")
        .expect("owner access key");
    assert_eq!(first.revision, 1);
    assert_eq!(
        first.policy.as_ref().map(S3IdentityPolicyDocument::sha256),
        Some(first_hash.as_str())
    );

    let cross_tenant = repository
        .put_s3_identity_policy(&PutS3IdentityPolicy {
            application_id: other_application_id,
            access_key_id: owner_key.into(),
            policy: policy("s3:PutObject", "arn:aws:s3:::photos/*"),
            updated_at: now + Duration::seconds(2),
        })
        .await
        .expect("cross-tenant put");
    assert!(cross_tenant.is_none());
    assert_eq!(load(&repository, owner_key).await.revision, 1);

    let replaced = repository
        .put_s3_identity_policy(&PutS3IdentityPolicy {
            application_id: owner_application_id,
            access_key_id: owner_key.into(),
            policy: policy("s3:PutObject", "arn:aws:s3:::photos/*"),
            updated_at: now + Duration::seconds(3),
        })
        .await
        .expect("replace policy")
        .expect("owner access key");
    assert_eq!(replaced.revision, 2);

    let isolated_delete = repository
        .delete_s3_identity_policy(&DeleteS3IdentityPolicy {
            application_id: other_application_id,
            access_key_id: owner_key.into(),
            updated_at: now + Duration::seconds(4),
        })
        .await
        .expect("cross-tenant delete");
    assert!(isolated_delete.is_none());

    let deleted = repository
        .delete_s3_identity_policy(&DeleteS3IdentityPolicy {
            application_id: owner_application_id,
            access_key_id: owner_key.into(),
            updated_at: now + Duration::seconds(5),
        })
        .await
        .expect("delete policy")
        .expect("owner access key");
    assert_eq!(deleted.revision, 3);
    assert!(deleted.policy.is_none());
    let deleted_at = deleted.updated_at;

    let replayed_delete = repository
        .delete_s3_identity_policy(&DeleteS3IdentityPolicy {
            application_id: owner_application_id,
            access_key_id: owner_key.into(),
            updated_at: now + Duration::seconds(6),
        })
        .await
        .expect("replay delete")
        .expect("owner access key");
    assert_eq!(replayed_delete.revision, 3);
    assert_eq!(replayed_delete.updated_at, deleted_at);

    let recreated = repository
        .put_s3_identity_policy(&PutS3IdentityPolicy {
            application_id: owner_application_id,
            access_key_id: owner_key.into(),
            policy: policy("s3:ListBucket", "arn:aws:s3:::photos"),
            updated_at: now + Duration::seconds(7),
        })
        .await
        .expect("recreate policy")
        .expect("owner access key");
    assert_eq!(recreated.revision, 4);
    assert!(recreated.policy.is_some());

    assert!(
        repository
            .get_s3_identity_policy("mh_ak_missing")
            .await
            .expect("read missing association")
            .is_none()
    );
    assert!(
        repository
            .put_s3_identity_policy(&PutS3IdentityPolicy {
                application_id: owner_application_id,
                access_key_id: "mh_ak_missing".into(),
                policy: policy("s3:GetObject", "arn:aws:s3:::photos/*"),
                updated_at: now + Duration::seconds(8),
            })
            .await
            .expect("put missing association")
            .is_none()
    );
    assert!(
        repository
            .delete_s3_identity_policy(&DeleteS3IdentityPolicy {
                application_id: owner_application_id,
                access_key_id: other_key.into(),
                updated_at: now + Duration::seconds(9),
            })
            .await
            .expect("delete invalid association")
            .is_none()
    );

    let invalid_json = sqlx::query(
        "UPDATE access_keys
            SET s3_identity_policy = '[]'::jsonb,
                s3_identity_policy_sha256 = repeat('a', 64),
                s3_identity_policy_revision = 1,
                s3_identity_policy_updated_at = $1
          WHERE access_key_id = $2",
    )
    .bind(now)
    .bind(other_key)
    .execute(repository.pool())
    .await
    .expect_err("database must reject a non-object policy");
    assert_eq!(database_code(&invalid_json).as_deref(), Some("23514"));

    let invalid_hash = sqlx::query(
        "UPDATE access_keys
            SET s3_identity_policy = '{}'::jsonb,
                s3_identity_policy_sha256 = 'short',
                s3_identity_policy_revision = 1,
                s3_identity_policy_updated_at = $1
          WHERE access_key_id = $2",
    )
    .bind(now)
    .bind(other_key)
    .execute(repository.pool())
    .await
    .expect_err("database must reject an invalid policy digest");
    assert_eq!(database_code(&invalid_hash).as_deref(), Some("23514"));
}

fn policy(action: &str, resource: &str) -> S3IdentityPolicyDocument {
    S3IdentityPolicyDocument::parse(
        format!(
            r#"{{"Version":"2012-10-17","Statement":{{"Effect":"Allow","Action":"{action}","Resource":"{resource}"}}}}"#
        )
        .as_bytes(),
    )
    .expect("valid identity policy")
}

async fn load(
    repository: &PostgresRepository,
    access_key_id: &str,
) -> mediahub_app::S3IdentityPolicySnapshot {
    repository
        .get_s3_identity_policy(access_key_id)
        .await
        .expect("load identity policy")
        .expect("access key")
}

async fn insert_user(repository: &PostgresRepository, user_id: UserId, now: OffsetDateTime) {
    sqlx::query(
        "INSERT INTO users (id, email_normalized, password_hash, created_at, updated_at)
         VALUES ($1, $2, 'identity-policy-contract-hash', $3, $3)",
    )
    .bind(user_id.as_uuid())
    .bind(format!("{user_id}@identity-policy.invalid"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert identity policy user");
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
    .bind(format!("identity-policy-{suffix}"))
    .bind(format!("app_identity_policy_{suffix}"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert identity policy application");
}

async fn insert_access_key(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    access_key_id: &str,
    now: OffsetDateTime,
) {
    sqlx::query(
        "INSERT INTO access_keys
         (id, application_id, access_key_id, secret_ciphertext, secret_key_version,
          secret_last_four, name, permissions, created_at)
         VALUES ($1, $2, $3, 'ciphertext', 1, 'last', 'identity contract',
                 '[\"bucket:list\", \"object:read\"]'::jsonb, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(application_id.as_uuid())
    .bind(access_key_id)
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert access key");
}

async fn insert_target_bucket(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    now: OffsetDateTime,
) {
    sqlx::query(
        "INSERT INTO buckets (id, application_id, name, visibility, created_at, updated_at)
         VALUES ($1, $2, 'identity-policy-foreign-target', 'private', $3, $3)",
    )
    .bind(BucketId::new().as_uuid())
    .bind(application_id.as_uuid())
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert foreign target bucket");
}

async fn account_id(repository: &PostgresRepository, application_id: ApplicationId) -> i64 {
    sqlx::query_scalar("SELECT s3_account_id FROM applications WHERE id = $1")
        .bind(application_id.as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("load signing application account ID")
}

async fn legacy_permissions(repository: &PostgresRepository, access_key_id: &str) -> Vec<String> {
    sqlx::query_scalar::<_, sqlx::types::Json<Vec<String>>>(
        "SELECT permissions FROM access_keys WHERE access_key_id = $1",
    )
    .bind(access_key_id)
    .fetch_one(repository.pool())
    .await
    .expect("load legacy permissions")
    .0
}

fn database_code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(DatabaseError::code)
        .map(|code| code.into_owned())
}
