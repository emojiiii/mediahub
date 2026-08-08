use async_trait::async_trait;
use mediahub_app::{
    DEFAULT_S3_MULTIPART_GC_MAX_ATTEMPTS, DeleteS3ObjectCommand, DeleteS3ObjectOutcome,
    DeletedS3ObjectVersion, MAX_STORAGE_GC_CLAIM_LIMIT, MAX_UPLOAD_INTENT_EXPIRY_LIMIT,
    PutS3ObjectLockCommand, PutS3ObjectLockOutcome, RepositoryError, S3BucketRepository,
    S3DeleteLockReason, S3ListingRepository, S3MultipartUploadListItem, S3MultipartUploadListQuery,
    S3MultipartUploadPage, S3ObjectCommitTarget, S3ObjectListItem, S3ObjectListQuery,
    S3ObjectLockMutation, S3ObjectPage, S3ObjectRepository, S3ObjectTaggingCommand,
    S3ObjectTaggingOutcome, S3ObjectVersionCommit, S3ObjectVersionListItem,
    S3ObjectVersionListQuery, S3ObjectVersionPage, S3ObjectVersionRead, S3UploadIntentRepository,
    StorageGcRepository, is_multipart_etag,
};
use mediahub_core::{
    ApplicationId, BucketId, BucketS3Configuration, Checksum, ChecksumAlgorithm, DefaultRetention,
    EntityTag, NewStorageGcTask, ObjectId, ObjectRetention, ObjectRetentionUpdateError,
    ObjectVersion, ObjectVersionId, ObjectVersionPayload, ObjectVersionState,
    PersistedBucketS3Configuration, PersistedObjectVersion, PersistedS3Bucket, PersistedS3Object,
    PersistedUploadIntent, RetentionMode, S3Bucket, S3LifecycleConfiguration, S3Object,
    S3ObjectTag, S3ObjectTagSet, S3VersionId, SourceProtocol, StorageGcReason, StorageGcTask,
    StorageGcTaskId, StorageGcTaskState, StoredObjectVersion, UploadIntent, UploadIntentId,
    UploadIntentState, VersioningStatus,
};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder, Row, postgres::PgRow, types::Json};
use time::OffsetDateTime;

use crate::{
    PostgresRepository,
    codec::{as_i64, as_u32, as_u64, database_error, postgres_time},
};

const CLAIM_UPLOAD_INTENT_SQL: &str = "WITH candidate AS (
    SELECT id FROM s3_upload_intents
    WHERE id = $1 AND expires_at > $4
      AND (state = 'ready' OR (state = 'committing' AND lease_until <= $4))
    FOR UPDATE SKIP LOCKED
)
UPDATE s3_upload_intents AS intent
SET state = 'committing', lease_token = $2, lease_until = $3, updated_at = $4
FROM candidate
WHERE intent.id = candidate.id
RETURNING intent.*";

const CLAIM_STORAGE_GC_TASKS_SQL: &str = "WITH candidates AS (
    SELECT id FROM storage_gc_tasks
    WHERE attempts < max_attempts
      AND ((state = 'pending' AND not_before <= $3)
        OR (state = 'leased' AND lease_until <= $3))
    ORDER BY not_before, id
    FOR UPDATE SKIP LOCKED
    LIMIT $4
)
UPDATE storage_gc_tasks AS task
SET state = 'leased', lease_token = $1, lease_until = $2,
    attempts = task.attempts + 1, updated_at = $3
FROM candidates
WHERE task.id = candidates.id
RETURNING task.*";

const FIND_ACTIVE_OBJECT_VERSION_SQL: &str = "SELECT * FROM object_versions
WHERE object_id = $1 AND external_version_id = $2
  AND superseded_at IS NULL AND state = 'committed'";

const FIND_OBJECT_VERSION_BY_ID_AUDIT_SQL: &str = "SELECT * FROM object_versions WHERE id = $1";

const FIND_COMMITTED_OBJECT_VERSION_FOR_APPLICATION_SQL: &str =
    "SELECT version.*, logical_object.object_key AS preview_object_key
FROM object_versions AS version
JOIN objects AS logical_object
  ON logical_object.id = version.object_id
 AND logical_object.application_id = version.application_id
 AND logical_object.bucket_id = version.bucket_id
WHERE version.id = $1 AND version.application_id = $2
  AND version.state = 'committed' AND NOT version.is_delete_marker";

const FIND_CURRENT_OBJECT_VERSION_SQL: &str = "SELECT version.* FROM objects object
JOIN object_versions version ON version.id = object.current_version_id
WHERE object.id = $1
  AND version.superseded_at IS NULL AND version.state = 'committed'";

const LIST_ACTIVE_OBJECT_VERSIONS_SQL: &str = "SELECT * FROM object_versions
WHERE object_id = $1 AND superseded_at IS NULL AND state = 'committed'
ORDER BY generation DESC";

const LOCK_ACTIVE_NULL_VERSION_SQL: &str = "SELECT * FROM object_versions
WHERE object_id = $1 AND external_version_id = 'null'
  AND superseded_at IS NULL AND state = 'committed'
FOR UPDATE";

const SUPERSEDE_ACTIVE_NULL_VERSION_SQL: &str = "UPDATE object_versions
SET superseded_at = $1
WHERE id = $2 AND object_id = $3 AND superseded_at IS NULL
  AND became_noncurrent_at IS NOT NULL";

fn s3_object_fetch_limit(query: &S3ObjectListQuery) -> Result<Option<i64>, RepositoryError> {
    query.validate()?;
    if query.limit == 0 {
        return Ok(None);
    }
    i64::try_from(query.limit + 1)
        .map(Some)
        .map_err(|_| RepositoryError::Invariant("S3 object page limit is too large".into()))
}

fn build_list_current_s3_objects_query(
    application_id: ApplicationId,
    query: &S3ObjectListQuery,
    fetch_limit: i64,
) -> QueryBuilder<Postgres> {
    let mut sql = QueryBuilder::<Postgres>::new(
        "WITH current_objects AS (\
         SELECT logical_object.object_key, version.id AS version_id \
         FROM objects AS logical_object \
         JOIN object_versions AS version \
           ON version.id = logical_object.current_version_id \
          AND version.object_id = logical_object.id \
         WHERE logical_object.application_id = ",
    );
    sql.push_bind(application_id.as_uuid())
        .push(" AND logical_object.bucket_id = ")
        .push_bind(query.bucket_id.as_uuid())
        .push(" AND version.superseded_at IS NULL")
        .push(" AND version.state = 'committed'")
        .push(" AND NOT version.is_delete_marker")
        .push(" AND LEFT(logical_object.object_key, char_length(")
        .push_bind(query.prefix.clone())
        .push(")) = ")
        .push_bind(query.prefix.clone())
        .push(')');

    if query.delimiter {
        sql.push(
            ", filtered AS (\
             SELECT object_key, version_id, substring(object_key FROM char_length(",
        )
        .push_bind(query.prefix.clone())
        .push(
            ") + 1) AS relative_key FROM current_objects\
             ), entries AS (\
             SELECT object_key AS entry_key, FALSE AS is_prefix, version_id \
             FROM filtered WHERE strpos(relative_key, '/') = 0 \
             UNION ALL SELECT ",
        )
        .push_bind(query.prefix.clone())
        .push(
            " || split_part(relative_key, '/', 1) || '/' AS entry_key, \
             TRUE AS is_prefix, NULL::uuid AS version_id \
             FROM filtered WHERE strpos(relative_key, '/') > 0 \
             GROUP BY split_part(relative_key, '/', 1)\
             )",
        );
    } else {
        sql.push(
            ", entries AS (\
             SELECT object_key AS entry_key, FALSE AS is_prefix, version_id \
             FROM current_objects\
             )",
        );
    }

    sql.push(
        ", paged AS (\
         SELECT entry_key, is_prefix, version_id FROM entries",
    );
    if let Some(start_after) = &query.start_after {
        sql.push(" WHERE entry_key COLLATE \"C\" > ")
            .push_bind(start_after.clone())
            .push(" COLLATE \"C\"");
    }
    sql.push(" ORDER BY entry_key COLLATE \"C\" ASC LIMIT ")
        .push_bind(fetch_limit)
        .push(
            ") \
             SELECT paged.entry_key, paged.is_prefix, version.* \
             FROM paged \
             LEFT JOIN object_versions AS version ON version.id = paged.version_id \
             ORDER BY paged.entry_key COLLATE \"C\" ASC",
        );
    sql
}

const FIND_LIST_VERSION_MARKER_GENERATION_SQL: &str = "SELECT version.generation
FROM objects AS logical_object
JOIN object_versions AS version ON version.object_id = logical_object.id
WHERE logical_object.application_id = $1
  AND logical_object.bucket_id = $2
  AND logical_object.object_key = $3
  AND version.external_version_id = $4
  AND version.superseded_at IS NULL
  AND version.state = 'committed'";

fn s3_version_fetch_limit(
    query: &S3ObjectVersionListQuery,
) -> Result<Option<i64>, RepositoryError> {
    query.validate()?;
    if query.limit == 0 {
        return Ok(None);
    }
    i64::try_from(query.limit + 1)
        .map(Some)
        .map_err(|_| RepositoryError::Invariant("S3 version page limit is too large".into()))
}

fn build_list_s3_object_versions_query(
    application_id: ApplicationId,
    query: &S3ObjectVersionListQuery,
    marker_generation: Option<i64>,
    fetch_limit: i64,
) -> QueryBuilder<Postgres> {
    let mut sql = QueryBuilder::<Postgres>::new(
        "WITH visible_versions AS (\
         SELECT logical_object.object_key, version.id AS version_id, version.generation, \
                version.external_version_id, \
                logical_object.current_version_id = version.id AS is_latest \
         FROM objects AS logical_object \
         JOIN object_versions AS version ON version.object_id = logical_object.id \
         WHERE logical_object.application_id = ",
    );
    sql.push_bind(application_id.as_uuid())
        .push(" AND logical_object.bucket_id = ")
        .push_bind(query.bucket_id.as_uuid())
        .push(" AND version.superseded_at IS NULL")
        .push(" AND version.state = 'committed'")
        .push(" AND LEFT(logical_object.object_key, char_length(")
        .push_bind(query.prefix.clone())
        .push(")) = ")
        .push_bind(query.prefix.clone())
        .push(')');

    if query.delimiter {
        sql.push(
            ", filtered AS (\
             SELECT object_key, version_id, generation, external_version_id, is_latest, \
                    substring(object_key FROM char_length(",
        )
        .push_bind(query.prefix.clone())
        .push(
            ") + 1) AS relative_key FROM visible_versions\
             ), entries AS (\
             SELECT object_key AS entry_key, FALSE AS is_prefix, version_id, generation, \
                    external_version_id, is_latest \
             FROM filtered WHERE strpos(relative_key, '/') = 0 \
             UNION ALL SELECT ",
        )
        .push_bind(query.prefix.clone())
        .push(
            " || split_part(relative_key, '/', 1) || '/' AS entry_key, \
             TRUE AS is_prefix, NULL::uuid AS version_id, NULL::bigint AS generation, \
             NULL::text AS external_version_id, FALSE AS is_latest \
             FROM filtered WHERE strpos(relative_key, '/') > 0 \
             GROUP BY split_part(relative_key, '/', 1)\
             )",
        );
    } else {
        sql.push(
            ", entries AS (\
             SELECT object_key AS entry_key, FALSE AS is_prefix, version_id, generation, \
                    external_version_id, is_latest FROM visible_versions\
             )",
        );
    }

    sql.push(
        ", paged AS (\
         SELECT entry_key, is_prefix, version_id, generation, external_version_id, is_latest \
         FROM entries",
    );
    if let Some(key_marker) = &query.key_marker {
        if let Some(marker_generation) = marker_generation {
            sql.push(" WHERE (entry_key COLLATE \"C\" > ")
                .push_bind(key_marker.clone())
                .push(" COLLATE \"C\") OR (entry_key COLLATE \"C\" = ")
                .push_bind(key_marker.clone())
                .push(" COLLATE \"C\" AND NOT is_prefix AND generation < ")
                .push_bind(marker_generation)
                .push(')');
        } else {
            sql.push(" WHERE entry_key COLLATE \"C\" > ")
                .push_bind(key_marker.clone())
                .push(" COLLATE \"C\"");
        }
    }
    sql.push(
        " ORDER BY entry_key COLLATE \"C\" ASC, is_prefix ASC, generation DESC NULLS LAST \
         LIMIT ",
    )
    .push_bind(fetch_limit)
    .push(
        ") \
         SELECT paged.entry_key, paged.is_prefix, paged.is_latest, version.* \
         FROM paged \
         LEFT JOIN object_versions AS version ON version.id = paged.version_id \
         ORDER BY paged.entry_key COLLATE \"C\" ASC, paged.is_prefix ASC, \
                  paged.generation DESC NULLS LAST",
    );
    sql
}

fn s3_multipart_upload_fetch_limit(
    query: &S3MultipartUploadListQuery,
) -> Result<Option<i64>, RepositoryError> {
    query.validate()?;
    if query.limit == 0 {
        return Ok(None);
    }
    i64::try_from(query.limit + 1).map(Some).map_err(|_| {
        RepositoryError::Invariant("S3 multipart-upload page limit is too large".into())
    })
}

fn build_list_s3_multipart_uploads_query(
    application_id: ApplicationId,
    query: &S3MultipartUploadListQuery,
    fetch_limit: i64,
) -> QueryBuilder<Postgres> {
    let mut sql = QueryBuilder::<Postgres>::new(
        "WITH active_uploads AS (\
         SELECT object_key, upload_id, created_at \
         FROM s3_multipart_uploads \
         WHERE application_id = ",
    );
    sql.push_bind(application_id.as_uuid())
        .push(" AND bucket_id = ")
        .push_bind(query.bucket_id.as_uuid())
        .push(" AND state IN ('pending', 'completing')")
        .push(" AND expires_at > ")
        .push_bind(query.as_of)
        .push(" AND LEFT(object_key, char_length(")
        .push_bind(query.prefix.clone())
        .push(")) = ")
        .push_bind(query.prefix.clone())
        .push(')');

    if query.delimiter {
        sql.push(
            ", filtered AS (\
             SELECT object_key, upload_id, created_at, \
                    substring(object_key FROM char_length(",
        )
        .push_bind(query.prefix.clone())
        .push(
            ") + 1) AS relative_key FROM active_uploads\
             ), entries AS (\
             SELECT object_key AS entry_key, FALSE AS is_prefix, upload_id, created_at \
             FROM filtered WHERE strpos(relative_key, '/') = 0 \
             UNION ALL SELECT ",
        )
        .push_bind(query.prefix.clone())
        .push(
            " || split_part(relative_key, '/', 1) || '/' AS entry_key, \
             TRUE AS is_prefix, NULL::text AS upload_id, NULL::timestamptz AS created_at \
             FROM filtered WHERE strpos(relative_key, '/') > 0 \
             GROUP BY split_part(relative_key, '/', 1)\
             )",
        );
    } else {
        sql.push(
            ", entries AS (\
             SELECT object_key AS entry_key, FALSE AS is_prefix, upload_id, created_at \
             FROM active_uploads\
             )",
        );
    }

    sql.push(
        ", paged AS (\
         SELECT entry_key, is_prefix, upload_id, created_at FROM entries",
    );
    if let Some(key_marker) = &query.key_marker {
        if let Some(upload_id_marker) = &query.upload_id_marker {
            sql.push(" WHERE (entry_key COLLATE \"C\" > ")
                .push_bind(key_marker.clone())
                .push(" COLLATE \"C\") OR (entry_key COLLATE \"C\" = ")
                .push_bind(key_marker.clone())
                .push(" COLLATE \"C\" AND NOT is_prefix AND upload_id COLLATE \"C\" > ")
                .push_bind(upload_id_marker.clone())
                .push(" COLLATE \"C\")");
        } else {
            sql.push(" WHERE entry_key COLLATE \"C\" > ")
                .push_bind(key_marker.clone())
                .push(" COLLATE \"C\"");
        }
    }
    sql.push(
        " ORDER BY entry_key COLLATE \"C\" ASC, is_prefix ASC, \
                  upload_id COLLATE \"C\" ASC NULLS LAST LIMIT ",
    )
    .push_bind(fetch_limit)
    .push(
        ") SELECT entry_key, is_prefix, upload_id, created_at FROM paged \
         ORDER BY entry_key COLLATE \"C\" ASC, is_prefix ASC, \
                  upload_id COLLATE \"C\" ASC NULLS LAST",
    );
    sql
}

