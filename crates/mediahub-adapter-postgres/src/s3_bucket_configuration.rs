use async_trait::async_trait;
use mediahub_app::{
    RepositoryError, S3BucketCorsRepository, S3BucketCorsSnapshot, S3BucketTaggingRepository,
    S3BucketTaggingSnapshot,
};
use mediahub_core::{ApplicationId, OffsetDateTime, S3BucketTagSet, S3CorsConfiguration};
use sqlx::{Row, postgres::PgRow, types::Json};

use crate::{
    PostgresRepository,
    codec::{as_u64, database_error, postgres_time},
};

const BUCKET_CORS_SQL: &str = "SELECT s3_cors_configuration, s3_cors_revision, s3_cors_updated_at
       FROM buckets
      WHERE application_id = $1 AND name = $2";

const BUCKET_TAGGING_SQL: &str = "SELECT s3_bucket_tags, s3_bucket_tags_revision,
            s3_bucket_tags_updated_at
       FROM buckets
      WHERE application_id = $1 AND name = $2";

#[async_trait]
impl S3BucketCorsRepository for PostgresRepository {
    async fn get_s3_bucket_cors(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
    ) -> Result<Option<S3BucketCorsSnapshot>, RepositoryError> {
        let row = sqlx::query(BUCKET_CORS_SQL)
            .bind(application_id.as_uuid())
            .bind(bucket_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_bucket_cors_snapshot).transpose()
    }

    async fn put_s3_bucket_cors(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        configuration: S3CorsConfiguration,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketCorsSnapshot>, RepositoryError> {
        let row = sqlx::query(
            "UPDATE buckets
                SET s3_cors_configuration = $3,
                    s3_cors_revision = s3_cors_revision + 1,
                    s3_cors_updated_at = $4
              WHERE application_id = $1 AND name = $2
          RETURNING s3_cors_configuration, s3_cors_revision, s3_cors_updated_at",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_name)
        .bind(Json(configuration))
        .bind(postgres_time(updated_at))
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_bucket_cors_snapshot).transpose()
    }

    async fn delete_s3_bucket_cors(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketCorsSnapshot>, RepositoryError> {
        let row = sqlx::query(
            "UPDATE buckets
                SET s3_cors_configuration = NULL,
                    s3_cors_revision = s3_cors_revision + 1,
                    s3_cors_updated_at = $3
              WHERE application_id = $1 AND name = $2
          RETURNING s3_cors_configuration, s3_cors_revision, s3_cors_updated_at",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_name)
        .bind(postgres_time(updated_at))
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_bucket_cors_snapshot).transpose()
    }
}

#[async_trait]
impl S3BucketTaggingRepository for PostgresRepository {
    async fn get_s3_bucket_tagging(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
    ) -> Result<Option<S3BucketTaggingSnapshot>, RepositoryError> {
        let row = sqlx::query(BUCKET_TAGGING_SQL)
            .bind(application_id.as_uuid())
            .bind(bucket_name)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_bucket_tagging_snapshot).transpose()
    }

    async fn put_s3_bucket_tagging(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        tags: S3BucketTagSet,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketTaggingSnapshot>, RepositoryError> {
        let row = sqlx::query(
            "UPDATE buckets
                SET s3_bucket_tags = $3,
                    s3_bucket_tags_revision = s3_bucket_tags_revision + 1,
                    s3_bucket_tags_updated_at = $4
              WHERE application_id = $1 AND name = $2
          RETURNING s3_bucket_tags, s3_bucket_tags_revision, s3_bucket_tags_updated_at",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_name)
        .bind(Json(tags))
        .bind(postgres_time(updated_at))
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_bucket_tagging_snapshot).transpose()
    }

    async fn delete_s3_bucket_tagging(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketTaggingSnapshot>, RepositoryError> {
        let row = sqlx::query(
            "UPDATE buckets
                SET s3_bucket_tags = NULL,
                    s3_bucket_tags_revision = s3_bucket_tags_revision + 1,
                    s3_bucket_tags_updated_at = $3
              WHERE application_id = $1 AND name = $2
          RETURNING s3_bucket_tags, s3_bucket_tags_revision, s3_bucket_tags_updated_at",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_name)
        .bind(postgres_time(updated_at))
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_bucket_tagging_snapshot).transpose()
    }
}

fn row_to_bucket_cors_snapshot(row: PgRow) -> Result<S3BucketCorsSnapshot, RepositoryError> {
    let configuration = row
        .try_get::<Option<Json<S3CorsConfiguration>>, _>("s3_cors_configuration")
        .map_err(database_error)?
        .map(|value| value.0);
    S3BucketCorsSnapshot::new(
        configuration,
        as_u64(row.try_get("s3_cors_revision").map_err(database_error)?)?,
        row.try_get("s3_cors_updated_at").map_err(database_error)?,
    )
}

fn row_to_bucket_tagging_snapshot(row: PgRow) -> Result<S3BucketTaggingSnapshot, RepositoryError> {
    let tags = row
        .try_get::<Option<Json<S3BucketTagSet>>, _>("s3_bucket_tags")
        .map_err(database_error)?
        .map(|value| value.0);
    S3BucketTaggingSnapshot::new(
        tags,
        as_u64(
            row.try_get("s3_bucket_tags_revision")
                .map_err(database_error)?,
        )?,
        row.try_get("s3_bucket_tags_updated_at")
            .map_err(database_error)?,
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn mutations_are_tenant_fenced_and_revisioned_in_one_statement() {
        let source = include_str!("s3_bucket_configuration.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("repository implementation");
        assert!(implementation.contains("WHERE application_id = $1 AND name = $2"));
        assert!(implementation.contains("s3_cors_revision = s3_cors_revision + 1"));
        assert!(implementation.contains("s3_bucket_tags_revision = s3_bucket_tags_revision + 1"));
    }
}
