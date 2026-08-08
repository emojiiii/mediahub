use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{S3BucketPolicyDocument, S3BucketPolicyRepository};
use mediahub_core::{ApplicationId, BucketId, OffsetDateTime, UserId};
use serde_json::json;
use sqlx::error::DatabaseError;
use time::Duration;

#[tokio::test]
async fn postgres_s3_bucket_policy_contract() {
    let database_url = std::env::var("MEDIAHUB_TEST_POSTGRES_URL")
        .expect("MEDIAHUB_TEST_POSTGRES_URL is required for destructive PostgreSQL tests");
    let repository = PostgresRepository::connect(&database_url)
        .await
        .expect("connect bucket policy database");
    repository
        .migrate()
        .await
        .expect("migrate bucket policy database");
    sqlx::query("TRUNCATE TABLE users CASCADE")
        .execute(repository.pool())
        .await
        .expect("reset dedicated bucket policy database");

    let now = OffsetDateTime::now_utc();
    let user_id = UserId::new();
    insert_user(&repository, user_id, now).await;
    let owner_application_id = ApplicationId::new();
    let other_application_id = ApplicationId::new();
    insert_application(&repository, user_id, owner_application_id, "owner", now).await;
    insert_application(&repository, user_id, other_application_id, "other", now).await;

    let owner_account_id = account_id(&repository, owner_application_id).await;
    let other_account_id = account_id(&repository, other_application_id).await;
    assert_eq!(owner_account_id.to_string().len(), 12);
    assert_eq!(other_account_id.to_string().len(), 12);
    assert_ne!(owner_account_id, other_account_id);

    sqlx::query("UPDATE applications SET name = 'renamed', app_id = 'app_renamed' WHERE id = $1")
        .bind(owner_application_id.as_uuid())
        .execute(repository.pool())
        .await
        .expect("rename application");
    assert_eq!(
        account_id(&repository, owner_application_id).await,
        owner_account_id,
        "S3 account identity must not derive from mutable application fields"
    );
    let immutable_error =
        sqlx::query("UPDATE applications SET s3_account_id = s3_account_id + 1 WHERE id = $1")
            .bind(owner_application_id.as_uuid())
            .execute(repository.pool())
            .await
            .expect_err("S3 account identity must reject direct rewrites");
    let immutable_code = immutable_error
        .as_database_error()
        .and_then(DatabaseError::code);
    assert!(matches!(immutable_code.as_deref(), Some("428C9" | "23514")));
    let default_rewrite_error =
        sqlx::query("UPDATE applications SET s3_account_id = DEFAULT WHERE id = $1")
            .bind(owner_application_id.as_uuid())
            .execute(repository.pool())
            .await
            .expect_err("S3 account identity must reject sequence reassignment");
    assert_eq!(
        default_rewrite_error
            .as_database_error()
            .and_then(DatabaseError::code)
            .as_deref(),
        Some("23514")
    );
    assert_eq!(
        account_id(&repository, owner_application_id).await,
        owner_account_id
    );

    let bucket_id = BucketId::new();
    insert_bucket(
        &repository,
        owner_application_id,
        bucket_id,
        "global-policy-bucket",
        now,
    )
    .await
    .expect("insert owner bucket");
    let conflict = insert_bucket(
        &repository,
        other_application_id,
        BucketId::new(),
        "global-policy-bucket",
        now,
    )
    .await
    .expect_err("bucket names must be globally unique");
    assert_eq!(
        conflict
            .as_database_error()
            .and_then(DatabaseError::code)
            .as_deref(),
        Some("23505")
    );

    let identity = repository
        .resolve_s3_bucket_identity("global-policy-bucket")
        .await
        .expect("resolve global bucket identity")
        .expect("bucket identity");
    assert_eq!(identity.bucket_id, bucket_id);
    assert_eq!(identity.application_id, owner_application_id);
    assert_eq!(
        identity.owner_account_id.as_str(),
        owner_account_id.to_string()
    );
    assert!(
        repository
            .resolve_s3_bucket_identity("missing-bucket")
            .await
            .expect("resolve missing bucket")
            .is_none()
    );

    let initial = repository
        .get_s3_bucket_policy("global-policy-bucket")
        .await
        .expect("get initial policy state")
        .expect("initial bucket state");
    assert_eq!(initial.revision, 0);
    assert!(initial.policy.is_none());
    assert!(initial.updated_at.is_none());

    let first_policy = S3BucketPolicyDocument::new(json!({
        "Version": "2012-10-17",
        "Statement": [{
            "Effect": "Allow",
            "Principal": "*",
            "Action": "s3:GetObject",
            "Resource": "arn:aws:s3:::global-policy-bucket/*"
        }]
    }))
    .expect("first policy document");
    let first_sha256 = first_policy.sha256().to_owned();
    let first = repository
        .put_s3_bucket_policy(
            owner_application_id,
            "global-policy-bucket",
            first_policy,
            now + Duration::seconds(1),
        )
        .await
        .expect("put first policy")
        .expect("owner bucket");
    assert_eq!(first.revision, 1);
    assert_eq!(
        first.policy.as_ref().map(S3BucketPolicyDocument::sha256),
        Some(first_sha256.as_str())
    );

    let isolated_put = repository
        .put_s3_bucket_policy(
            other_application_id,
            "global-policy-bucket",
            S3BucketPolicyDocument::new(json!({
                "Version": "2012-10-17",
                "Statement": []
            }))
            .expect("isolated policy"),
            now + Duration::seconds(2),
        )
        .await
        .expect("tenant-fenced put");
    assert!(isolated_put.is_none());
    assert_eq!(
        repository
            .get_s3_bucket_policy("global-policy-bucket")
            .await
            .expect("get after isolated put")
            .expect("bucket state")
            .revision,
        1
    );

    let second = repository
        .put_s3_bucket_policy(
            owner_application_id,
            "global-policy-bucket",
            S3BucketPolicyDocument::new(json!({
                "Version": "2012-10-17",
                "Id": "replacement",
                "Statement": [{
                    "Effect": "Allow",
                    "Principal": "*",
                    "Action": "s3:ListBucket",
                    "Resource": "arn:aws:s3:::global-policy-bucket",
                    "Condition": {
                        "NumericLessThanEquals": {"s3:max-keys": 1000.0}
                    }
                }]
            }))
            .expect("replacement policy"),
            now + Duration::seconds(3),
        )
        .await
        .expect("replace policy")
        .expect("owner bucket");
    assert_eq!(second.revision, 2);

    let isolated_delete = repository
        .delete_s3_bucket_policy(
            other_application_id,
            "global-policy-bucket",
            now + Duration::seconds(4),
        )
        .await
        .expect("tenant-fenced delete");
    assert!(isolated_delete.is_none());

    let deleted = repository
        .delete_s3_bucket_policy(
            owner_application_id,
            "global-policy-bucket",
            now + Duration::seconds(5),
        )
        .await
        .expect("delete policy")
        .expect("owner bucket");
    assert_eq!(deleted.revision, 3);
    assert!(deleted.policy.is_none());
    assert!(deleted.updated_at.is_some());

    let after_delete = repository
        .get_s3_bucket_policy("global-policy-bucket")
        .await
        .expect("get deleted policy state")
        .expect("bucket remains globally resolvable");
    assert_eq!(after_delete.revision, 3);
    assert!(after_delete.policy.is_none());
    assert_eq!(
        after_delete.identity.owner_account_id.as_str(),
        owner_account_id.to_string()
    );
}