#[async_trait]
impl S3ListingRepository for PostgresRepository {
    async fn list_s3_object_versions_page(
        &self,
        application_id: ApplicationId,
        query: &S3ObjectVersionListQuery,
    ) -> Result<S3ObjectVersionPage, RepositoryError> {
        let Some(fetch_limit) = s3_version_fetch_limit(query)? else {
            return Ok(S3ObjectVersionPage::default());
        };
        let marker_generation = match (&query.key_marker, &query.version_id_marker) {
            (Some(key_marker), Some(version_id_marker)) => Some(
                sqlx::query_scalar::<_, i64>(FIND_LIST_VERSION_MARKER_GENERATION_SQL)
                    .bind(application_id.as_uuid())
                    .bind(query.bucket_id.as_uuid())
                    .bind(key_marker)
                    .bind(version_id_marker.as_str())
                    .fetch_optional(&self.pool)
                    .await
                    .map_err(database_error)?
                    .ok_or(RepositoryError::NotFound)?,
            ),
            _ => None,
        };
        let mut sql = build_list_s3_object_versions_query(
            application_id,
            query,
            marker_generation,
            fetch_limit,
        );
        let mut rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        let has_more = rows.len() > query.limit;
        rows.truncate(query.limit);
        let (next_key_marker, next_version_id_marker) = if has_more {
            let row = rows
                .last()
                .expect("a truncated positive-limit page is non-empty");
            let key = row
                .try_get::<String, _>("entry_key")
                .map_err(database_error)?;
            let version_id = row
                .try_get::<Option<String>, _>("external_version_id")
                .map_err(database_error)?
                .map(S3VersionId::new)
                .transpose()
                .map_err(invariant)?;
            (Some(key), version_id)
        } else {
            (None, None)
        };
        let mut page = S3ObjectVersionPage {
            next_key_marker,
            next_version_id_marker,
            ..S3ObjectVersionPage::default()
        };
        for row in rows {
            let key = row
                .try_get::<String, _>("entry_key")
                .map_err(database_error)?;
            if row
                .try_get::<bool, _>("is_prefix")
                .map_err(database_error)?
            {
                page.common_prefixes.push(key);
                continue;
            }
            let is_latest = row.try_get("is_latest").map_err(database_error)?;
            let version = row_to_object_version(row)?;
            if version.state() != ObjectVersionState::Committed {
                return Err(invariant(
                    "ListObjectVersions query returned a non-committed version",
                ));
            }
            page.items.push(S3ObjectVersionListItem {
                key,
                version,
                is_latest,
            });
        }
        Ok(page)
    }

    async fn list_s3_multipart_uploads_page(
        &self,
        application_id: ApplicationId,
        query: &S3MultipartUploadListQuery,
    ) -> Result<S3MultipartUploadPage, RepositoryError> {
        let Some(fetch_limit) = s3_multipart_upload_fetch_limit(query)? else {
            return Ok(S3MultipartUploadPage::default());
        };
        let mut sql = build_list_s3_multipart_uploads_query(application_id, query, fetch_limit);
        let mut rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        let has_more = rows.len() > query.limit;
        rows.truncate(query.limit);
        let (next_key_marker, next_upload_id_marker) = if has_more {
            let row = rows
                .last()
                .expect("a truncated positive-limit page is non-empty");
            (
                Some(
                    row.try_get::<String, _>("entry_key")
                        .map_err(database_error)?,
                ),
                row.try_get("upload_id").map_err(database_error)?,
            )
        } else {
            (None, None)
        };
        let mut page = S3MultipartUploadPage {
            next_key_marker,
            next_upload_id_marker,
            ..S3MultipartUploadPage::default()
        };
        for row in rows {
            let key = row
                .try_get::<String, _>("entry_key")
                .map_err(database_error)?;
            if row
                .try_get::<bool, _>("is_prefix")
                .map_err(database_error)?
            {
                page.common_prefixes.push(key);
                continue;
            }
            page.items.push(S3MultipartUploadListItem {
                key,
                upload_id: row.try_get("upload_id").map_err(database_error)?,
                initiated_at: row.try_get("created_at").map_err(database_error)?,
            });
        }
        Ok(page)
    }
}

#[async_trait]
impl S3BucketRepository for PostgresRepository {
    async fn list_s3_buckets(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<S3Bucket>, RepositoryError> {
        let rows = sqlx::query("SELECT * FROM buckets WHERE application_id = $1 ORDER BY name")
            .bind(application_id.as_uuid())
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        rows.into_iter().map(row_to_s3_bucket).collect()
    }

    async fn find_s3_bucket(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<S3Bucket>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM buckets WHERE application_id = $1 AND name = $2")
            .bind(application_id.as_uuid())
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_s3_bucket).transpose()
    }

