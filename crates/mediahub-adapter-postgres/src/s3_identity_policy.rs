use async_trait::async_trait;
use mediahub_app::{
    DeleteS3IdentityPolicy, PutS3IdentityPolicy, RepositoryError, S3AccessKeyIdentity, S3AccountId,
    S3IdentityPolicyDocument, S3IdentityPolicyRepository, S3IdentityPolicySnapshot,
};
use mediahub_core::ApplicationId;
use serde_json::Value;
use sqlx::{Row, postgres::PgRow, types::Json};

use crate::{
    PostgresRepository,
    codec::{as_u64, database_error, postgres_time},
};

const GET_ACCESS_KEY_POLICY_SQL: &str =
    "SELECT ak.application_id, ak.access_key_id, a.s3_account_id,
            ak.s3_identity_policy, ak.s3_identity_policy_sha256,
            ak.s3_identity_policy_revision, ak.s3_identity_policy_updated_at
       FROM access_keys AS ak
       JOIN applications AS a ON a.id = ak.application_id
      WHERE ak.access_key_id = $1";

const LOCK_ACCESS_KEY_POLICY_SQL: &str =
    "SELECT ak.application_id, ak.access_key_id, a.s3_account_id,
            ak.s3_identity_policy, ak.s3_identity_policy_sha256,
            ak.s3_identity_policy_revision, ak.s3_identity_policy_updated_at
       FROM access_keys AS ak
       JOIN applications AS a ON a.id = ak.application_id
      WHERE ak.application_id = $1 AND ak.access_key_id = $2
        FOR UPDATE OF ak";

#[async_trait]
impl S3IdentityPolicyRepository for PostgresRepository {
    async fn get_s3_identity_policy(
        &self,
        access_key_id: &str,
    ) -> Result<Option<S3IdentityPolicySnapshot>, RepositoryError> {
        let row = sqlx::query(GET_ACCESS_KEY_POLICY_SQL)
            .bind(access_key_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_identity_policy_snapshot).transpose()
    }

    async fn put_s3_identity_policy(
        &self,
        command: &PutS3IdentityPolicy,
    ) -> Result<Option<S3IdentityPolicySnapshot>, RepositoryError> {
        let row = sqlx::query(
            "UPDATE access_keys AS ak
                SET s3_identity_policy = CAST($3 AS JSONB),
                    s3_identity_policy_sha256 = $4,
                    s3_identity_policy_revision = ak.s3_identity_policy_revision + 1,
                    s3_identity_policy_updated_at = $5
               FROM applications AS a
              WHERE ak.application_id = $1
                AND ak.access_key_id = $2
                AND a.id = ak.application_id
          RETURNING ak.application_id, ak.access_key_id, a.s3_account_id,
                    ak.s3_identity_policy, ak.s3_identity_policy_sha256,
                    ak.s3_identity_policy_revision, ak.s3_identity_policy_updated_at",
        )
        .bind(command.application_id.as_uuid())
        .bind(&command.access_key_id)
        .bind(command.policy.stable_json())
        .bind(command.policy.sha256())
        .bind(postgres_time(command.updated_at))
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_identity_policy_snapshot).transpose()
    }

    async fn delete_s3_identity_policy(
        &self,
        command: &DeleteS3IdentityPolicy,
    ) -> Result<Option<S3IdentityPolicySnapshot>, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let current = sqlx::query(LOCK_ACCESS_KEY_POLICY_SQL)
            .bind(command.application_id.as_uuid())
            .bind(&command.access_key_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
            .map(row_to_identity_policy_snapshot)
            .transpose()?;
        let Some(current) = current else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(None);
        };
        if current.policy.is_none() {
            transaction.commit().await.map_err(database_error)?;
            return Ok(Some(current));
        }

        let row = sqlx::query(
            "UPDATE access_keys AS ak
                SET s3_identity_policy = NULL,
                    s3_identity_policy_sha256 = NULL,
                    s3_identity_policy_revision = ak.s3_identity_policy_revision + 1,
                    s3_identity_policy_updated_at = $3
               FROM applications AS a
              WHERE ak.application_id = $1
                AND ak.access_key_id = $2
                AND a.id = ak.application_id
          RETURNING ak.application_id, ak.access_key_id, a.s3_account_id,
                    ak.s3_identity_policy, ak.s3_identity_policy_sha256,
                    ak.s3_identity_policy_revision, ak.s3_identity_policy_updated_at",
        )
        .bind(command.application_id.as_uuid())
        .bind(&command.access_key_id)
        .bind(postgres_time(command.updated_at))
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let snapshot = row_to_identity_policy_snapshot(row)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(Some(snapshot))
    }
}

fn row_to_identity_policy_snapshot(
    row: PgRow,
) -> Result<S3IdentityPolicySnapshot, RepositoryError> {
    let identity = S3AccessKeyIdentity {
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        account_id: S3AccountId::new(
            row.try_get::<i64, _>("s3_account_id")
                .map_err(database_error)?
                .to_string(),
        )?,
        access_key_id: row.try_get("access_key_id").map_err(database_error)?,
    };
    let document: Option<Json<Value>> =
        row.try_get("s3_identity_policy").map_err(database_error)?;
    let expected_sha256: Option<String> = row
        .try_get("s3_identity_policy_sha256")
        .map_err(database_error)?;
    let policy = match (document, expected_sha256) {
        (Some(Json(document)), Some(expected_sha256)) => {
            let encoded = serde_json::to_vec(&document)
                .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
            let policy = S3IdentityPolicyDocument::parse(&encoded)
                .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
            if policy.sha256() != expected_sha256 {
                return Err(RepositoryError::Invariant(
                    "persisted S3 identity policy digest does not match its document".into(),
                ));
            }
            Some(policy)
        }
        (None, None) => None,
        _ => {
            return Err(RepositoryError::Invariant(
                "persisted S3 identity policy document and digest disagree".into(),
            ));
        }
    };
    S3IdentityPolicySnapshot::new(
        identity,
        policy,
        as_u64(
            row.try_get("s3_identity_policy_revision")
                .map_err(database_error)?,
        )?,
        row.try_get("s3_identity_policy_updated_at")
            .map_err(database_error)?,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn mutations_are_tenant_fenced_and_revisioned() {
        let source = include_str!("s3_identity_policy.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("repository implementation");
        assert!(implementation.contains("ak.application_id = $1"));
        assert!(implementation.contains("ak.access_key_id = $2"));
        assert!(implementation.contains("a.s3_account_id"));
        assert!(
            implementation
                .contains("s3_identity_policy_revision = ak.s3_identity_policy_revision + 1")
        );
        assert!(implementation.contains("FOR UPDATE"));
    }
}