async fn insert_user(repository: &PostgresRepository, user_id: UserId, now: OffsetDateTime) {
    sqlx::query(
        "INSERT INTO users (id, email_normalized, password_hash, created_at, updated_at)
         VALUES ($1, $2, 'bucket-policy-contract-hash', $3, $3)",
    )
    .bind(user_id.as_uuid())
    .bind(format!("{user_id}@bucket-policy.invalid"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert bucket policy user");
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
    .bind(format!("bucket-policy-{suffix}"))
    .bind(format!("app_bucket_policy_{suffix}"))
    .bind(now)
    .execute(repository.pool())
    .await
    .expect("insert bucket policy application");
}

async fn insert_bucket(
    repository: &PostgresRepository,
    application_id: ApplicationId,
    bucket_id: BucketId,
    name: &str,
    now: OffsetDateTime,
) -> Result<sqlx::postgres::PgQueryResult, sqlx::Error> {
    sqlx::query(
        "INSERT INTO buckets (id, application_id, name, visibility, created_at, updated_at)
         VALUES ($1, $2, $3, 'private', $4, $4)",
    )
    .bind(bucket_id.as_uuid())
    .bind(application_id.as_uuid())
    .bind(name)
    .bind(now)
    .execute(repository.pool())
    .await
}

async fn account_id(repository: &PostgresRepository, application_id: ApplicationId) -> i64 {
    sqlx::query_scalar("SELECT s3_account_id FROM applications WHERE id = $1")
        .bind(application_id.as_uuid())
        .fetch_one(repository.pool())
        .await
        .expect("load S3 account identity")
}