    async fn create_s3_bucket(&self, bucket: &S3Bucket) -> Result<(), RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let configuration = bucket.configuration();
        let default_retention = configuration.default_retention().map(Json);
        let lifecycle_configuration = configuration.lifecycle_configuration().cloned().map(Json);
        sqlx::query(
            "INSERT INTO buckets (
                id, application_id, name, visibility, region, versioning_status,
                object_lock_enabled, default_retention, s3_lifecycle_configuration,
                s3_configuration_revision, created_at, updated_at
             ) VALUES ($1, $2, $3, 'private', $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(bucket.id().as_uuid())
        .bind(bucket.application_id().as_uuid())
        .bind(bucket.name())
        .bind(configuration.region())
        .bind(versioning_status_name(configuration.versioning_status()))
        .bind(configuration.object_lock_enabled())
        .bind(default_retention)
        .bind(lifecycle_configuration)
        .bind(as_i64(configuration.revision())?)
        .bind(postgres_time(bucket.created_at()))
        .bind(postgres_time(configuration.updated_at()))
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)
    }

    async fn delete_s3_bucket(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<bool, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query(
            "SELECT id FROM buckets WHERE application_id = $1 AND name = $2 FOR UPDATE",
        )
        .bind(application_id.as_uuid())
        .bind(name)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(false);
        };
        let bucket_id: uuid::Uuid = row.try_get("id").map_err(database_error)?;
        let has_contents = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE bucket_id = $1)
                OR EXISTS(SELECT 1 FROM media WHERE bucket_id = $1)
                OR EXISTS(SELECT 1 FROM upload_sessions WHERE bucket_id = $1)
                OR EXISTS(SELECT 1 FROM s3_multipart_uploads WHERE bucket_id = $1)",
        )
        .bind(bucket_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        if has_contents {
            return Err(RepositoryError::Conflict);
        }
        let result = sqlx::query("DELETE FROM buckets WHERE id = $1 AND application_id = $2")
            .bind(bucket_id)
            .bind(application_id.as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn get_s3_bucket_location(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<String>, RepositoryError> {
        sqlx::query_scalar("SELECT region FROM buckets WHERE application_id = $1 AND name = $2")
            .bind(application_id.as_uuid())
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)
    }

    async fn get_s3_bucket_configuration(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<BucketS3Configuration>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM buckets WHERE application_id = $1 AND name = $2")
            .bind(application_id.as_uuid())
            .bind(name)
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.as_ref().map(row_to_s3_configuration).transpose()
    }

    async fn get_s3_bucket_versioning(
        &self,
        application_id: ApplicationId,
        name: &str,
    ) -> Result<Option<VersioningStatus>, RepositoryError> {
        let status = sqlx::query_scalar::<_, String>(
            "SELECT versioning_status FROM buckets WHERE application_id = $1 AND name = $2",
        )
        .bind(application_id.as_uuid())
        .bind(name)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        status.as_deref().map(parse_versioning_status).transpose()
    }

    async fn set_s3_bucket_versioning(
        &self,
        application_id: ApplicationId,
        name: &str,
        status: VersioningStatus,
        updated_at: OffsetDateTime,
    ) -> Result<BucketS3Configuration, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row =
            sqlx::query("SELECT * FROM buckets WHERE application_id = $1 AND name = $2 FOR UPDATE")
                .bind(application_id.as_uuid())
                .bind(name)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(database_error)?
                .ok_or(RepositoryError::NotFound)?;
        let mut configuration = row_to_s3_configuration(&row)?;
        let changed = configuration
            .transition_versioning(status, updated_at)
            .map_err(invariant)?;
        if changed {
            sqlx::query(
                "UPDATE buckets
                 SET versioning_status = $1, s3_configuration_revision = $2, updated_at = $3
                 WHERE application_id = $4 AND name = $5",
            )
            .bind(versioning_status_name(configuration.versioning_status()))
            .bind(as_i64(configuration.revision())?)
            .bind(postgres_time(configuration.updated_at()))
            .bind(application_id.as_uuid())
            .bind(name)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(configuration)
    }

    async fn replace_s3_bucket_object_lock(
        &self,
        application_id: ApplicationId,
        name: &str,
        default_retention: Option<DefaultRetention>,
        updated_at: OffsetDateTime,
    ) -> Result<BucketS3Configuration, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row =
            sqlx::query("SELECT * FROM buckets WHERE application_id = $1 AND name = $2 FOR UPDATE")
                .bind(application_id.as_uuid())
                .bind(name)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(database_error)?
                .ok_or(RepositoryError::NotFound)?;
        let mut configuration = row_to_s3_configuration(&row)?;
        let changed = configuration
            .replace_object_lock_configuration(default_retention, updated_at)
            .map_err(invariant)?;
        if changed {
            let default_retention = configuration.default_retention().map(Json);
            sqlx::query(
                "UPDATE buckets
                 SET versioning_status = $1, object_lock_enabled = TRUE,
                     default_retention = $2, s3_configuration_revision = $3, updated_at = $4
                 WHERE application_id = $5 AND name = $6",
            )
            .bind(versioning_status_name(configuration.versioning_status()))
            .bind(default_retention)
            .bind(as_i64(configuration.revision())?)
            .bind(postgres_time(configuration.updated_at()))
            .bind(application_id.as_uuid())
            .bind(name)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(configuration)
    }

    async fn replace_s3_bucket_lifecycle(
        &self,
        application_id: ApplicationId,
        name: &str,
        lifecycle_configuration: Option<S3LifecycleConfiguration>,
        updated_at: OffsetDateTime,
    ) -> Result<BucketS3Configuration, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row =
            sqlx::query("SELECT * FROM buckets WHERE application_id = $1 AND name = $2 FOR UPDATE")
                .bind(application_id.as_uuid())
                .bind(name)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(database_error)?
                .ok_or(RepositoryError::NotFound)?;
        let mut configuration = row_to_s3_configuration(&row)?;
        let changed = configuration
            .replace_lifecycle_configuration(lifecycle_configuration, updated_at)
            .map_err(invariant)?;
        if changed {
            let lifecycle_configuration =
                configuration.lifecycle_configuration().cloned().map(Json);
            sqlx::query(
                "UPDATE buckets
                 SET s3_lifecycle_configuration = $1, s3_configuration_revision = $2,
                     updated_at = $3
                 WHERE application_id = $4 AND name = $5",
            )
            .bind(lifecycle_configuration)
            .bind(as_i64(configuration.revision())?)
            .bind(postgres_time(configuration.updated_at()))
            .bind(application_id.as_uuid())
            .bind(name)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(configuration)
    }
}

#[async_trait]
impl S3ObjectRepository for PostgresRepository {
    async fn find_s3_object(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        key: &str,
    ) -> Result<Option<S3Object>, RepositoryError> {
        let row = sqlx::query(
            "SELECT * FROM objects
             WHERE application_id = $1 AND bucket_id = $2 AND object_key = $3",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_id.as_uuid())
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_s3_object).transpose()
    }

    async fn list_current_s3_objects(
        &self,
        application_id: ApplicationId,
        query: &S3ObjectListQuery,
    ) -> Result<S3ObjectPage, RepositoryError> {
        let Some(fetch_limit) = s3_object_fetch_limit(query)? else {
            return Ok(S3ObjectPage::default());
        };
        let mut sql = build_list_current_s3_objects_query(application_id, query, fetch_limit);
        let mut rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        let has_more = rows.len() > query.limit;
        rows.truncate(query.limit);
        let next_cursor = if has_more {
            rows.last()
                .map(|row| row.try_get::<String, _>("entry_key"))
                .transpose()
                .map_err(database_error)?
        } else {
            None
        };
        let mut page = S3ObjectPage {
            next_cursor,
            ..S3ObjectPage::default()
        };
        for row in rows {
            let entry_key = row
                .try_get::<String, _>("entry_key")
                .map_err(database_error)?;
            if row
                .try_get::<bool, _>("is_prefix")
                .map_err(database_error)?
            {
                page.common_prefixes.push(entry_key);
                continue;
            }
            let version = row_to_object_version(row)?;
            if version.state() != ObjectVersionState::Committed || version.is_delete_marker() {
                return Err(invariant(
                    "ListObjects query returned a non-committed data version",
                ));
            }
            page.items.push(S3ObjectListItem {
                key: entry_key,
                version,
            });
        }
        Ok(page)
    }

    async fn find_s3_object_version(
        &self,
        object_id: ObjectId,
        version_id: &S3VersionId,
    ) -> Result<Option<ObjectVersion>, RepositoryError> {
        let row = sqlx::query(FIND_ACTIVE_OBJECT_VERSION_SQL)
            .bind(object_id.as_uuid())
            .bind(version_id.as_str())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_object_version).transpose()
    }

    async fn find_s3_object_version_by_id(
        &self,
        version_id: ObjectVersionId,
    ) -> Result<Option<ObjectVersion>, RepositoryError> {
        let row = sqlx::query(FIND_OBJECT_VERSION_BY_ID_AUDIT_SQL)
            .bind(version_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_object_version).transpose()
    }

    async fn find_committed_s3_object_version_for_application(
        &self,
        application_id: ApplicationId,
        version_id: ObjectVersionId,
    ) -> Result<Option<S3ObjectVersionRead>, RepositoryError> {
        let row = sqlx::query(FIND_COMMITTED_OBJECT_VERSION_FOR_APPLICATION_SQL)
            .bind(version_id.as_uuid())
            .bind(application_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(|row| {
            let object_key = row.try_get("preview_object_key").map_err(database_error)?;
            let version = row_to_object_version(row)?;
            Ok(S3ObjectVersionRead {
                object_key,
                version,
            })
        })
        .transpose()
    }

    async fn find_current_s3_object_version(
        &self,
        object_id: ObjectId,
    ) -> Result<Option<ObjectVersion>, RepositoryError> {
        let row = sqlx::query(FIND_CURRENT_OBJECT_VERSION_SQL)
            .bind(object_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_object_version).transpose()
    }

    async fn list_s3_object_versions(
        &self,
        object_id: ObjectId,
    ) -> Result<Vec<ObjectVersion>, RepositoryError> {
        let rows = sqlx::query(LIST_ACTIVE_OBJECT_VERSIONS_SQL)
            .bind(object_id.as_uuid())
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        rows.into_iter().map(row_to_object_version).collect()
    }

    async fn find_s3_object_version_tags(
        &self,
        application_id: ApplicationId,
        version_id: ObjectVersionId,
    ) -> Result<Option<S3ObjectTagSet>, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let exists = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(
                SELECT 1 FROM object_versions
                WHERE id = $1 AND application_id = $2
                  AND state = 'committed' AND NOT is_delete_marker
                  AND superseded_at IS NULL
             )",
        )
        .bind(version_id.as_uuid())
        .bind(application_id.as_uuid())
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let tags = if exists {
            Some(load_object_version_tags(&mut transaction, version_id).await?)
        } else {
            None
        };
        transaction.commit().await.map_err(database_error)?;
        Ok(tags)
    }

    async fn get_s3_object_tagging(
        &self,
        command: &S3ObjectTaggingCommand,
    ) -> Result<S3ObjectTaggingOutcome, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let outcome = resolve_object_tagging(&mut transaction, command, false).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(outcome)
    }

    async fn replace_s3_object_tagging(
        &self,
        command: &S3ObjectTaggingCommand,
        tags: &S3ObjectTagSet,
    ) -> Result<S3ObjectTaggingOutcome, RepositoryError> {
        tags.validate()
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let outcome = resolve_object_tagging(&mut transaction, command, true).await?;
        let outcome = match outcome {
            S3ObjectTaggingOutcome::Found {
                object, version, ..
            } => {
                sqlx::query("DELETE FROM object_version_tags WHERE object_version_id = $1")
                    .bind(version.id().as_uuid())
                    .execute(&mut *transaction)
                    .await
                    .map_err(database_error)?;
                insert_object_version_tags(&mut transaction, version.id(), tags).await?;
                S3ObjectTaggingOutcome::Found {
                    object,
                    version,
                    tags: tags.clone(),
                }
            }
            outcome => outcome,
        };
        transaction.commit().await.map_err(database_error)?;
        Ok(outcome)
    }

    async fn delete_s3_object(
        &self,
        command: &DeleteS3ObjectCommand,
    ) -> Result<DeleteS3ObjectOutcome, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let outcome = delete_s3_object_in_transaction(&mut transaction, command).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(outcome)
    }

    async fn put_s3_object_lock(
        &self,
        command: &PutS3ObjectLockCommand,
    ) -> Result<PutS3ObjectLockOutcome, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let object_lock_enabled = sqlx::query_scalar::<_, bool>(
            "SELECT object_lock_enabled FROM buckets
             WHERE application_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(command.application_id.as_uuid())
        .bind(command.bucket_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
        if !object_lock_enabled {
            transaction.commit().await.map_err(database_error)?;
            return Ok(PutS3ObjectLockOutcome::ObjectLockNotEnabled);
        }

        let object = sqlx::query(
            "SELECT * FROM objects
             WHERE application_id = $1 AND bucket_id = $2 AND object_key = $3
             FOR UPDATE",
        )
        .bind(command.application_id.as_uuid())
        .bind(command.bucket_id.as_uuid())
        .bind(&command.object_key)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .map(row_to_s3_object)
        .transpose()?;
        let Some(object) = object else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(if command.version_id.is_some() {
                PutS3ObjectLockOutcome::VersionNotFound
            } else {
                PutS3ObjectLockOutcome::ObjectNotFound
            });
        };

        let version_row = if let Some(version_id) = &command.version_id {
            sqlx::query(
                "SELECT * FROM object_versions
                 WHERE application_id = $1 AND bucket_id = $2 AND object_id = $3
                   AND external_version_id = $4 AND state = 'committed'
                   AND superseded_at IS NULL
                 FOR UPDATE",
            )
            .bind(command.application_id.as_uuid())
            .bind(command.bucket_id.as_uuid())
            .bind(object.id().as_uuid())
            .bind(version_id.as_str())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
        } else if let Some(version_id) = object.current_version_id() {
            sqlx::query(
                "SELECT * FROM object_versions
                 WHERE application_id = $1 AND bucket_id = $2 AND object_id = $3
                   AND id = $4 AND state = 'committed' AND superseded_at IS NULL
                 FOR UPDATE",
            )
            .bind(command.application_id.as_uuid())
            .bind(command.bucket_id.as_uuid())
            .bind(object.id().as_uuid())
            .bind(version_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
        } else {
            None
        };
        let Some(version_row) = version_row else {
            transaction.commit().await.map_err(database_error)?;
            return Ok(if command.version_id.is_some() {
                PutS3ObjectLockOutcome::VersionNotFound
            } else {
                PutS3ObjectLockOutcome::ObjectNotFound
            });
        };
        let version = row_to_object_version(version_row)?;
        if version.is_delete_marker() {
            transaction.commit().await.map_err(database_error)?;
            return Ok(PutS3ObjectLockOutcome::DeleteMarker {
                version_id: version.external_version_id().clone(),
                is_current: object.current_version_id() == Some(version.id()),
            });
        }

        let updated = match command.mutation {
            S3ObjectLockMutation::Retention {
                retention,
                bypass_governance,
            } => match version.with_retention_update(
                retention,
                command.updated_at,
                bypass_governance,
            ) {
                Ok(updated) => updated,
                Err(ObjectRetentionUpdateError::RetainUntilMustBeFuture) => {
                    return Ok(PutS3ObjectLockOutcome::InvalidRetention);
                }
                Err(ObjectRetentionUpdateError::ComplianceRetentionLocked) => {
                    return Ok(PutS3ObjectLockOutcome::RetentionLocked(
                        S3DeleteLockReason::ComplianceRetention,
                    ));
                }
                Err(ObjectRetentionUpdateError::GovernanceRetentionLocked) => {
                    return Ok(PutS3ObjectLockOutcome::RetentionLocked(
                        S3DeleteLockReason::GovernanceRetention,
                    ));
                }
                Err(ObjectRetentionUpdateError::InvalidVersion) => {
                    return Ok(PutS3ObjectLockOutcome::VersionNotFound);
                }
            },
            S3ObjectLockMutation::LegalHold(legal_hold) => version
                .with_legal_hold_update(legal_hold)
                .map_err(invariant)?,
        };
        let retention_mode = updated
            .retention()
            .map(|retention| retention_mode_name(retention.mode()));
        let retain_until = updated
            .retention()
            .map(|retention| postgres_time(retention.retain_until()));
        let result = sqlx::query(
            "UPDATE object_versions
             SET retention_mode = $1, retain_until = $2, legal_hold = $3
             WHERE id = $4 AND object_id = $5 AND application_id = $6 AND bucket_id = $7
               AND state = 'committed' AND superseded_at IS NULL",
        )
        .bind(retention_mode)
        .bind(retain_until)
        .bind(updated.legal_hold())
        .bind(updated.id().as_uuid())
        .bind(updated.object_id().as_uuid())
        .bind(command.application_id.as_uuid())
        .bind(command.bucket_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(PutS3ObjectLockOutcome::Updated(updated))
    }
    async fn create_s3_object_with_version(
        &self,
        object: S3Object,
        version: ObjectVersion,
        updated_at: OffsetDateTime,
    ) -> Result<S3Object, RepositoryError> {
        if object.generation() != 0 || object.current_version_id().is_some() {
            return Err(RepositoryError::Invariant(
                "new logical object must not already have a current version".into(),
            ));
        }
        let advanced = object
            .advanced_to(&version, updated_at)
            .map_err(invariant)?;
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        sqlx::query(
            "INSERT INTO objects (
                id, application_id, bucket_id, object_key, current_version_id,
                generation, created_at, updated_at
             ) VALUES ($1, $2, $3, $4, NULL, 0, $5, $6)",
        )
        .bind(object.id().as_uuid())
        .bind(object.application_id().as_uuid())
        .bind(object.bucket_id().as_uuid())
        .bind(object.key())
        .bind(postgres_time(object.created_at()))
        .bind(postgres_time(object.updated_at()))
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        insert_object_version(&mut transaction, &version).await?;
        sqlx::query(
            "UPDATE objects SET current_version_id = $1, generation = $2, updated_at = $3
             WHERE id = $4 AND generation = 0",
        )
        .bind(version.id().as_uuid())
        .bind(as_i64(advanced.generation())?)
        .bind(postgres_time(advanced.updated_at()))
        .bind(object.id().as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(advanced)
    }

    async fn append_s3_object_version(
        &self,
        object_id: ObjectId,
        expected_generation: u64,
        version: ObjectVersion,
        updated_at: OffsetDateTime,
    ) -> Result<S3Object, RepositoryError> {
        if version.is_null_version() {
            return Err(RepositoryError::Invariant(
                "null versions must use the fenced S3ObjectVersionCommit transaction".into(),
            ));
        }
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query("SELECT * FROM objects WHERE id = $1 FOR UPDATE")
            .bind(object_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
            .ok_or(RepositoryError::NotFound)?;
        let object = row_to_s3_object(row)?;
        if object.generation() != expected_generation {
            return Err(RepositoryError::Conflict);
        }
        let advanced = object
            .advanced_to(&version, updated_at)
            .map_err(invariant)?;

        if let Some(current_version_id) = object.current_version_id() {
            sqlx::query(
                "UPDATE object_versions SET became_noncurrent_at = $1
                 WHERE id = $2 AND object_id = $3 AND became_noncurrent_at IS NULL",
            )
            .bind(postgres_time(updated_at))
            .bind(current_version_id.as_uuid())
            .bind(object.id().as_uuid())
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        }

        insert_object_version(&mut transaction, &version).await?;
        let result = sqlx::query(
            "UPDATE objects SET current_version_id = $1, generation = $2, updated_at = $3
             WHERE id = $4 AND generation = $5",
        )
        .bind(version.id().as_uuid())
        .bind(as_i64(advanced.generation())?)
        .bind(postgres_time(advanced.updated_at()))
        .bind(object.id().as_uuid())
        .bind(as_i64(expected_generation)?)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(advanced)
    }
}

fn delete_lock_reason(
    version: &ObjectVersion,
    command: &DeleteS3ObjectCommand,
) -> Option<S3DeleteLockReason> {
    if version.legal_hold() {
        return Some(S3DeleteLockReason::LegalHold);
    }
    let retention = version.retention()?;
    if retention.retain_until() <= command.deleted_at {
        return None;
    }
    match retention.mode() {
        RetentionMode::Compliance => Some(S3DeleteLockReason::ComplianceRetention),
        RetentionMode::Governance if !command.bypass_governance => {
            Some(S3DeleteLockReason::GovernanceRetention)
        }
        RetentionMode::Governance => None,
    }
}

fn explicit_delete_gc_task(
    version: &ObjectVersion,
    command: &DeleteS3ObjectCommand,
) -> Option<NewStorageGcTask> {
    let ObjectVersionPayload::Object(payload) = version.payload() else {
        return None;
    };
    Some(NewStorageGcTask {
        id: command.gc_task_id,
        application_id: version.application_id(),
        bucket_id: version.bucket_id(),
        object_version_id: Some(version.id()),
        upload_intent_id: None,
        multipart_upload_id: None,
        storage_backend: payload.storage_backend().to_owned(),
        storage_key: payload.storage_key().to_owned(),
        reason: StorageGcReason::ExplicitDelete,
        not_before: command.gc_not_before,
        max_attempts: command.gc_max_attempts,
        created_at: command.deleted_at,
    })
}

async fn mark_current_version_noncurrent(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    object: &S3Object,
    changed_at: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let Some(current_version_id) = object.current_version_id() else {
        return Ok(());
    };
    let result = sqlx::query(
        "UPDATE object_versions
         SET became_noncurrent_at = $1
         WHERE id = $2 AND object_id = $3
           AND became_noncurrent_at IS NULL",
    )
    .bind(postgres_time(changed_at))
    .bind(current_version_id.as_uuid())
    .bind(object.id().as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn update_deleted_object_head(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    object: &S3Object,
    current_version_id: Option<ObjectVersionId>,
    updated_at: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let result = sqlx::query(
        "UPDATE objects
         SET current_version_id = $1, updated_at = $2
         WHERE id = $3 AND generation = $4",
    )
    .bind(current_version_id.map(ObjectVersionId::as_uuid))
    .bind(postgres_time(updated_at))
    .bind(object.id().as_uuid())
    .bind(as_i64(object.generation())?)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn advance_object_to_delete_marker(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    object: &S3Object,
    marker: &ObjectVersion,
    updated_at: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let next_generation = object
        .generation()
        .checked_add(1)
        .ok_or_else(|| RepositoryError::Invariant("object generation overflow".into()))?;
    if marker.generation() != next_generation {
        return Err(RepositoryError::Invariant(
            "delete marker generation is inconsistent".into(),
        ));
    }
    let result = sqlx::query(
        "UPDATE objects
         SET current_version_id = $1, generation = $2, updated_at = $3
         WHERE id = $4 AND generation = $5",
    )
    .bind(marker.id().as_uuid())
    .bind(as_i64(next_generation)?)
    .bind(postgres_time(updated_at))
    .bind(object.id().as_uuid())
    .bind(as_i64(object.generation())?)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn hide_exact_deleted_version(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    version: &ObjectVersion,
    deleted_at: OffsetDateTime,
) -> Result<(), RepositoryError> {
    if version.is_null_version() {
        return supersede_active_null_version(transaction, Some(version), deleted_at).await;
    }
    let result = sqlx::query(
        "UPDATE object_versions
         SET state = 'deleting'
         WHERE id = $1 AND object_id = $2
           AND state = 'committed' AND superseded_at IS NULL",
    )
    .bind(version.id().as_uuid())
    .bind(version.object_id().as_uuid())
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn enqueue_explicit_delete_gc_task(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &NewStorageGcTask,
) -> Result<(), RepositoryError> {
    insert_storage_gc_task(transaction, task).await?;
    let row = sqlx::query(
        "SELECT * FROM storage_gc_tasks WHERE storage_backend = $1 AND storage_key = $2",
    )
    .bind(&task.storage_backend)
    .bind(&task.storage_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or(RepositoryError::Conflict)?;
    let persisted = row_to_storage_gc_task(row)?.task;
    if persisted.application_id == task.application_id
        && persisted.bucket_id == task.bucket_id
        && persisted.object_version_id == task.object_version_id
        && persisted.upload_intent_id.is_none()
        && persisted.multipart_upload_id.is_none()
        && persisted.storage_backend == task.storage_backend
        && persisted.storage_key == task.storage_key
        && persisted.reason == StorageGcReason::ExplicitDelete
    {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn resolve_object_tagging(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    command: &S3ObjectTaggingCommand,
    exclusive: bool,
) -> Result<S3ObjectTaggingOutcome, RepositoryError> {
    let bucket_sql = if exclusive {
        "SELECT id FROM buckets WHERE application_id = $1 AND id = $2 FOR UPDATE"
    } else {
        "SELECT id FROM buckets WHERE application_id = $1 AND id = $2 FOR SHARE"
    };
    let bucket_exists = sqlx::query_scalar::<_, uuid::Uuid>(bucket_sql)
        .bind(command.application_id.as_uuid())
        .bind(command.bucket_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?
        .is_some();
    if !bucket_exists {
        return Err(RepositoryError::NotFound);
    }

    let object_sql = if exclusive {
        "SELECT * FROM objects
         WHERE application_id = $1 AND bucket_id = $2 AND object_key = $3 FOR UPDATE"
    } else {
        "SELECT * FROM objects
         WHERE application_id = $1 AND bucket_id = $2 AND object_key = $3 FOR SHARE"
    };
    let object = sqlx::query(object_sql)
        .bind(command.application_id.as_uuid())
        .bind(command.bucket_id.as_uuid())
        .bind(&command.object_key)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?
        .map(row_to_s3_object)
        .transpose()?;
    let Some(object) = object else {
        return Ok(if command.version_id.is_some() {
            S3ObjectTaggingOutcome::VersionNotFound
        } else {
            S3ObjectTaggingOutcome::ObjectNotFound
        });
    };

    let version_row = if let Some(version_id) = &command.version_id {
        let sql = if exclusive {
            "SELECT * FROM object_versions
             WHERE application_id = $1 AND bucket_id = $2 AND object_id = $3
               AND external_version_id = $4 AND state = 'committed'
               AND superseded_at IS NULL FOR UPDATE"
        } else {
            "SELECT * FROM object_versions
             WHERE application_id = $1 AND bucket_id = $2 AND object_id = $3
               AND external_version_id = $4 AND state = 'committed'
               AND superseded_at IS NULL FOR SHARE"
        };
        sqlx::query(sql)
            .bind(command.application_id.as_uuid())
            .bind(command.bucket_id.as_uuid())
            .bind(object.id().as_uuid())
            .bind(version_id.as_str())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(database_error)?
    } else if let Some(version_id) = object.current_version_id() {
        let sql = if exclusive {
            "SELECT * FROM object_versions
             WHERE application_id = $1 AND bucket_id = $2 AND object_id = $3
               AND id = $4 AND state = 'committed' AND superseded_at IS NULL FOR UPDATE"
        } else {
            "SELECT * FROM object_versions
             WHERE application_id = $1 AND bucket_id = $2 AND object_id = $3
               AND id = $4 AND state = 'committed' AND superseded_at IS NULL FOR SHARE"
        };
        sqlx::query(sql)
            .bind(command.application_id.as_uuid())
            .bind(command.bucket_id.as_uuid())
            .bind(object.id().as_uuid())
            .bind(version_id.as_uuid())
            .fetch_optional(&mut **transaction)
            .await
            .map_err(database_error)?
    } else {
        None
    };
    let Some(version_row) = version_row else {
        return Ok(if command.version_id.is_some() {
            S3ObjectTaggingOutcome::VersionNotFound
        } else {
            S3ObjectTaggingOutcome::ObjectNotFound
        });
    };
    let version = row_to_object_version(version_row)?;
    if version.is_delete_marker() {
        return Ok(S3ObjectTaggingOutcome::DeleteMarker {
            version_id: version.external_version_id().clone(),
            is_current: object.current_version_id() == Some(version.id()),
        });
    }
    let tags = load_object_version_tags(transaction, version.id()).await?;
    Ok(S3ObjectTaggingOutcome::Found {
        object,
        version: Box::new(version),
        tags,
    })
}

async fn delete_s3_object_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    command: &DeleteS3ObjectCommand,
) -> Result<DeleteS3ObjectOutcome, RepositoryError> {
    if command.deleted_by.is_empty() || command.gc_max_attempts == 0 {
        return Err(RepositoryError::Invariant(
            "S3 delete command contains invalid audit or GC facts".into(),
        ));
    }

    let bucket_row = sqlx::query(
        "SELECT versioning_status FROM buckets
         WHERE id = $1 AND application_id = $2
         FOR SHARE",
    )
    .bind(command.bucket_id.as_uuid())
    .bind(command.application_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or(RepositoryError::NotFound)?;
    let versioning_status = parse_versioning_status(
        &bucket_row
            .try_get::<String, _>("versioning_status")
            .map_err(database_error)?,
    )?;

    let object_row = sqlx::query(
        "SELECT * FROM objects
         WHERE application_id = $1 AND bucket_id = $2 AND object_key = $3
         FOR UPDATE",
    )
    .bind(command.application_id.as_uuid())
    .bind(command.bucket_id.as_uuid())
    .bind(&command.object_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?;
    let Some(object_row) = object_row else {
        return Ok(if command.version_id.is_some() {
            DeleteS3ObjectOutcome::VersionNotFound
        } else {
            DeleteS3ObjectOutcome::NoOp
        });
    };
    let object = row_to_s3_object(object_row)?;
    let active_versions = sqlx::query(
        "SELECT * FROM object_versions
         WHERE object_id = $1
           AND superseded_at IS NULL AND state = 'committed'
         ORDER BY generation DESC
         FOR UPDATE",
    )
    .bind(object.id().as_uuid())
    .fetch_all(&mut **transaction)
    .await
    .map_err(database_error)?
    .into_iter()
    .map(row_to_object_version)
    .collect::<Result<Vec<_>, _>>()?;

    if let Some(requested_version_id) = &command.version_id {
        let Some(target) = active_versions
            .iter()
            .find(|version| version.external_version_id() == requested_version_id)
        else {
            return Ok(DeleteS3ObjectOutcome::VersionNotFound);
        };
        if let Some(reason) = delete_lock_reason(target, command) {
            return Ok(DeleteS3ObjectOutcome::Locked(reason));
        }
        let is_current = object.current_version_id() == Some(target.id());
        if is_current {
            mark_current_version_noncurrent(transaction, &object, command.deleted_at).await?;
        }
        hide_exact_deleted_version(transaction, target, command.deleted_at).await?;
        if let Some(task) = explicit_delete_gc_task(target, command) {
            enqueue_explicit_delete_gc_task(transaction, &task).await?;
        }
        if is_current {
            let next_current = active_versions
                .iter()
                .filter(|version| version.id() != target.id())
                .max_by_key(|version| version.generation());
            if let Some(next) = next_current {
                let result = sqlx::query(
                    "UPDATE object_versions
                     SET became_noncurrent_at = NULL
                     WHERE id = $1 AND object_id = $2
                       AND state = 'committed' AND superseded_at IS NULL",
                )
                .bind(next.id().as_uuid())
                .bind(object.id().as_uuid())
                .execute(&mut **transaction)
                .await
                .map_err(database_error)?;
                if result.rows_affected() != 1 {
                    return Err(RepositoryError::Conflict);
                }
            }
            update_deleted_object_head(
                transaction,
                &object,
                next_current.map(ObjectVersion::id),
                command.deleted_at,
            )
            .await?;
        }
        return Ok(DeleteS3ObjectOutcome::Deleted(DeletedS3ObjectVersion {
            version_id: Some(target.external_version_id().clone()),
            delete_marker: target.is_delete_marker(),
        }));
    }

    match versioning_status {
        VersioningStatus::Enabled => {
            let generation = object
                .generation()
                .checked_add(1)
                .ok_or_else(|| RepositoryError::Invariant("object generation overflow".into()))?;
            let marker = ObjectVersion::new_delete_marker(
                command.delete_marker_id,
                object.id(),
                command.application_id,
                command.bucket_id,
                command.delete_marker_version_id.clone(),
                generation,
                false,
                &command.deleted_by,
                SourceProtocol::S3,
                command.deleted_at,
            )
            .map_err(invariant)?;
            mark_current_version_noncurrent(transaction, &object, command.deleted_at).await?;
            insert_object_version(transaction, &marker).await?;
            advance_object_to_delete_marker(transaction, &object, &marker, command.deleted_at)
                .await?;
            Ok(DeleteS3ObjectOutcome::Deleted(DeletedS3ObjectVersion {
                version_id: Some(command.delete_marker_version_id.clone()),
                delete_marker: true,
            }))
        }
        VersioningStatus::Suspended => {
            let active_null = active_versions
                .iter()
                .find(|version| version.is_null_version());
            if let Some(reason) =
                active_null.and_then(|version| delete_lock_reason(version, command))
            {
                return Ok(DeleteS3ObjectOutcome::Locked(reason));
            }
            let generation = object
                .generation()
                .checked_add(1)
                .ok_or_else(|| RepositoryError::Invariant("object generation overflow".into()))?;
            let null_version_id = S3VersionId::new("null").map_err(invariant)?;
            let marker = ObjectVersion::new_delete_marker(
                command.delete_marker_id,
                object.id(),
                command.application_id,
                command.bucket_id,
                null_version_id.clone(),
                generation,
                true,
                &command.deleted_by,
                SourceProtocol::S3,
                command.deleted_at,
            )
            .map_err(invariant)?;
            mark_current_version_noncurrent(transaction, &object, command.deleted_at).await?;
            if let Some(version) = active_null {
                if let Some(task) = explicit_delete_gc_task(version, command) {
                    enqueue_explicit_delete_gc_task(transaction, &task).await?;
                }
                supersede_active_null_version(transaction, Some(version), command.deleted_at)
                    .await?;
            }
            insert_object_version(transaction, &marker).await?;
            advance_object_to_delete_marker(transaction, &object, &marker, command.deleted_at)
                .await?;
            Ok(DeleteS3ObjectOutcome::Deleted(DeletedS3ObjectVersion {
                version_id: Some(null_version_id),
                delete_marker: true,
            }))
        }
        VersioningStatus::Unversioned => {
            let Some(active_null) = active_versions
                .iter()
                .find(|version| version.is_null_version())
            else {
                return Ok(DeleteS3ObjectOutcome::NoOp);
            };
            if let Some(reason) = delete_lock_reason(active_null, command) {
                return Ok(DeleteS3ObjectOutcome::Locked(reason));
            }
            mark_current_version_noncurrent(transaction, &object, command.deleted_at).await?;
            if let Some(task) = explicit_delete_gc_task(active_null, command) {
                enqueue_explicit_delete_gc_task(transaction, &task).await?;
            }
            supersede_active_null_version(transaction, Some(active_null), command.deleted_at)
                .await?;
            update_deleted_object_head(transaction, &object, None, command.deleted_at).await?;
            Ok(DeleteS3ObjectOutcome::Deleted(DeletedS3ObjectVersion {
                version_id: None,
                delete_marker: false,
            }))
        }
    }
}

#[async_trait]
impl S3UploadIntentRepository for PostgresRepository {
    async fn create_upload_intent(&self, intent: &UploadIntent) -> Result<(), RepositoryError> {
        if intent.state() != UploadIntentState::Staging {
            return Err(RepositoryError::Invariant(
                "a new upload intent must be in staging state".into(),
            ));
        }
        sqlx::query(
            "INSERT INTO s3_upload_intents (
                id, application_id, bucket_id, object_key, proposed_version_id, state,
                storage_backend, temporary_storage_key, final_storage_key,
                entity_tag, checksum_algorithm, checksum_value,
                expected_size_bytes, size_bytes, content_type, user_metadata, object_tags,
                lease_token, lease_until, committed_object_id, committed_version_id,
                expires_at, created_at, updated_at
             ) VALUES (
                $1, $2, $3, $4, $5, 'staging', $6, $7, $8,
                NULL, NULL, NULL, $9, NULL, $10, $11, $12,
                NULL, NULL, NULL, NULL, $13, $14, $15
             )",
        )
        .bind(intent.id().as_uuid())
        .bind(intent.application_id().as_uuid())
        .bind(intent.bucket_id().as_uuid())
        .bind(intent.object_key())
        .bind(intent.proposed_version_id().as_uuid())
        .bind(intent.storage_backend())
        .bind(intent.temporary_storage_key())
        .bind(intent.final_storage_key())
        .bind(as_i64(intent.expected_size_bytes())?)
        .bind(intent.content_type())
        .bind(Json(intent.user_metadata().clone()))
        .bind(Json(intent.object_tags().clone()))
        .bind(postgres_time(intent.expires_at()))
        .bind(postgres_time(intent.created_at()))
        .bind(postgres_time(intent.updated_at()))
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(())
    }

    async fn find_upload_intent(
        &self,
        intent_id: UploadIntentId,
    ) -> Result<Option<UploadIntent>, RepositoryError> {
        let row = sqlx::query("SELECT * FROM s3_upload_intents WHERE id = $1")
            .bind(intent_id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        row.map(row_to_upload_intent).transpose()
    }

    async fn complete_upload_intent_staging(
        &self,
        intent_id: UploadIntentId,
        entity_tag: &EntityTag,
        checksum: &Checksum,
        size_bytes: u64,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError> {
        let now = postgres_time(now);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let intent = lock_upload_intent(&mut transaction, intent_id).await?;
        if intent.state() != UploadIntentState::Staging || intent.expires_at() <= now {
            return Err(RepositoryError::Conflict);
        }
        if intent.expected_size_bytes() != size_bytes {
            return Err(RepositoryError::Invariant(
                "completed upload size does not match expected_size_bytes".into(),
            ));
        }

        let row = sqlx::query(
            "UPDATE s3_upload_intents
             SET state = 'ready', entity_tag = $1, checksum_algorithm = $2,
                 checksum_value = $3, size_bytes = $4, updated_at = $5
             WHERE id = $6 AND state = 'staging'
             RETURNING *",
        )
        .bind(entity_tag.as_str())
        .bind(checksum_algorithm_name(checksum.algorithm()))
        .bind(checksum.value())
        .bind(as_i64(size_bytes)?)
        .bind(now)
        .bind(intent_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::Conflict)?;
        let ready = row_to_upload_intent(row)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(ready)
    }

    async fn claim_upload_intent(
        &self,
        intent_id: UploadIntentId,
        lease_token: &str,
        lease_until: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError> {
        validate_lease(lease_token, lease_until, now)?;
        let row = sqlx::query(CLAIM_UPLOAD_INTENT_SQL)
            .bind(intent_id.as_uuid())
            .bind(lease_token)
            .bind(postgres_time(lease_until))
            .bind(postgres_time(now))
            .fetch_optional(&self.pool)
            .await
            .map_err(database_error)?;
        match row {
            Some(row) => row_to_upload_intent(row),
            None => upload_intent_write_miss(&self.pool, intent_id).await,
        }
    }

    async fn release_upload_intent(
        &self,
        intent_id: UploadIntentId,
        lease_token: &str,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError> {
        validate_token(lease_token)?;
        let now = postgres_time(now);
        let row = sqlx::query(
            "UPDATE s3_upload_intents
             SET state = 'ready', lease_token = NULL, lease_until = NULL, updated_at = $3
             WHERE id = $1 AND state = 'committing' AND lease_token = $2
               AND lease_until > $3 AND expires_at > $3
             RETURNING *",
        )
        .bind(intent_id.as_uuid())
        .bind(lease_token)
        .bind(now)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        match row {
            Some(row) => row_to_upload_intent(row),
            None => upload_intent_write_miss(&self.pool, intent_id).await,
        }
    }

    async fn commit_upload_intent(
        &self,
        intent_id: UploadIntentId,
        lease_token: &str,
        commit: S3ObjectVersionCommit,
    ) -> Result<S3Object, RepositoryError> {
        validate_token(lease_token)?;
        commit.validate()?;
        let mut commit = commit;
        let committed_at = postgres_time(commit.committed_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let intent = lock_upload_intent(&mut transaction, intent_id).await?;
        validate_upload_intent_commit(&intent, lease_token, &commit)?;

        if intent
            .lease_until()
            .is_none_or(|lease_until| lease_until <= committed_at)
        {
            return Err(RepositoryError::Conflict);
        }
        freeze_postgres_object_lock(&mut transaction, &mut commit).await?;

        let expected = ExpectedObjectIdentity {
            application_id: intent.application_id(),
            bucket_id: intent.bucket_id(),
            object_key: intent.object_key(),
        };
        let object = commit_object_version_in_transaction(
            &mut transaction,
            &commit,
            intent.object_tags(),
            expected,
        )
        .await?;

        let result = sqlx::query(
            "UPDATE s3_upload_intents
             SET state = 'committed', lease_token = NULL, lease_until = NULL,
                 committed_object_id = $1, committed_version_id = $2, updated_at = $3
             WHERE id = $4 AND state = 'committing' AND lease_token = $5
               AND lease_until > $3",
        )
        .bind(object.id().as_uuid())
        .bind(commit.version.id().as_uuid())
        .bind(committed_at)
        .bind(intent_id.as_uuid())
        .bind(lease_token)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }

        transaction.commit().await.map_err(database_error)?;
        Ok(object)
    }

    async fn abort_upload_intent_and_enqueue_gc(
        &self,
        intent_id: UploadIntentId,
        gc_task: NewStorageGcTask,
        now: OffsetDateTime,
    ) -> Result<UploadIntent, RepositoryError> {
        gc_task
            .validate()
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        let now = postgres_time(now);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let intent = lock_upload_intent(&mut transaction, intent_id).await?;
        if intent.state() == UploadIntentState::Committed {
            return Err(RepositoryError::Conflict);
        }
        validate_upload_intent_gc_template(&intent, &gc_task)?;
        enqueue_upload_intent_cleanup(&mut transaction, &intent, &gc_task).await?;

        let aborted = match intent.state() {
            UploadIntentState::Aborted | UploadIntentState::Expired => intent,
            UploadIntentState::Staging
            | UploadIntentState::Ready
            | UploadIntentState::Committing => {
                let row = sqlx::query(
                    "UPDATE s3_upload_intents
                     SET state = 'aborted', lease_token = NULL, lease_until = NULL, updated_at = $2
                     WHERE id = $1 AND state <> 'committed'
                     RETURNING *",
                )
                .bind(intent_id.as_uuid())
                .bind(now)
                .fetch_one(&mut *transaction)
                .await
                .map_err(database_error)?;
                row_to_upload_intent(row)?
            }
            UploadIntentState::Committed => unreachable!("handled above"),
        };

        transaction.commit().await.map_err(database_error)?;
        Ok(aborted)
    }

    async fn expire_upload_intents(
        &self,
        now: OffsetDateTime,
        limit: usize,
        gc_max_attempts: u32,
    ) -> Result<usize, RepositoryError> {
        if limit == 0 || limit > MAX_UPLOAD_INTENT_EXPIRY_LIMIT || gc_max_attempts == 0 {
            return Err(RepositoryError::Invariant(
                "upload intent expiry limit and GC max attempts are invalid".into(),
            ));
        }
        let limit = i64::try_from(limit)
            .map_err(|_| RepositoryError::Invariant("expiry limit exceeds i64".into()))?;
        let now = postgres_time(now);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let rows = sqlx::query(
            "SELECT * FROM s3_upload_intents
             WHERE expires_at <= $1
               AND (state IN ('staging', 'ready')
                 OR (state = 'committing' AND lease_until <= $1))
             ORDER BY expires_at, id
             FOR UPDATE SKIP LOCKED
             LIMIT $2",
        )
        .bind(now)
        .bind(limit)
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;
        let intents = rows
            .into_iter()
            .map(row_to_upload_intent)
            .collect::<Result<Vec<_>, _>>()?;

        for intent in &intents {
            enqueue_expired_upload_intent_cleanup(&mut transaction, intent, gc_max_attempts, now)
                .await?;
            let result = sqlx::query(
                "UPDATE s3_upload_intents
                 SET state = 'expired', lease_token = NULL, lease_until = NULL, updated_at = $2
                 WHERE id = $1
                   AND (state IN ('staging', 'ready')
                     OR (state = 'committing' AND lease_until <= $2))",
            )
            .bind(intent.id().as_uuid())
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
            if result.rows_affected() != 1 {
                return Err(RepositoryError::Conflict);
            }
        }

        transaction.commit().await.map_err(database_error)?;
        Ok(intents.len())
    }

    async fn commit_multipart_object_version(
        &self,
        upload_id: &str,
        completion_token: &str,
        commit: S3ObjectVersionCommit,
        final_entity_tag: &EntityTag,
        final_checksum: &Checksum,
    ) -> Result<S3Object, RepositoryError> {
        validate_token(upload_id)?;
        validate_token(completion_token)?;
        commit.validate()?;
        let mut commit = commit;
        validate_multipart_commit_payload(&commit, final_entity_tag, final_checksum)?;
        if !is_multipart_etag(final_entity_tag.as_str())
            || final_checksum.algorithm() != ChecksumAlgorithm::Sha256
        {
            return Err(RepositoryError::Invariant(
                "multipart completion requires a canonical multipart ETag and SHA-256 checksum"
                    .into(),
            ));
        }

        let committed_at = postgres_time(commit.committed_at);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query("SELECT * FROM s3_multipart_uploads WHERE upload_id = $1 FOR UPDATE")
            .bind(upload_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
            .ok_or(RepositoryError::NotFound)?;

        let state: String = row.try_get("state").map_err(database_error)?;
        let application_id =
            ApplicationId::from_uuid(row.try_get("application_id").map_err(database_error)?);
        let bucket_id = BucketId::from_uuid(row.try_get("bucket_id").map_err(database_error)?);
        let object_key: String = row.try_get("object_key").map_err(database_error)?;
        let content_type: String = row.try_get("content_type").map_err(database_error)?;
        let user_metadata = row
            .try_get::<Json<Value>, _>("user_metadata")
            .map_err(database_error)?
            .0;
        let object_tags = validated_tag_set(
            row.try_get::<Json<S3ObjectTagSet>, _>("object_tags")
                .map_err(database_error)?
                .0,
        )?;
        let storage_backend: String = row.try_get("storage_backend").map_err(database_error)?;
        let intent_id = UploadIntentId::from_uuid(
            row.try_get::<Option<uuid::Uuid>, _>("upload_intent_id")
                .map_err(database_error)?
                .ok_or_else(|| invariant("multipart completion has no attached upload intent"))?,
        );
        let intent = lock_upload_intent(&mut transaction, intent_id).await?;
        let payload = stored_payload(&commit.version)?;
        if commit.version.application_id() != application_id
            || commit.version.bucket_id() != bucket_id
            || intent.application_id() != application_id
            || intent.bucket_id() != bucket_id
            || intent.object_key() != object_key
            || intent.storage_backend() != storage_backend
            || intent.content_type() != Some(content_type.as_str())
            || intent.user_metadata() != &user_metadata
            || intent.object_tags() != &object_tags
            || payload.content_type() != Some(content_type.as_str())
            || payload.user_metadata() != &user_metadata
        {
            return Err(RepositoryError::Conflict);
        }

        let facts = sqlx::query(
            "SELECT
                CASE WHEN COUNT(*) > 0 THEN
                    md5(decode(string_agg(part.md5, '' ORDER BY item.ordinality), 'hex'))
                    || '-' || COUNT(*)::TEXT
                END AS calculated_etag,
                COALESCE(SUM(part.size_bytes), 0)::BIGINT AS total_size,
                COUNT(*)::BIGINT AS part_count,
                COALESCE(MAX(jsonb_array_length(upload.completion_manifest)), 0)::BIGINT
                    AS manifest_count
             FROM s3_multipart_uploads AS upload
             CROSS JOIN LATERAL jsonb_array_elements(upload.completion_manifest)
                 WITH ORDINALITY AS item(value, ordinality)
             JOIN s3_multipart_parts AS part
               ON part.upload_id = upload.upload_id
              AND part.part_number = (item.value ->> 'part_number')::INTEGER
             WHERE upload.upload_id = $1",
        )
        .bind(upload_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let calculated_etag = facts
            .try_get::<Option<String>, _>("calculated_etag")
            .map_err(database_error)?;
        let total_size = as_u64(facts.try_get("total_size").map_err(database_error)?)?;
        let part_count = facts
            .try_get::<i64, _>("part_count")
            .map_err(database_error)?;
        let manifest_count = facts
            .try_get::<i64, _>("manifest_count")
            .map_err(database_error)?;
        if part_count == 0
            || part_count != manifest_count
            || calculated_etag.as_deref() != Some(final_entity_tag.as_str())
            || intent.expected_size_bytes() != total_size
            || intent.size_bytes() != Some(total_size)
        {
            return Err(RepositoryError::Conflict);
        }

        let expected = ExpectedObjectIdentity {
            application_id,
            bucket_id,
            object_key: &object_key,
        };
        if state == "completed" {
            let object_id = row
                .try_get::<Option<uuid::Uuid>, _>("object_id")
                .map_err(database_error)?
                .ok_or_else(|| invariant("completed multipart upload has no object_id"))?;
            let version_id = row
                .try_get::<Option<uuid::Uuid>, _>("object_version_id")
                .map_err(database_error)?
                .ok_or_else(|| invariant("completed multipart upload has no object_version_id"))?;
            validate_completed_multipart_replay(
                &row,
                &commit,
                final_entity_tag,
                final_checksum,
                version_id,
            )?;
            if object_id != commit.version.object_id().as_uuid()
                || intent.state() != UploadIntentState::Committed
                || intent.proposed_version_id() != commit.version.id()
                || intent.committed_object_id().map(ObjectId::as_uuid) != Some(object_id)
                || intent.committed_version_id().map(ObjectVersionId::as_uuid) != Some(version_id)
                || intent.entity_tag() != Some(final_entity_tag)
                || intent.checksum() != Some(final_checksum)
            {
                return Err(RepositoryError::Conflict);
            }
            let object_row = sqlx::query("SELECT * FROM objects WHERE id = $1")
                .bind(object_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(database_error)?
                .ok_or(RepositoryError::NotFound)?;
            let object = row_to_s3_object(object_row)?;
            validate_expected_object_identity(&object, &expected)?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(object);
        }
        if state != "completing"
            || row
                .try_get::<Option<String>, _>("completion_token")
                .map_err(database_error)?
                .as_deref()
                != Some(completion_token)
            || row
                .try_get::<Option<OffsetDateTime>, _>("completion_lease_until")
                .map_err(database_error)?
                .is_none_or(|lease_until| lease_until <= committed_at)
        {
            return Err(RepositoryError::Conflict);
        }
        validate_upload_intent_commit(&intent, completion_token, &commit)?;
        if intent
            .lease_until()
            .is_none_or(|lease_until| lease_until <= committed_at)
        {
            return Err(RepositoryError::Conflict);
        }

        freeze_postgres_object_lock(&mut transaction, &mut commit).await?;

        let object = commit_object_version_in_transaction(
            &mut transaction,
            &commit,
            intent.object_tags(),
            expected,
        )
        .await?;
        let intent_updated = sqlx::query(
            "UPDATE s3_upload_intents
             SET state = 'committed', lease_token = NULL, lease_until = NULL,
                 committed_object_id = $1, committed_version_id = $2, updated_at = $3
             WHERE id = $4 AND state = 'committing' AND lease_token = $5
               AND lease_until > $3",
        )
        .bind(object.id().as_uuid())
        .bind(commit.version.id().as_uuid())
        .bind(committed_at)
        .bind(intent_id.as_uuid())
        .bind(completion_token)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if intent_updated.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }

        let part_storage_keys = sqlx::query_scalar::<_, String>(
            "SELECT storage_key FROM s3_multipart_parts
             WHERE upload_id = $1 ORDER BY part_number",
        )
        .bind(upload_id)
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;
        for storage_key in part_storage_keys
            .into_iter()
            .chain(std::iter::once(intent.temporary_storage_key().to_owned()))
        {
            let task = NewStorageGcTask {
                id: StorageGcTaskId::new(),
                application_id,
                bucket_id,
                object_version_id: None,
                upload_intent_id: Some(intent_id),
                multipart_upload_id: Some(upload_id.to_owned()),
                storage_backend: storage_backend.clone(),
                storage_key,
                reason: StorageGcReason::MultipartTemporary,
                not_before: committed_at,
                max_attempts: DEFAULT_S3_MULTIPART_GC_MAX_ATTEMPTS,
                created_at: committed_at,
            };
            insert_storage_gc_task(&mut transaction, &task).await?;
        }

        let result = sqlx::query(
            "UPDATE s3_multipart_uploads
             SET state = 'completed', completion_token = NULL, completion_lease_until = NULL,
                 object_id = $1, object_version_id = $2, final_etag = $3,
                 final_checksum_algorithm = $4, final_checksum_value = $5,
                 completed_at = $6, updated_at = $6
             WHERE upload_id = $7 AND state = 'completing' AND completion_token = $8
               AND completion_lease_until > $6 AND upload_intent_id = $9",
        )
        .bind(object.id().as_uuid())
        .bind(commit.version.id().as_uuid())
        .bind(final_entity_tag.as_str())
        .bind(checksum_algorithm_name(final_checksum.algorithm()))
        .bind(final_checksum.value())
        .bind(committed_at)
        .bind(upload_id)
        .bind(completion_token)
        .bind(intent_id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }

        transaction.commit().await.map_err(database_error)?;
        Ok(object)
    }
}
#[async_trait]
impl StorageGcRepository for PostgresRepository {
    async fn enqueue_storage_gc_task(&self, task: NewStorageGcTask) -> Result<(), RepositoryError> {
        task.validate()
            .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        insert_storage_gc_task(&mut transaction, &task).await?;
        transaction.commit().await.map_err(database_error)
    }

    async fn claim_storage_gc_tasks(
        &self,
        lease_token: &str,
        lease_until: OffsetDateTime,
        now: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<StorageGcTask>, RepositoryError> {
        validate_lease(lease_token, lease_until, now)?;
        if limit == 0 || limit > MAX_STORAGE_GC_CLAIM_LIMIT {
            return Err(RepositoryError::Invariant(
                "storage GC claim limit is invalid".into(),
            ));
        }
        let limit = i64::try_from(limit)
            .map_err(|_| RepositoryError::Invariant("GC claim limit exceeds i64".into()))?;
        let now = postgres_time(now);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;

        sqlx::query(
            "UPDATE storage_gc_tasks
             SET state = 'dead_letter', lease_token = NULL, lease_until = NULL, updated_at = $1
             WHERE attempts >= max_attempts
               AND ((state = 'pending' AND not_before <= $1)
                 OR (state = 'leased' AND lease_until <= $1))",
        )
        .bind(now)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;

        let rows = sqlx::query(CLAIM_STORAGE_GC_TASKS_SQL)
            .bind(lease_token)
            .bind(postgres_time(lease_until))
            .bind(now)
            .bind(limit)
            .fetch_all(&mut *transaction)
            .await
            .map_err(database_error)?;
        let tasks = rows
            .into_iter()
            .map(row_to_storage_gc_task)
            .collect::<Result<Vec<_>, _>>()?;
        transaction.commit().await.map_err(database_error)?;
        Ok(tasks)
    }

    async fn complete_storage_gc_task(
        &self,
        task_id: StorageGcTaskId,
        lease_token: &str,
        completed_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        validate_token(lease_token)?;
        let completed_at = postgres_time(completed_at);
        let result = sqlx::query(
            "UPDATE storage_gc_tasks
             SET state = 'completed', lease_token = NULL, lease_until = NULL,
                 completed_at = $3, updated_at = $3
             WHERE id = $1 AND state = 'leased' AND lease_token = $2
               AND lease_until > $3",
        )
        .bind(task_id.as_uuid())
        .bind(lease_token)
        .bind(completed_at)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 1 {
            Ok(())
        } else {
            storage_gc_write_miss(&self.pool, task_id).await
        }
    }

    async fn fail_storage_gc_task(
        &self,
        task_id: StorageGcTaskId,
        lease_token: &str,
        error: &str,
        retry_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> Result<StorageGcTask, RepositoryError> {
        validate_token(lease_token)?;
        if error.len() > 4_096 || retry_at < now {
            return Err(RepositoryError::Invariant(
                "storage GC failure metadata is invalid".into(),
            ));
        }
        let retry_at = postgres_time(retry_at);
        let now = postgres_time(now);
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let row = sqlx::query("SELECT * FROM storage_gc_tasks WHERE id = $1 FOR UPDATE")
            .bind(task_id.as_uuid())
            .fetch_optional(&mut *transaction)
            .await
            .map_err(database_error)?
            .ok_or(RepositoryError::NotFound)?;
        let task = row_to_storage_gc_task(row)?;
        if task.state != StorageGcTaskState::Leased
            || task.lease_token.as_deref() != Some(lease_token)
            || task
                .lease_until
                .is_none_or(|lease_until| lease_until <= now)
        {
            return Err(RepositoryError::Conflict);
        }

        let next_state = if task.attempts >= task.task.max_attempts {
            "dead_letter"
        } else {
            "pending"
        };
        let row = sqlx::query(
            "UPDATE storage_gc_tasks
             SET state = $1,
                 not_before = CASE WHEN $1 = 'pending' THEN $2 ELSE not_before END,
                 lease_token = NULL, lease_until = NULL, last_error = $3, updated_at = $4
             WHERE id = $5 AND state = 'leased' AND lease_token = $6
               AND lease_until > $4
             RETURNING *",
        )
        .bind(next_state)
        .bind(retry_at)
        .bind(error)
        .bind(now)
        .bind(task_id.as_uuid())
        .bind(lease_token)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::Conflict)?;
        let failed = row_to_storage_gc_task(row)?;
        transaction.commit().await.map_err(database_error)?;
        Ok(failed)
    }
}
async fn insert_object_version(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    version: &ObjectVersion,
) -> Result<(), RepositoryError> {
    let (
        storage_backend,
        storage_key,
        provider_etag,
        provider_version,
        etag,
        size_bytes,
        content_type,
        user_metadata,
        checksum_algorithm,
        checksum_value,
    ) = match version.payload() {
        ObjectVersionPayload::Object(object) => (
            Some(object.storage_backend()),
            Some(object.storage_key()),
            object.provider_etag().map(EntityTag::as_str),
            object.provider_version(),
            Some(object.etag().as_str()),
            Some(as_i64(object.size_bytes())?),
            object.content_type(),
            object.user_metadata().clone(),
            object
                .checksum()
                .map(|value| checksum_algorithm_name(value.algorithm())),
            object.checksum().map(Checksum::value),
        ),
        ObjectVersionPayload::DeleteMarker => (
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            json!({}),
            None,
            None,
        ),
    };
    let retention_mode = version
        .retention()
        .map(|value| retention_mode_name(value.mode()));
    let retain_until = version
        .retention()
        .map(|value| postgres_time(value.retain_until()));
    sqlx::query(
        "INSERT INTO object_versions (
            id, object_id, application_id, bucket_id, external_version_id, generation,
            is_null_version, is_delete_marker, state, storage_backend, storage_key,
            provider_etag, provider_version, etag, size_bytes, content_type, user_metadata,
            checksum_algorithm, checksum_value, retention_mode, retain_until, legal_hold,
            created_by, source_protocol, created_at, became_noncurrent_at
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
            $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26
         )",
    )
    .bind(version.id().as_uuid())
    .bind(version.object_id().as_uuid())
    .bind(version.application_id().as_uuid())
    .bind(version.bucket_id().as_uuid())
    .bind(version.external_version_id().as_str())
    .bind(as_i64(version.generation())?)
    .bind(version.is_null_version())
    .bind(version.is_delete_marker())
    .bind(object_version_state_name(version.state()))
    .bind(storage_backend)
    .bind(storage_key)
    .bind(provider_etag)
    .bind(provider_version)
    .bind(etag)
    .bind(size_bytes)
    .bind(content_type)
    .bind(Json(user_metadata))
    .bind(checksum_algorithm)
    .bind(checksum_value)
    .bind(retention_mode)
    .bind(retain_until)
    .bind(version.legal_hold())
    .bind(version.created_by())
    .bind(source_protocol_name(version.source_protocol()))
    .bind(postgres_time(version.created_at()))
    .bind(version.became_noncurrent_at().map(postgres_time))
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(())
}

async fn insert_object_version_tags(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    version_id: ObjectVersionId,
    tags: &S3ObjectTagSet,
) -> Result<(), RepositoryError> {
    tags.validate()
        .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
    for (position, tag) in tags.iter().enumerate() {
        let position = i16::try_from(position)
            .map_err(|_| RepositoryError::Invariant("tag position overflow".into()))?;
        sqlx::query(
            "INSERT INTO object_version_tags (
                object_version_id, is_delete_marker, position, tag_key, tag_value
             ) VALUES ($1, FALSE, $2, $3, $4)",
        )
        .bind(version_id.as_uuid())
        .bind(position)
        .bind(tag.key())
        .bind(tag.value())
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    }
    Ok(())
}

async fn load_object_version_tags(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    version_id: ObjectVersionId,
) -> Result<S3ObjectTagSet, RepositoryError> {
    let rows = sqlx::query(
        "SELECT tag_key, tag_value FROM object_version_tags
         WHERE object_version_id = $1 ORDER BY position",
    )
    .bind(version_id.as_uuid())
    .fetch_all(&mut **transaction)
    .await
    .map_err(database_error)?;
    let tags = rows
        .into_iter()
        .map(|row| {
            S3ObjectTag::new(
                row.try_get::<String, _>("tag_key")
                    .map_err(database_error)?,
                row.try_get::<String, _>("tag_value")
                    .map_err(database_error)?,
            )
            .map_err(|error| RepositoryError::Invariant(error.to_string()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    S3ObjectTagSet::new(tags).map_err(|error| RepositoryError::Invariant(error.to_string()))
}

fn validated_tag_set(tags: S3ObjectTagSet) -> Result<S3ObjectTagSet, RepositoryError> {
    tags.validate()
        .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
    Ok(tags)
}

struct ExpectedObjectIdentity<'a> {
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: &'a str,
}

pub(crate) async fn lock_upload_intent(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    intent_id: UploadIntentId,
) -> Result<UploadIntent, RepositoryError> {
    let row = sqlx::query("SELECT * FROM s3_upload_intents WHERE id = $1 FOR UPDATE")
        .bind(intent_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
    row_to_upload_intent(row)
}

async fn upload_intent_write_miss(
    pool: &sqlx::PgPool,
    intent_id: UploadIntentId,
) -> Result<UploadIntent, RepositoryError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM s3_upload_intents WHERE id = $1)",
    )
    .bind(intent_id.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    if exists {
        Err(RepositoryError::Conflict)
    } else {
        Err(RepositoryError::NotFound)
    }
}

async fn storage_gc_write_miss(
    pool: &sqlx::PgPool,
    task_id: StorageGcTaskId,
) -> Result<(), RepositoryError> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM storage_gc_tasks WHERE id = $1)",
    )
    .bind(task_id.as_uuid())
    .fetch_one(pool)
    .await
    .map_err(database_error)?;
    if exists {
        Err(RepositoryError::Conflict)
    } else {
        Err(RepositoryError::NotFound)
    }
}

fn validate_token(token: &str) -> Result<(), RepositoryError> {
    if token.is_empty() || token.len() > 1_024 || token.bytes().any(|byte| byte.is_ascii_control())
    {
        Err(RepositoryError::Invariant(
            "lease or upload token is invalid".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_lease(
    lease_token: &str,
    lease_until: OffsetDateTime,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    validate_token(lease_token)?;
    if lease_until <= now {
        Err(RepositoryError::Invariant(
            "lease_until must be later than now".into(),
        ))
    } else {
        Ok(())
    }
}

fn stored_payload(version: &ObjectVersion) -> Result<&StoredObjectVersion, RepositoryError> {
    match version.payload() {
        ObjectVersionPayload::Object(payload) => Ok(payload),
        ObjectVersionPayload::DeleteMarker => Err(RepositoryError::Invariant(
            "an upload cannot commit a delete marker".into(),
        )),
    }
}

fn validate_upload_intent_commit(
    intent: &UploadIntent,
    lease_token: &str,
    commit: &S3ObjectVersionCommit,
) -> Result<(), RepositoryError> {
    if intent.state() != UploadIntentState::Committing
        || intent.lease_token() != Some(lease_token)
        || intent.proposed_version_id() != commit.version.id()
        || intent.application_id() != commit.version.application_id()
        || intent.bucket_id() != commit.version.bucket_id()
    {
        return Err(RepositoryError::Conflict);
    }

    let payload = stored_payload(&commit.version)?;
    if payload.storage_backend() != intent.storage_backend()
        || payload.storage_key() != intent.final_storage_key()
        || Some(payload.etag()) != intent.entity_tag()
        || payload.checksum() != intent.checksum()
        || Some(payload.size_bytes()) != intent.size_bytes()
        || payload.size_bytes() != intent.expected_size_bytes()
        || payload.content_type() != intent.content_type()
        || payload.user_metadata() != intent.user_metadata()
    {
        return Err(RepositoryError::Conflict);
    }
    Ok(())
}

fn validate_multipart_commit_payload(
    commit: &S3ObjectVersionCommit,
    final_entity_tag: &EntityTag,
    final_checksum: &Checksum,
) -> Result<(), RepositoryError> {
    let payload = stored_payload(&commit.version)?;
    if payload.etag() != final_entity_tag || payload.checksum() != Some(final_checksum) {
        return Err(RepositoryError::Conflict);
    }
    Ok(())
}

fn validate_completed_multipart_replay(
    row: &PgRow,
    commit: &S3ObjectVersionCommit,
    final_entity_tag: &EntityTag,
    final_checksum: &Checksum,
    persisted_version_id: uuid::Uuid,
) -> Result<(), RepositoryError> {
    let persisted_etag = required_entity_tag(row, "final_etag")?;
    let persisted_checksum = row_checksum(row, "final_checksum_algorithm", "final_checksum_value")?;
    if persisted_version_id != commit.version.id().as_uuid()
        || persisted_etag != *final_entity_tag
        || persisted_checksum.as_ref() != Some(final_checksum)
    {
        return Err(RepositoryError::Conflict);
    }
    Ok(())
}

fn validate_expected_object_identity(
    object: &S3Object,
    expected: &ExpectedObjectIdentity<'_>,
) -> Result<(), RepositoryError> {
    if object.application_id() != expected.application_id
        || object.bucket_id() != expected.bucket_id
        || object.key() != expected.object_key
    {
        Err(RepositoryError::Conflict)
    } else {
        Ok(())
    }
}

fn validate_null_version_replacement(
    commit: &S3ObjectVersionCommit,
    active_null: Option<&ObjectVersion>,
) -> Result<(), RepositoryError> {
    let replacement_tasks = commit
        .gc_tasks
        .iter()
        .filter(|task| task.reason == StorageGcReason::ReplacedNullVersion)
        .collect::<Vec<_>>();

    if !commit.version.is_null_version() {
        return if commit.replaced_null_version_id.is_none() && replacement_tasks.is_empty() {
            Ok(())
        } else {
            Err(RepositoryError::Conflict)
        };
    }

    if active_null.map(ObjectVersion::id) != commit.replaced_null_version_id {
        return Err(RepositoryError::Conflict);
    }

    let Some(previous) = active_null else {
        return if replacement_tasks.is_empty() {
            Ok(())
        } else {
            Err(RepositoryError::Conflict)
        };
    };
    if previous.state() != ObjectVersionState::Committed || !previous.is_null_version() {
        return Err(RepositoryError::Conflict);
    }

    match previous.payload() {
        ObjectVersionPayload::DeleteMarker => {
            if replacement_tasks.is_empty() {
                Ok(())
            } else {
                Err(RepositoryError::Conflict)
            }
        }
        ObjectVersionPayload::Object(payload) => {
            let [task] = replacement_tasks.as_slice() else {
                return Err(RepositoryError::Conflict);
            };
            if task.application_id == previous.application_id()
                && task.bucket_id == previous.bucket_id()
                && task.object_version_id == Some(previous.id())
                && task.upload_intent_id.is_none()
                && task.multipart_upload_id.is_none()
                && task.storage_backend == payload.storage_backend()
                && task.storage_key == payload.storage_key()
            {
                Ok(())
            } else {
                Err(RepositoryError::Conflict)
            }
        }
    }
}

async fn lock_active_null_version(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    object_id: ObjectId,
) -> Result<Option<ObjectVersion>, RepositoryError> {
    sqlx::query(LOCK_ACTIVE_NULL_VERSION_SQL)
        .bind(object_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?
        .map(row_to_object_version)
        .transpose()
}

async fn supersede_active_null_version(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    active_null: Option<&ObjectVersion>,
    superseded_at: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let Some(previous) = active_null else {
        return Ok(());
    };
    let result = sqlx::query(SUPERSEDE_ACTIVE_NULL_VERSION_SQL)
        .bind(postgres_time(superseded_at))
        .bind(previous.id().as_uuid())
        .bind(previous.object_id().as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
    if result.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn freeze_postgres_object_lock(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    commit: &mut S3ObjectVersionCommit,
) -> Result<(), RepositoryError> {
    let row = sqlx::query(
        "SELECT * FROM buckets
         WHERE application_id = $1 AND id = $2 FOR UPDATE",
    )
    .bind(commit.version.application_id().as_uuid())
    .bind(commit.version.bucket_id().as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or(RepositoryError::NotFound)?;
    let configuration = row_to_s3_configuration(&row)?;
    let requested = commit.requested_retention.is_some() || commit.requested_legal_hold.is_some();
    if requested && !configuration.object_lock_enabled() {
        return Err(RepositoryError::Conflict);
    }
    let retention = if configuration.object_lock_enabled() {
        match (
            commit.requested_retention,
            configuration.default_retention(),
        ) {
            (Some(retention), _) => Some(retention),
            (None, Some(default_retention)) => Some(
                default_retention
                    .for_object_at(commit.committed_at)
                    .map_err(invariant)?,
            ),
            (None, None) => None,
        }
    } else {
        None
    };
    commit.version = commit
        .version
        .with_initial_object_lock(retention, commit.requested_legal_hold.unwrap_or(false))
        .map_err(invariant)?;
    Ok(())
}

async fn commit_object_version_in_transaction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    commit: &S3ObjectVersionCommit,
    object_tags: &S3ObjectTagSet,
    expected: ExpectedObjectIdentity<'_>,
) -> Result<S3Object, RepositoryError> {
    commit.validate()?;
    let advanced = match &commit.target {
        S3ObjectCommitTarget::Create(object) => {
            validate_expected_object_identity(object, &expected)?;
            validate_null_version_replacement(commit, None)?;
            let advanced = object
                .advanced_to(&commit.version, commit.committed_at)
                .map_err(invariant)?;
            sqlx::query(
                "INSERT INTO objects (
                    id, application_id, bucket_id, object_key, current_version_id,
                    generation, created_at, updated_at
                 ) VALUES ($1, $2, $3, $4, NULL, 0, $5, $6)",
            )
            .bind(object.id().as_uuid())
            .bind(object.application_id().as_uuid())
            .bind(object.bucket_id().as_uuid())
            .bind(object.key())
            .bind(postgres_time(object.created_at()))
            .bind(postgres_time(object.updated_at()))
            .execute(&mut **transaction)
            .await
            .map_err(database_error)?;

            insert_object_version(transaction, &commit.version).await?;
            let result = sqlx::query(
                "UPDATE objects
                 SET current_version_id = $1, generation = $2, updated_at = $3
                 WHERE id = $4 AND generation = 0",
            )
            .bind(commit.version.id().as_uuid())
            .bind(as_i64(advanced.generation())?)
            .bind(postgres_time(advanced.updated_at()))
            .bind(object.id().as_uuid())
            .execute(&mut **transaction)
            .await
            .map_err(database_error)?;
            if result.rows_affected() != 1 {
                return Err(RepositoryError::Conflict);
            }
            advanced
        }
        S3ObjectCommitTarget::Append {
            object_id,
            expected_generation,
        } => {
            let row = sqlx::query("SELECT * FROM objects WHERE id = $1 FOR UPDATE")
                .bind(object_id.as_uuid())
                .fetch_optional(&mut **transaction)
                .await
                .map_err(database_error)?
                .ok_or(RepositoryError::NotFound)?;
            let object = row_to_s3_object(row)?;
            validate_expected_object_identity(&object, &expected)?;
            if object.generation() != *expected_generation {
                return Err(RepositoryError::Conflict);
            }
            let active_null = if commit.version.is_null_version() {
                lock_active_null_version(transaction, object.id()).await?
            } else {
                None
            };
            validate_null_version_replacement(commit, active_null.as_ref())?;

            let advanced = object
                .advanced_to(&commit.version, commit.committed_at)
                .map_err(invariant)?;
            let current_version_id = object.current_version_id().ok_or_else(|| {
                RepositoryError::Invariant(
                    "an existing versioned object must have a current version".into(),
                )
            })?;
            let result = sqlx::query(
                "UPDATE object_versions
                 SET became_noncurrent_at = $1
                 WHERE id = $2 AND object_id = $3 AND became_noncurrent_at IS NULL",
            )
            .bind(postgres_time(commit.committed_at))
            .bind(current_version_id.as_uuid())
            .bind(object.id().as_uuid())
            .execute(&mut **transaction)
            .await
            .map_err(database_error)?;
            if result.rows_affected() != 1 {
                return Err(RepositoryError::Conflict);
            }

            supersede_active_null_version(transaction, active_null.as_ref(), commit.committed_at)
                .await?;
            insert_object_version(transaction, &commit.version).await?;
            let result = sqlx::query(
                "UPDATE objects
                 SET current_version_id = $1, generation = $2, updated_at = $3
                 WHERE id = $4 AND generation = $5",
            )
            .bind(commit.version.id().as_uuid())
            .bind(as_i64(advanced.generation())?)
            .bind(postgres_time(advanced.updated_at()))
            .bind(object.id().as_uuid())
            .bind(as_i64(*expected_generation)?)
            .execute(&mut **transaction)
            .await
            .map_err(database_error)?;
            if result.rows_affected() != 1 {
                return Err(RepositoryError::Conflict);
            }
            advanced
        }
    };

    insert_object_version_tags(transaction, commit.version.id(), object_tags).await?;

    for task in &commit.gc_tasks {
        if task.application_id != commit.version.application_id()
            || task.bucket_id != commit.version.bucket_id()
        {
            return Err(RepositoryError::Conflict);
        }
        insert_storage_gc_task(transaction, task).await?;
        if task.reason == StorageGcReason::ReplacedNullVersion {
            ensure_persisted_null_replacement_gc_task(transaction, task).await?;
        }
    }
    Ok(advanced)
}

fn validate_upload_intent_gc_template(
    intent: &UploadIntent,
    task: &NewStorageGcTask,
) -> Result<(), RepositoryError> {
    let storage_key_is_reserved = task.storage_key == intent.temporary_storage_key()
        || task.storage_key == intent.final_storage_key();
    if task.application_id != intent.application_id()
        || task.bucket_id != intent.bucket_id()
        || task.upload_intent_id != Some(intent.id())
        || task.object_version_id.is_some()
        || task.multipart_upload_id.is_some()
        || task.storage_backend != intent.storage_backend()
        || !storage_key_is_reserved
        || task.reason != StorageGcReason::AbortedUploadIntent
    {
        return Err(RepositoryError::Conflict);
    }
    Ok(())
}

fn upload_intent_gc_task_for_key(
    template: &NewStorageGcTask,
    storage_key: &str,
) -> NewStorageGcTask {
    let mut task = template.clone();
    if task.storage_key != storage_key {
        task.id = StorageGcTaskId::new();
        task.storage_key = storage_key.to_owned();
    }
    task
}

async fn enqueue_upload_intent_cleanup(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    intent: &UploadIntent,
    template: &NewStorageGcTask,
) -> Result<(), RepositoryError> {
    let temporary = upload_intent_gc_task_for_key(template, intent.temporary_storage_key());
    let final_task = upload_intent_gc_task_for_key(template, intent.final_storage_key());
    insert_storage_gc_task(transaction, &temporary).await?;
    insert_storage_gc_task(transaction, &final_task).await
}

async fn enqueue_expired_upload_intent_cleanup(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    intent: &UploadIntent,
    max_attempts: u32,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    for storage_key in [intent.temporary_storage_key(), intent.final_storage_key()] {
        let task = NewStorageGcTask {
            id: StorageGcTaskId::new(),
            application_id: intent.application_id(),
            bucket_id: intent.bucket_id(),
            object_version_id: None,
            upload_intent_id: Some(intent.id()),
            multipart_upload_id: None,
            storage_backend: intent.storage_backend().to_owned(),
            storage_key: storage_key.to_owned(),
            reason: StorageGcReason::AbortedUploadIntent,
            not_before: now,
            max_attempts,
            created_at: now,
        };
        insert_storage_gc_task(transaction, &task).await?;
    }
    Ok(())
}

pub(crate) async fn insert_storage_gc_task(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &NewStorageGcTask,
) -> Result<(), RepositoryError> {
    task.validate()
        .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
    let max_attempts = i32::try_from(task.max_attempts)
        .map_err(|_| RepositoryError::Invariant("GC max attempts exceeds i32".into()))?;
    sqlx::query(
        "INSERT INTO storage_gc_tasks (
            id, application_id, bucket_id, object_version_id, upload_intent_id,
            multipart_upload_id, storage_backend, storage_key, reason, state,
            not_before, lease_token, lease_until, attempts, max_attempts,
            last_error, completed_at, created_at, updated_at
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, 'pending',
            $10, NULL, NULL, 0, $11, NULL, NULL, $12, $12
         )
         ON CONFLICT (storage_backend, storage_key) DO NOTHING",
    )
    .bind(task.id.as_uuid())
    .bind(task.application_id.as_uuid())
    .bind(task.bucket_id.as_uuid())
    .bind(task.object_version_id.map(ObjectVersionId::as_uuid))
    .bind(task.upload_intent_id.map(UploadIntentId::as_uuid))
    .bind(task.multipart_upload_id.as_deref())
    .bind(&task.storage_backend)
    .bind(&task.storage_key)
    .bind(storage_gc_reason_name(task.reason))
    .bind(postgres_time(task.not_before))
    .bind(max_attempts)
    .bind(postgres_time(task.created_at))
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;

    let row = sqlx::query(
        "SELECT * FROM storage_gc_tasks WHERE storage_backend = $1 AND storage_key = $2",
    )
    .bind(&task.storage_backend)
    .bind(&task.storage_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or(RepositoryError::Conflict)?;
    let persisted = row_to_storage_gc_task(row)?.task;
    if persisted.application_id != task.application_id
        || persisted.bucket_id != task.bucket_id
        || persisted.object_version_id != task.object_version_id
        || persisted.upload_intent_id != task.upload_intent_id
        || persisted.multipart_upload_id != task.multipart_upload_id
        || persisted.storage_backend != task.storage_backend
        || persisted.storage_key != task.storage_key
        || persisted.reason != task.reason
    {
        return Err(RepositoryError::Conflict);
    }
    Ok(())
}
async fn ensure_persisted_null_replacement_gc_task(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    expected: &NewStorageGcTask,
) -> Result<(), RepositoryError> {
    let row = sqlx::query(
        "SELECT * FROM storage_gc_tasks WHERE storage_backend = $1 AND storage_key = $2",
    )
    .bind(&expected.storage_backend)
    .bind(&expected.storage_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or(RepositoryError::Conflict)?;
    let persisted = row_to_storage_gc_task(row)?;
    let task = persisted.task;
    if task.application_id == expected.application_id
        && task.bucket_id == expected.bucket_id
        && task.object_version_id == expected.object_version_id
        && task.upload_intent_id.is_none()
        && task.multipart_upload_id.is_none()
        && task.storage_backend == expected.storage_backend
        && task.storage_key == expected.storage_key
        && task.reason == StorageGcReason::ReplacedNullVersion
    {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

fn row_to_upload_intent(row: PgRow) -> Result<UploadIntent, RepositoryError> {
    UploadIntent::from_persistence(PersistedUploadIntent {
        id: UploadIntentId::from_uuid(row.try_get("id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_id: BucketId::from_uuid(row.try_get("bucket_id").map_err(database_error)?),
        object_key: row.try_get("object_key").map_err(database_error)?,
        proposed_version_id: ObjectVersionId::from_uuid(
            row.try_get("proposed_version_id").map_err(database_error)?,
        ),
        state: parse_upload_intent_state(
            &row.try_get::<String, _>("state").map_err(database_error)?,
        )?,
        storage_backend: row.try_get("storage_backend").map_err(database_error)?,
        temporary_storage_key: row
            .try_get("temporary_storage_key")
            .map_err(database_error)?,
        final_storage_key: row.try_get("final_storage_key").map_err(database_error)?,
        entity_tag: optional_entity_tag(&row, "entity_tag")?,
        checksum: row_checksum(&row, "checksum_algorithm", "checksum_value")?,
        expected_size_bytes: as_u64(row.try_get("expected_size_bytes").map_err(database_error)?)?,
        size_bytes: row
            .try_get::<Option<i64>, _>("size_bytes")
            .map_err(database_error)?
            .map(as_u64)
            .transpose()?,
        content_type: row.try_get("content_type").map_err(database_error)?,
        user_metadata: row
            .try_get::<Json<Value>, _>("user_metadata")
            .map_err(database_error)?
            .0,
        object_tags: validated_tag_set(
            row.try_get::<Json<S3ObjectTagSet>, _>("object_tags")
                .map_err(database_error)?
                .0,
        )?,
        lease_token: row.try_get("lease_token").map_err(database_error)?,
        lease_until: row.try_get("lease_until").map_err(database_error)?,
        committed_object_id: row
            .try_get::<Option<uuid::Uuid>, _>("committed_object_id")
            .map_err(database_error)?
            .map(ObjectId::from_uuid),
        committed_version_id: row
            .try_get::<Option<uuid::Uuid>, _>("committed_version_id")
            .map_err(database_error)?
            .map(ObjectVersionId::from_uuid),
        expires_at: row.try_get("expires_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
    .map_err(invariant)
}

fn row_to_storage_gc_task(row: PgRow) -> Result<StorageGcTask, RepositoryError> {
    let task = NewStorageGcTask {
        id: StorageGcTaskId::from_uuid(row.try_get("id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_id: BucketId::from_uuid(row.try_get("bucket_id").map_err(database_error)?),
        object_version_id: row
            .try_get::<Option<uuid::Uuid>, _>("object_version_id")
            .map_err(database_error)?
            .map(ObjectVersionId::from_uuid),
        upload_intent_id: row
            .try_get::<Option<uuid::Uuid>, _>("upload_intent_id")
            .map_err(database_error)?
            .map(UploadIntentId::from_uuid),
        multipart_upload_id: row.try_get("multipart_upload_id").map_err(database_error)?,
        storage_backend: row.try_get("storage_backend").map_err(database_error)?,
        storage_key: row.try_get("storage_key").map_err(database_error)?,
        reason: parse_storage_gc_reason(
            &row.try_get::<String, _>("reason").map_err(database_error)?,
        )?,
        not_before: row.try_get("not_before").map_err(database_error)?,
        max_attempts: as_u32(row.try_get("max_attempts").map_err(database_error)?)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
    };
    let persisted = StorageGcTask {
        task,
        state: parse_storage_gc_task_state(
            &row.try_get::<String, _>("state").map_err(database_error)?,
        )?,
        attempts: as_u32(row.try_get("attempts").map_err(database_error)?)?,
        lease_token: row.try_get("lease_token").map_err(database_error)?,
        lease_until: row.try_get("lease_until").map_err(database_error)?,
        last_error: row.try_get("last_error").map_err(database_error)?,
        completed_at: row.try_get("completed_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    };
    persisted.validate().map_err(invariant)?;
    Ok(persisted)
}

fn parse_upload_intent_state(value: &str) -> Result<UploadIntentState, RepositoryError> {
    match value {
        "staging" => Ok(UploadIntentState::Staging),
        "ready" => Ok(UploadIntentState::Ready),
        "committing" => Ok(UploadIntentState::Committing),
        "committed" => Ok(UploadIntentState::Committed),
        "aborted" => Ok(UploadIntentState::Aborted),
        "expired" => Ok(UploadIntentState::Expired),
        _ => Err(invariant("persisted upload intent state is invalid")),
    }
}

const fn storage_gc_reason_name(reason: StorageGcReason) -> &'static str {
    match reason {
        StorageGcReason::AbortedUploadIntent => "aborted_upload_intent",
        StorageGcReason::MultipartTemporary => "multipart_temporary",
        StorageGcReason::ReplacedNullVersion => "replaced_null_version",
        StorageGcReason::LifecycleExpiration => "lifecycle_expiration",
        StorageGcReason::ExplicitDelete => "explicit_delete",
    }
}

fn parse_storage_gc_reason(value: &str) -> Result<StorageGcReason, RepositoryError> {
    match value {
        "aborted_upload_intent" => Ok(StorageGcReason::AbortedUploadIntent),
        "multipart_temporary" => Ok(StorageGcReason::MultipartTemporary),
        "replaced_null_version" => Ok(StorageGcReason::ReplacedNullVersion),
        "lifecycle_expiration" => Ok(StorageGcReason::LifecycleExpiration),
        "explicit_delete" => Ok(StorageGcReason::ExplicitDelete),
        _ => Err(invariant("persisted storage GC reason is invalid")),
    }
}

fn parse_storage_gc_task_state(value: &str) -> Result<StorageGcTaskState, RepositoryError> {
    match value {
        "pending" => Ok(StorageGcTaskState::Pending),
        "leased" => Ok(StorageGcTaskState::Leased),
        "completed" => Ok(StorageGcTaskState::Completed),
        "dead_letter" => Ok(StorageGcTaskState::DeadLetter),
        _ => Err(invariant("persisted storage GC state is invalid")),
    }
}
fn row_to_s3_bucket(row: PgRow) -> Result<S3Bucket, RepositoryError> {
    let configuration = row_to_s3_configuration(&row)?;
    S3Bucket::from_persistence(PersistedS3Bucket {
        id: BucketId::from_uuid(row.try_get("id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        name: row.try_get("name").map_err(database_error)?,
        configuration: PersistedBucketS3Configuration {
            region: configuration.region().to_owned(),
            versioning_status: configuration.versioning_status(),
            object_lock_enabled: configuration.object_lock_enabled(),
            default_retention: configuration.default_retention(),
            lifecycle_configuration: configuration.lifecycle_configuration().cloned(),
            revision: configuration.revision(),
            updated_at: configuration.updated_at(),
        },
        created_at: row.try_get("created_at").map_err(database_error)?,
    })
    .map_err(invariant)
}

fn row_to_s3_configuration(row: &PgRow) -> Result<BucketS3Configuration, RepositoryError> {
    BucketS3Configuration::from_persistence(PersistedBucketS3Configuration {
        region: row.try_get("region").map_err(database_error)?,
        versioning_status: parse_versioning_status(
            &row.try_get::<String, _>("versioning_status")
                .map_err(database_error)?,
        )?,
        object_lock_enabled: row.try_get("object_lock_enabled").map_err(database_error)?,
        default_retention: row
            .try_get::<Option<Json<DefaultRetention>>, _>("default_retention")
            .map_err(database_error)?
            .map(|value| value.0),
        lifecycle_configuration: row
            .try_get::<Option<Json<S3LifecycleConfiguration>>, _>("s3_lifecycle_configuration")
            .map_err(database_error)?
            .map(|value| value.0),
        revision: as_u64(
            row.try_get("s3_configuration_revision")
                .map_err(database_error)?,
        )?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
    .map_err(invariant)
}

fn row_to_s3_object(row: PgRow) -> Result<S3Object, RepositoryError> {
    S3Object::from_persistence(PersistedS3Object {
        id: ObjectId::from_uuid(row.try_get("id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_id: BucketId::from_uuid(row.try_get("bucket_id").map_err(database_error)?),
        key: row.try_get("object_key").map_err(database_error)?,
        current_version_id: row
            .try_get::<Option<uuid::Uuid>, _>("current_version_id")
            .map_err(database_error)?
            .map(ObjectVersionId::from_uuid),
        generation: as_u64(row.try_get("generation").map_err(database_error)?)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
    .map_err(invariant)
}

fn row_to_object_version(row: PgRow) -> Result<ObjectVersion, RepositoryError> {
    let is_delete_marker: bool = row.try_get("is_delete_marker").map_err(database_error)?;
    let payload = if is_delete_marker {
        ObjectVersionPayload::DeleteMarker
    } else {
        ObjectVersionPayload::Object(Box::new(
            StoredObjectVersion::new(
                required_text(&row, "storage_backend")?,
                required_text(&row, "storage_key")?,
                optional_entity_tag(&row, "provider_etag")?,
                row.try_get("provider_version").map_err(database_error)?,
                required_entity_tag(&row, "etag")?,
                as_u64(
                    row.try_get::<Option<i64>, _>("size_bytes")
                        .map_err(database_error)?
                        .ok_or_else(|| invariant("object version size is missing"))?,
                )?,
                row.try_get("content_type").map_err(database_error)?,
                row.try_get::<Json<Value>, _>("user_metadata")
                    .map_err(database_error)?
                    .0,
                row_checksum(&row, "checksum_algorithm", "checksum_value")?,
            )
            .map_err(invariant)?,
        ))
    };
    let retention_mode = row
        .try_get::<Option<String>, _>("retention_mode")
        .map_err(database_error)?;
    let retain_until = row
        .try_get::<Option<OffsetDateTime>, _>("retain_until")
        .map_err(database_error)?;
    let retention = match (retention_mode.as_deref(), retain_until) {
        (None, None) => None,
        (Some(mode), Some(until)) => Some(ObjectRetention::new(parse_retention_mode(mode)?, until)),
        _ => return Err(invariant("persisted retention fields are inconsistent")),
    };
    ObjectVersion::from_persistence(PersistedObjectVersion {
        id: ObjectVersionId::from_uuid(row.try_get("id").map_err(database_error)?),
        object_id: ObjectId::from_uuid(row.try_get("object_id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_id: BucketId::from_uuid(row.try_get("bucket_id").map_err(database_error)?),
        external_version_id: S3VersionId::new(
            row.try_get::<String, _>("external_version_id")
                .map_err(database_error)?,
        )
        .map_err(invariant)?,
        generation: as_u64(row.try_get("generation").map_err(database_error)?)?,
        is_null_version: row.try_get("is_null_version").map_err(database_error)?,
        state: parse_object_version_state(
            &row.try_get::<String, _>("state").map_err(database_error)?,
        )?,
        payload,
        retention,
        legal_hold: row.try_get("legal_hold").map_err(database_error)?,
        created_by: row.try_get("created_by").map_err(database_error)?,
        source_protocol: parse_source_protocol(
            &row.try_get::<String, _>("source_protocol")
                .map_err(database_error)?,
        )?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        became_noncurrent_at: row
            .try_get("became_noncurrent_at")
            .map_err(database_error)?,
    })
    .map_err(invariant)
}

fn optional_entity_tag(row: &PgRow, column: &str) -> Result<Option<EntityTag>, RepositoryError> {
    row.try_get::<Option<String>, _>(column)
        .map_err(database_error)?
        .map(EntityTag::new)
        .transpose()
        .map_err(invariant)
}

fn required_entity_tag(row: &PgRow, column: &str) -> Result<EntityTag, RepositoryError> {
    EntityTag::new(required_text(row, column)?).map_err(invariant)
}

fn row_checksum(
    row: &PgRow,
    algorithm_column: &str,
    value_column: &str,
) -> Result<Option<Checksum>, RepositoryError> {
    let algorithm = row
        .try_get::<Option<String>, _>(algorithm_column)
        .map_err(database_error)?;
    let value = row
        .try_get::<Option<String>, _>(value_column)
        .map_err(database_error)?;
    match (algorithm.as_deref(), value) {
        (None, None) => Ok(None),
        (Some("sha256"), Some(value)) => Checksum::sha256_hex(value).map(Some).map_err(invariant),
        _ => Err(invariant("persisted checksum fields are inconsistent")),
    }
}
const fn checksum_algorithm_name(algorithm: ChecksumAlgorithm) -> &'static str {
    match algorithm {
        ChecksumAlgorithm::Sha256 => "sha256",
    }
}

fn required_text(row: &PgRow, column: &str) -> Result<String, RepositoryError> {
    row.try_get::<Option<String>, _>(column)
        .map_err(database_error)?
        .ok_or_else(|| invariant(format!("object version {column} is missing")))
}

const fn versioning_status_name(status: VersioningStatus) -> &'static str {
    match status {
        VersioningStatus::Unversioned => "unversioned",
        VersioningStatus::Enabled => "enabled",
        VersioningStatus::Suspended => "suspended",
    }
}

fn parse_versioning_status(value: &str) -> Result<VersioningStatus, RepositoryError> {
    match value {
        "unversioned" => Ok(VersioningStatus::Unversioned),
        "enabled" => Ok(VersioningStatus::Enabled),
        "suspended" => Ok(VersioningStatus::Suspended),
        _ => Err(invariant("persisted versioning status is invalid")),
    }
}

const fn object_version_state_name(state: ObjectVersionState) -> &'static str {
    match state {
        ObjectVersionState::Staged => "staged",
        ObjectVersionState::Committed => "committed",
        ObjectVersionState::Deleting => "deleting",
        ObjectVersionState::Failed => "failed",
    }
}

fn parse_object_version_state(value: &str) -> Result<ObjectVersionState, RepositoryError> {
    match value {
        "staged" => Ok(ObjectVersionState::Staged),
        "committed" => Ok(ObjectVersionState::Committed),
        "deleting" => Ok(ObjectVersionState::Deleting),
        "failed" => Ok(ObjectVersionState::Failed),
        _ => Err(invariant("persisted object version state is invalid")),
    }
}

const fn source_protocol_name(protocol: SourceProtocol) -> &'static str {
    match protocol {
        SourceProtocol::S3 => "s3",
        SourceProtocol::Dav => "dav",
        SourceProtocol::Json => "json",
        SourceProtocol::Processor => "processor",
    }
}

fn parse_source_protocol(value: &str) -> Result<SourceProtocol, RepositoryError> {
    match value {
        "s3" => Ok(SourceProtocol::S3),
        "dav" => Ok(SourceProtocol::Dav),
        "json" => Ok(SourceProtocol::Json),
        "processor" => Ok(SourceProtocol::Processor),
        _ => Err(invariant("persisted source protocol is invalid")),
    }
}

const fn retention_mode_name(mode: RetentionMode) -> &'static str {
    match mode {
        RetentionMode::Governance => "governance",
        RetentionMode::Compliance => "compliance",
    }
}

fn parse_retention_mode(value: &str) -> Result<RetentionMode, RepositoryError> {
    match value {
        "governance" => Ok(RetentionMode::Governance),
        "compliance" => Ok(RetentionMode::Compliance),
        _ => Err(invariant("persisted retention mode is invalid")),
    }
}

fn invariant(error: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Invariant(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enum_codec_matches_migration_values() {
        for status in [
            VersioningStatus::Unversioned,
            VersioningStatus::Enabled,
            VersioningStatus::Suspended,
        ] {
            assert_eq!(
                parse_versioning_status(versioning_status_name(status)),
                Ok(status)
            );
        }
        assert!(parse_versioning_status("disabled").is_err());
    }

    #[test]
    fn migration_encodes_version_and_latest_invariants() {
        let migration = include_str!("../migrations/0012_s3_object_versions.sql");
        assert!(migration.contains("('unversioned', 'enabled', 'suspended')"));
        assert!(!migration.contains("CONSTRAINT object_versions_external_version_key"));
        assert!(
            migration.contains("CREATE UNIQUE INDEX object_versions_active_external_version_key")
        );
        assert!(migration.contains("ON object_versions(object_id, external_version_id)"));
        assert!(migration.contains("WHERE superseded_at IS NULL"));
        assert!(migration.contains("is_null_version = (external_version_id = 'null')"));
        assert!(migration.contains("superseded_at TIMESTAMPTZ"));
        assert!(!migration.contains("is_latest"));
        assert!(migration.contains("current_version_id"));
        assert!(migration.contains("became_noncurrent_at"));
        assert!(migration.contains("cannot return to unversioned"));
        assert!(migration.contains("Object Lock cannot be disabled"));
    }

    #[test]
    fn repository_switches_head_under_a_row_lock() {
        let source = include_str!("s3_repository.rs");
        assert!(source.contains("SELECT * FROM objects WHERE id = $1 FOR UPDATE"));
        assert!(source.contains("SET became_noncurrent_at = $1"));
        assert!(source.contains("SET current_version_id = $1"));
        assert!(LOCK_ACTIVE_NULL_VERSION_SQL.contains("FOR UPDATE"));
        assert!(SUPERSEDE_ACTIVE_NULL_VERSION_SQL.contains("SET superseded_at = $1"));

        let commit_body = source
            .split("async fn commit_object_version_in_transaction")
            .nth(1)
            .expect("commit transaction body");
        let supersede = commit_body
            .find("supersede_active_null_version(")
            .expect("supersede call");
        assert!(
            commit_body[supersede..]
                .contains("insert_object_version(transaction, &commit.version)"),
            "the append branch must supersede the active null before inserting its replacement"
        );
    }

    #[test]
    fn s3_visibility_and_internal_audit_queries_are_separate() {
        assert!(FIND_ACTIVE_OBJECT_VERSION_SQL.contains("superseded_at IS NULL"));
        assert!(FIND_CURRENT_OBJECT_VERSION_SQL.contains("superseded_at IS NULL"));
        assert!(LIST_ACTIVE_OBJECT_VERSIONS_SQL.contains("superseded_at IS NULL"));
        assert!(!FIND_OBJECT_VERSION_BY_ID_AUDIT_SQL.contains("superseded_at"));
        assert!(FIND_OBJECT_VERSION_BY_ID_AUDIT_SQL.contains("WHERE id = $1"));
    }

    #[test]
    fn delete_transaction_locks_configuration_object_and_versions_before_atomic_gc() {
        let source = include_str!("s3_repository.rs");
        let delete = source
            .split("async fn delete_s3_object_in_transaction")
            .nth(1)
            .expect("delete transaction")
            .split("#[async_trait]\nimpl S3UploadIntentRepository")
            .next()
            .expect("delete transaction boundary");

        assert!(delete.contains("FOR SHARE"));
        assert!(delete.contains("FOR UPDATE"));
        assert!(source.contains("state = 'deleting'"));
        assert!(source.contains("SET current_version_id = $1"));
        assert!(delete.contains("SET became_noncurrent_at = NULL"));
        assert!(delete.contains("supersede_active_null_version"));
        assert!(delete.contains("enqueue_explicit_delete_gc_task"));
        assert!(source.contains("StorageGcReason::ExplicitDelete"));
        assert!(delete.contains("VersioningStatus::Enabled"));
        assert!(delete.contains("VersioningStatus::Suspended"));
        assert!(delete.contains("VersioningStatus::Unversioned"));
        assert!(delete.contains("DeleteS3ObjectOutcome::VersionNotFound"));
        assert!(delete.contains("DeleteS3ObjectOutcome::NoOp"));
        assert!(!delete.contains("INSERT INTO media"));
        assert!(!delete.contains("UPDATE media"));
        assert!(!delete.contains("DELETE FROM media"));
    }

    #[test]
    fn delete_lock_and_gc_helpers_bind_the_exact_version_storage_target() {
        let source = include_str!("s3_repository.rs");
        let lock = source
            .split("fn delete_lock_reason")
            .nth(1)
            .expect("delete lock helper")
            .split("fn explicit_delete_gc_task")
            .next()
            .expect("delete lock boundary");
        assert!(lock.contains("version.legal_hold()"));
        assert!(lock.contains("RetentionMode::Compliance"));
        assert!(lock.contains("RetentionMode::Governance"));
        assert!(lock.contains("command.bypass_governance"));
        assert!(lock.contains("retention.retain_until() <= command.deleted_at"));

        let gc = source
            .split("fn explicit_delete_gc_task")
            .nth(1)
            .expect("explicit delete GC helper")
            .split("async fn mark_current_version_noncurrent")
            .next()
            .expect("explicit delete GC boundary");
        assert!(gc.contains("object_version_id: Some(version.id())"));
        assert!(gc.contains("storage_backend: payload.storage_backend()"));
        assert!(gc.contains("storage_key: payload.storage_key()"));
        assert!(gc.contains("not_before: command.gc_not_before"));
        assert!(gc.contains("StorageGcReason::ExplicitDelete"));

        assert!(FIND_ACTIVE_OBJECT_VERSION_SQL.contains("state = 'committed'"));
        assert!(FIND_CURRENT_OBJECT_VERSION_SQL.contains("state = 'committed'"));
        assert!(LIST_ACTIVE_OBJECT_VERSIONS_SQL.contains("state = 'committed'"));
        assert!(!FIND_OBJECT_VERSION_BY_ID_AUDIT_SQL.contains("state = 'committed'"));
    }

    #[test]
    fn list_objects_sql_reads_only_visible_current_committed_data_heads() {
        let query = S3ObjectListQuery {
            bucket_id: BucketId::new(),
            prefix: "docs/".into(),
            start_after: Some("docs/previous".into()),
            delimiter: true,
            limit: 1_000,
        };
        let sql = build_list_current_s3_objects_query(ApplicationId::new(), &query, 1_001);
        let sql = sql.sql();
        let sql = sql.as_ref();

        assert!(sql.contains("FROM objects AS logical_object"));
        assert!(sql.contains("JOIN object_versions AS version"));
        assert!(sql.contains("version.id = logical_object.current_version_id"));
        assert!(sql.contains("version.object_id = logical_object.id"));
        assert!(sql.contains("version.superseded_at IS NULL"));
        assert!(sql.contains("version.state = 'committed'"));
        assert!(sql.contains("NOT version.is_delete_marker"));
        assert!(sql.contains("LEFT(logical_object.object_key, char_length("));
        assert!(sql.contains("filtered AS"));
        assert!(sql.contains("UNION ALL"));
        assert!(sql.contains("GROUP BY split_part(relative_key, '/', 1)"));
        assert!(sql.contains("WHERE entry_key COLLATE \"C\" >"));
        assert!(sql.contains("ORDER BY entry_key COLLATE \"C\" ASC LIMIT"));
        assert!(
            sql.contains("LEFT JOIN object_versions AS version ON version.id = paged.version_id")
        );
        assert!(!sql.contains(" media "));
    }

    #[test]
    fn list_objects_sql_uses_one_entry_stream_for_delimiter_and_cursor_pages() {
        let recursive = S3ObjectListQuery {
            bucket_id: BucketId::new(),
            prefix: "assets/".into(),
            start_after: None,
            delimiter: false,
            limit: 10,
        };
        let recursive_sql =
            build_list_current_s3_objects_query(ApplicationId::new(), &recursive, 11);
        let recursive_sql = recursive_sql.sql();
        let recursive_sql = recursive_sql.as_ref();
        assert!(recursive_sql.contains("entries AS"));
        assert!(!recursive_sql.contains("filtered AS"));
        assert!(!recursive_sql.contains("UNION ALL"));
        assert!(!recursive_sql.contains("WHERE entry_key COLLATE \"C\" >"));
        assert!(recursive_sql.contains("SELECT entry_key, is_prefix, version_id FROM entries"));

        assert_eq!(
            s3_object_fetch_limit(&S3ObjectListQuery {
                limit: 0,
                ..recursive.clone()
            }),
            Ok(None)
        );
        assert_eq!(
            s3_object_fetch_limit(&S3ObjectListQuery {
                limit: 1_000,
                ..recursive.clone()
            }),
            Ok(Some(1_001))
        );
        assert!(matches!(
            s3_object_fetch_limit(&S3ObjectListQuery {
                limit: 1_001,
                ..recursive
            }),
            Err(RepositoryError::Invariant(_))
        ));
    }

    #[test]
    fn list_object_versions_sql_is_bucket_scoped_stable_and_media_free() {
        let query = S3ObjectVersionListQuery {
            bucket_id: BucketId::new(),
            prefix: "photos/".into(),
            key_marker: Some("photos/a.jpg".into()),
            version_id_marker: Some(S3VersionId::new("version-2").expect("version marker")),
            delimiter: true,
            limit: 1_000,
        };
        let sql = build_list_s3_object_versions_query(ApplicationId::new(), &query, Some(2), 1_001);
        let sql = sql.sql();
        let sql = sql.as_ref();

        assert!(sql.contains("FROM objects AS logical_object"));
        assert!(sql.contains("JOIN object_versions AS version"));
        assert!(sql.contains("version.superseded_at IS NULL"));
        assert!(sql.contains("version.state = 'committed'"));
        assert!(sql.contains("logical_object.current_version_id = version.id AS is_latest"));
        assert!(sql.contains("GROUP BY split_part(relative_key, '/', 1)"));
        assert!(sql.contains("entry_key COLLATE \"C\" >"));
        assert!(sql.contains("entry_key COLLATE \"C\" ="));
        assert!(sql.contains("generation <"));
        assert!(sql.contains("generation DESC NULLS LAST"));
        assert!(sql.contains("LIMIT"));
        assert!(!sql.contains(" media "));
        assert!(FIND_LIST_VERSION_MARKER_GENERATION_SQL.contains("external_version_id = $4"));
        assert!(FIND_LIST_VERSION_MARKER_GENERATION_SQL.contains("superseded_at IS NULL"));
    }

    #[test]
    fn list_object_versions_limit_and_key_only_marker_are_explicit() {
        let query = S3ObjectVersionListQuery {
            bucket_id: BucketId::new(),
            prefix: String::new(),
            key_marker: Some("before".into()),
            version_id_marker: None,
            delimiter: false,
            limit: 10,
        };
        let sql = build_list_s3_object_versions_query(ApplicationId::new(), &query, None, 11);
        let sql = sql.sql();
        let sql = sql.as_ref();
        assert!(sql.contains("WHERE entry_key COLLATE \"C\" >"));
        assert!(!sql.contains("generation <"));
        assert!(!sql.contains("filtered AS"));
        assert_eq!(
            s3_version_fetch_limit(&S3ObjectVersionListQuery {
                limit: 0,
                ..query.clone()
            }),
            Ok(None)
        );
        assert_eq!(
            s3_version_fetch_limit(&S3ObjectVersionListQuery {
                limit: 1_000,
                ..query.clone()
            }),
            Ok(Some(1_001))
        );
        assert!(matches!(
            s3_version_fetch_limit(&S3ObjectVersionListQuery {
                limit: 1_001,
                ..query
            }),
            Err(RepositoryError::Invariant(_))
        ));
    }

    #[test]
    fn list_multipart_uploads_sql_uses_active_database_state_and_byte_order() {
        let query = S3MultipartUploadListQuery {
            bucket_id: BucketId::new(),
            prefix: "uploads/".into(),
            key_marker: Some("uploads/movie.mp4".into()),
            upload_id_marker: Some("upload-002".into()),
            delimiter: true,
            limit: 1_000,
            as_of: OffsetDateTime::UNIX_EPOCH,
        };
        let sql = build_list_s3_multipart_uploads_query(ApplicationId::new(), &query, 1_001);
        let sql = sql.sql();
        let sql = sql.as_ref();

        assert!(sql.contains("FROM s3_multipart_uploads"));
        assert!(sql.contains("state IN ('pending', 'completing')"));
        assert!(sql.contains("expires_at >"));
        assert!(sql.contains("GROUP BY split_part(relative_key, '/', 1)"));
        assert!(sql.contains("entry_key COLLATE \"C\" >"));
        assert!(sql.contains("upload_id COLLATE \"C\" >"));
        assert!(sql.contains("upload_id COLLATE \"C\" ASC NULLS LAST"));
        assert!(sql.contains("LIMIT"));
        for forbidden in ["object_store", "storage_key", " media "] {
            assert!(
                !sql.contains(forbidden),
                "forbidden listing dependency: {forbidden}"
            );
        }
        assert_eq!(
            s3_multipart_upload_fetch_limit(&S3MultipartUploadListQuery {
                limit: 0,
                ..query.clone()
            }),
            Ok(None)
        );
        assert_eq!(s3_multipart_upload_fetch_limit(&query), Ok(Some(1_001)));
    }

    #[test]
    fn upload_id_marker_without_key_marker_does_not_change_sql_cursor() {
        let query = S3MultipartUploadListQuery {
            bucket_id: BucketId::new(),
            prefix: String::new(),
            key_marker: None,
            upload_id_marker: Some("ignored-by-s3".into()),
            delimiter: false,
            limit: 10,
            as_of: OffsetDateTime::UNIX_EPOCH,
        };
        let sql = build_list_s3_multipart_uploads_query(ApplicationId::new(), &query, 11);
        let sql = sql.sql();
        let sql = sql.as_ref();
        assert!(!sql.contains("WHERE entry_key COLLATE"));
        assert!(!sql.contains("upload_id COLLATE \"C\" >"));
    }

    #[test]
    fn upload_intent_sql_matches_staging_contract() {
        let migration = include_str!("../migrations/0012_s3_object_versions.sql");
        assert!(migration.contains("proposed_version_id UUID NOT NULL UNIQUE"));
        assert!(migration.contains("temporary_storage_key TEXT NOT NULL CHECK"));
        assert!(migration.contains("final_storage_key TEXT NOT NULL CHECK"));
        assert!(migration.contains("expected_size_bytes BIGINT NOT NULL"));
        assert!(migration.contains(
            "state IN ('staging', 'ready', 'committing', 'committed', 'aborted', 'expired')"
        ));
        assert!(CLAIM_UPLOAD_INTENT_SQL.contains("WHERE id = $1 AND expires_at > $4"));
        assert!(CLAIM_UPLOAD_INTENT_SQL.contains("state = 'ready'"));
        assert!(CLAIM_UPLOAD_INTENT_SQL.contains("state = 'committing' AND lease_until <= $4"));
        assert!(CLAIM_UPLOAD_INTENT_SQL.contains("FOR UPDATE SKIP LOCKED"));
    }

    #[test]
    fn multipart_takeover_preserves_the_attached_upload_intent() {
        let lifecycle = include_str!("multipart_lifecycle.rs");
        let claim = lifecycle
            .split("async fn claim_multipart_completion")
            .nth(1)
            .expect("claim implementation")
            .split("async fn attach_multipart_upload_intent")
            .next()
            .expect("claim implementation boundary");
        assert!(!claim.contains("abort_attached_intent"));
        assert!(!claim.contains("upload_intent_id = NULL"));
        assert!(claim.contains("completion_token = $1"));
    }
    #[test]
    fn storage_gc_claim_and_cleanup_are_fenced_and_idempotent() {
        let source = include_str!("s3_repository.rs");
        assert!(CLAIM_STORAGE_GC_TASKS_SQL.contains("FOR UPDATE SKIP LOCKED"));
        assert!(CLAIM_STORAGE_GC_TASKS_SQL.contains("attempts < max_attempts"));
        assert!(source.contains("state = 'dead_letter'"));
        assert!(source.contains("lease_token = $2"));
        assert!(source.contains("ON CONFLICT (storage_backend, storage_key) DO NOTHING"));
        assert!(source.contains("ensure_persisted_null_replacement_gc_task"));
        assert!(source.contains("WHERE storage_backend = $1 AND storage_key = $2"));
        assert!(source.contains("intent.temporary_storage_key()"));
        assert!(source.contains("intent.final_storage_key()"));
    }

    #[test]
    fn new_s3_commit_path_never_writes_media() {
        let source = include_str!("s3_repository.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("implementation section");
        assert!(!implementation.contains("INSERT INTO media"));
        assert!(!implementation.contains("UPDATE media"));
        assert!(!implementation.contains("DELETE FROM media"));
        assert!(implementation.contains("object_id = $1, object_version_id = $2"));
    }

    #[test]
    fn upload_and_gc_enum_codecs_match_migration_values() {
        for state in [
            "staging",
            "ready",
            "committing",
            "committed",
            "aborted",
            "expired",
        ] {
            assert!(parse_upload_intent_state(state).is_ok());
        }
        for reason in [
            StorageGcReason::AbortedUploadIntent,
            StorageGcReason::MultipartTemporary,
            StorageGcReason::ReplacedNullVersion,
            StorageGcReason::LifecycleExpiration,
            StorageGcReason::ExplicitDelete,
        ] {
            assert_eq!(
                parse_storage_gc_reason(storage_gc_reason_name(reason)),
                Ok(reason)
            );
        }
    }

    fn test_stored_version(
        id: ObjectVersionId,
        object_id: ObjectId,
        application_id: ApplicationId,
        bucket_id: BucketId,
        generation: u64,
        storage_key: &str,
    ) -> ObjectVersion {
        ObjectVersion::new_object(
            id,
            object_id,
            application_id,
            bucket_id,
            S3VersionId::new("null").expect("null version"),
            generation,
            true,
            ObjectVersionState::Committed,
            StoredObjectVersion::new(
                "filesystem",
                storage_key,
                None,
                None,
                EntityTag::new(format!("etag-{generation}")).expect("etag"),
                4,
                None,
                serde_json::json!({}),
                Some(Checksum::sha256_hex("0".repeat(64)).expect("checksum")),
            )
            .expect("stored payload"),
            None,
            false,
            "test",
            SourceProtocol::S3,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("object version")
    }

    fn replacement_commit(
        previous: &ObjectVersion,
        new_version: ObjectVersion,
    ) -> S3ObjectVersionCommit {
        let ObjectVersionPayload::Object(payload) = previous.payload() else {
            panic!("fixture must contain data");
        };
        S3ObjectVersionCommit {
            target: S3ObjectCommitTarget::Append {
                object_id: previous.object_id(),
                expected_generation: new_version.generation() - 1,
            },
            version: new_version,
            committed_at: OffsetDateTime::UNIX_EPOCH,
            replaced_null_version_id: Some(previous.id()),
            gc_tasks: vec![NewStorageGcTask {
                id: StorageGcTaskId::new(),
                application_id: previous.application_id(),
                bucket_id: previous.bucket_id(),
                object_version_id: Some(previous.id()),
                upload_intent_id: None,
                multipart_upload_id: None,
                storage_backend: payload.storage_backend().into(),
                storage_key: payload.storage_key().into(),
                reason: StorageGcReason::ReplacedNullVersion,
                not_before: OffsetDateTime::UNIX_EPOCH,
                max_attempts: 3,
                created_at: OffsetDateTime::UNIX_EPOCH,
            }],
            requested_retention: None,
            requested_legal_hold: None,
        }
    }

    #[test]
    fn null_data_replacement_requires_one_exact_gc_task() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let object_id = ObjectId::new();
        let previous = test_stored_version(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            1,
            "objects/old-null",
        );
        let next = test_stored_version(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            2,
            "objects/new-null",
        );
        let commit = replacement_commit(&previous, next);

        assert_eq!(
            validate_null_version_replacement(&commit, Some(&previous)),
            Ok(())
        );

        let mut wrong_key = commit.clone();
        wrong_key.gc_tasks[0].storage_key = "objects/not-the-old-null".into();
        assert_eq!(
            validate_null_version_replacement(&wrong_key, Some(&previous)),
            Err(RepositoryError::Conflict)
        );

        let mut duplicate = commit;
        duplicate.gc_tasks.push(duplicate.gc_tasks[0].clone());
        assert_eq!(
            validate_null_version_replacement(&duplicate, Some(&previous)),
            Err(RepositoryError::Conflict)
        );
    }

    #[test]
    fn stale_null_replacement_id_conflicts() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let object_id = ObjectId::new();
        let previous = test_stored_version(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            1,
            "objects/old-null",
        );
        let next = test_stored_version(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            2,
            "objects/new-null",
        );
        let mut commit = replacement_commit(&previous, next);
        commit.replaced_null_version_id = Some(ObjectVersionId::new());

        assert_eq!(
            validate_null_version_replacement(&commit, Some(&previous)),
            Err(RepositoryError::Conflict)
        );
    }

    #[test]
    fn replacing_a_null_delete_marker_needs_no_blob_gc_task() {
        let application_id = ApplicationId::new();
        let bucket_id = BucketId::new();
        let object_id = ObjectId::new();
        let previous = ObjectVersion::new_delete_marker(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            S3VersionId::new("null").expect("null version"),
            1,
            true,
            "test",
            SourceProtocol::S3,
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("delete marker");
        let next = test_stored_version(
            ObjectVersionId::new(),
            object_id,
            application_id,
            bucket_id,
            2,
            "objects/new-null",
        );
        let mut commit = S3ObjectVersionCommit {
            target: S3ObjectCommitTarget::Append {
                object_id,
                expected_generation: 1,
            },
            version: next,
            committed_at: OffsetDateTime::UNIX_EPOCH,
            replaced_null_version_id: Some(previous.id()),
            gc_tasks: Vec::new(),
            requested_retention: None,
            requested_legal_hold: None,
        };

        assert_eq!(
            validate_null_version_replacement(&commit, Some(&previous)),
            Ok(())
        );

        commit.gc_tasks.push(NewStorageGcTask {
            id: StorageGcTaskId::new(),
            application_id,
            bucket_id,
            object_version_id: Some(previous.id()),
            upload_intent_id: None,
            multipart_upload_id: None,
            storage_backend: "filesystem".into(),
            storage_key: "objects/marker-has-no-blob".into(),
            reason: StorageGcReason::ReplacedNullVersion,
            not_before: OffsetDateTime::UNIX_EPOCH,
            max_attempts: 3,
            created_at: OffsetDateTime::UNIX_EPOCH,
        });
        assert_eq!(
            validate_null_version_replacement(&commit, Some(&previous)),
            Err(RepositoryError::Conflict)
        );
    }

    #[test]
    fn migration_keeps_object_tags_version_scoped_and_out_of_user_metadata() {
        let migration = include_str!("../migrations/0012_s3_object_versions.sql");
        assert!(migration.contains("CREATE TABLE object_version_tags"));
        assert!(migration.contains("UNIQUE (object_version_id, tag_key)"));
        assert!(migration.contains("CHECK (position BETWEEN 0 AND 9)"));
        assert!(migration.contains("REFERENCES object_versions (id, is_delete_marker)"));
        assert!(migration.contains("CHECK (NOT is_delete_marker)"));
        assert!(migration.contains("object_tags JSONB NOT NULL DEFAULT '[]'::jsonb"));
        assert!(!migration.contains("user_metadata -> 'tags'"));
    }
}
