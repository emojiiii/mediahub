use async_trait::async_trait;
use mediahub_app::{
    RepositoryError, S3AccountId, S3BucketIdentity, S3BucketPolicyDocument,
    S3BucketPolicyRepository, S3BucketPolicySnapshot,
};
use mediahub_core::{ApplicationId, BucketId, OffsetDateTime};
use serde_json::Value;
use sqlx::{Row, postgres::PgRow, types::Json};

use crate::{
    PostgresRepository,
    codec::{as_u64, database_error, postgres_time},
};

const GLOBAL_BUCKET_IDENTITY_SQL: &str = "SELECT b.id, b.application_id, b.name, a.s3_account_id
       FROM buckets b
       JOIN applications a ON a.id = b.application_id
      WHERE b.name = $1";

const GLOBAL_BUCKET_POLICY_SQL: &str = "SELECT b.id, b.application_id, b.name, a.s3_account_id,
            b.s3_bucket_policy, b.s3_bucket_policy_sha256,
            b.s3_bucket_policy_revision, b.s3_bucket_policy_updated_at
       FROM buckets b
       JOIN applications a ON a.id = b.application_id
      WHERE b.name = $1";

#[async_trait]
impl S3BucketPolicyRepository for PostgresRepository {
    async fn resolve_s3_bucket_identity(
        &self,
        bucket_name: &str,
    ) -> Result<Option<S3BucketIdentity>, RepositoryError> {
        let row = sqlx::query(GLOBAL_BUCKET_IDENTITY_SQL)
            .bind(bucket_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_bucket_identity).transpose()
    }

    async fn get_s3_bucket_policy(
        &self,
        bucket_name: &str,
    ) -> Result<Option<S3BucketPolicySnapshot>, RepositoryError> {
        let row = sqlx::query(GLOBAL_BUCKET_POLICY_SQL)
            .bind(bucket_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_bucket_policy_snapshot).transpose()
    }

    async fn put_s3_bucket_policy(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        policy: S3BucketPolicyDocument,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketPolicySnapshot>, RepositoryError> {
        // UPDATE acquires the bucket row lock before the owner account is read.
        // No application row lock is taken, preserving the global bucket-first
        // lock order used by the other S3 mutations.
        let row = sqlx::query(
            "UPDATE buckets AS b
                SET s3_bucket_policy = $3,
                    s3_bucket_policy_sha256 = $4,
                    s3_bucket_policy_revision = b.s3_bucket_policy_revision + 1,
                    s3_bucket_policy_updated_at = $5
               FROM applications AS a
              WHERE b.application_id = $1
                AND b.name = $2
                AND a.id = b.application_id
          RETURNING b.id, b.application_id, b.name, a.s3_account_id,
                    b.s3_bucket_policy, b.s3_bucket_policy_sha256,
                    b.s3_bucket_policy_revision, b.s3_bucket_policy_updated_at",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_name)
        .bind(Json(policy.document().clone()))
        .bind(policy.sha256())
        .bind(postgres_time(updated_at))
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_bucket_policy_snapshot).transpose()
    }

    async fn delete_s3_bucket_policy(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketPolicySnapshot>, RepositoryError> {
        let row = sqlx::query(
            "UPDATE buckets AS b
                SET s3_bucket_policy = NULL,
                    s3_bucket_policy_sha256 = NULL,
                    s3_bucket_policy_revision = b.s3_bucket_policy_revision + 1,
                    s3_bucket_policy_updated_at = $3
               FROM applications AS a
              WHERE b.application_id = $1
                AND b.name = $2
                AND a.id = b.application_id
          RETURNING b.id, b.application_id, b.name, a.s3_account_id,
                    b.s3_bucket_policy, b.s3_bucket_policy_sha256,
                    b.s3_bucket_policy_revision, b.s3_bucket_policy_updated_at",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_name)
        .bind(postgres_time(updated_at))
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_bucket_policy_snapshot).transpose()
    }
}

fn row_to_bucket_identity(row: PgRow) -> Result<S3BucketIdentity, RepositoryError> {
    let account_id: i64 = row.try_get("s3_account_id").map_err(database_error)?;
    Ok(S3BucketIdentity {
        bucket_id: BucketId::from_uuid(row.try_get("id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_name: row.try_get("name").map_err(database_error)?,
        owner_account_id: S3AccountId::new(account_id.to_string())?,
    })
}

fn row_to_bucket_policy_snapshot(row: PgRow) -> Result<S3BucketPolicySnapshot, RepositoryError> {
    let account_id: i64 = row.try_get("s3_account_id").map_err(database_error)?;
    let identity = S3BucketIdentity {
        bucket_id: BucketId::from_uuid(row.try_get("id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_name: row.try_get("name").map_err(database_error)?,
        owner_account_id: S3AccountId::new(account_id.to_string())?,
    };
    let document: Option<Json<Value>> = row.try_get("s3_bucket_policy").map_err(database_error)?;
    let expected_sha256: Option<String> = row
        .try_get("s3_bucket_policy_sha256")
        .map_err(database_error)?;
    let policy = match (document, expected_sha256) {
        (Some(Json(document)), Some(expected_sha256)) => {
            let policy = S3BucketPolicyDocument::new(document)?;
            if policy.sha256() != expected_sha256 {
                return Err(RepositoryError::Invariant(
                    "persisted S3 bucket policy digest does not match its document".into(),
                ));
            }
            Some(policy)
        }
        (None, None) => None,
        _ => {
            return Err(RepositoryError::Invariant(
                "persisted S3 bucket policy document and digest disagree".into(),
            ));
        }
    };
    S3BucketPolicySnapshot::new(
        identity,
        policy,
        as_u64(
            row.try_get("s3_bucket_policy_revision")
                .map_err(database_error)?,
        )?,
        row.try_get("s3_bucket_policy_updated_at")
            .map_err(database_error)?,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn policy_writes_are_tenant_fenced_and_increment_revision_atomically() {
        let source = include_str!("s3_bucket_policy.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("repository implementation");
        assert!(implementation.contains("b.application_id = $1"));
        assert!(implementation.contains("b.name = $2"));
        assert!(
            implementation.contains("s3_bucket_policy_revision = b.s3_bucket_policy_revision + 1")
        );
        assert!(!implementation.contains("FOR UPDATE OF a"));
    }
}
