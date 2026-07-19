use mediahub_app::{PendingMediaDeletion, QuotaSnapshot, RepositoryError};
use mediahub_core::{ApplicationId, MediaId};
use sqlx::Row;

use crate::{
    PostgresRepository,
    codec::{as_u64, database_error},
};

impl PostgresRepository {
    pub async fn health_check(&self) -> Result<(), RepositoryError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(database_error)?;
        Ok(())
    }

    pub async fn storage_consistency_sample(
        &self,
        limit: usize,
    ) -> Result<(u64, Vec<String>), RepositoryError> {
        if limit == 0 {
            return Err(RepositoryError::Invariant(
                "storage consistency sample limit must be positive".into(),
            ));
        }
        let limit = i64::try_from(limit).map_err(|_| {
            RepositoryError::Invariant("storage consistency sample limit is too large".into())
        })?;
        let used_bytes = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(SUM(used_bytes), 0)::BIGINT FROM applications",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(database_error)?;
        let storage_keys = sqlx::query_scalar::<_, String>(
            "SELECT storage_key FROM media \
             WHERE state <> 'deleted' AND size_bytes > 0 \
             ORDER BY CASE state \
                 WHEN 'active' THEN 0 \
                 WHEN 'archived' THEN 1 \
                 WHEN 'quarantined' THEN 2 \
                 WHEN 'archive_pending' THEN 3 \
                 WHEN 'uploading' THEN 4 \
                 ELSE 5 END, updated_at DESC, id DESC \
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        Ok((as_u64(used_bytes)?, storage_keys))
    }

    pub async fn list_pending_deletions(
        &self,
        limit: usize,
    ) -> Result<Vec<PendingMediaDeletion>, RepositoryError> {
        let limit = i64::try_from(limit).map_err(|_| {
            RepositoryError::Invariant("pending deletion limit is too large".into())
        })?;
        let rows = sqlx::query(
            "SELECT id, storage_key FROM media WHERE state = 'delete_pending' \
             ORDER BY updated_at, id LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter()
            .map(|row| {
                Ok(PendingMediaDeletion {
                    media_id: MediaId::from_uuid(row.try_get("id").map_err(database_error)?),
                    storage_key: row.try_get("storage_key").map_err(database_error)?,
                })
            })
            .collect()
    }

    pub async fn quota(
        &self,
        application_id: ApplicationId,
    ) -> Result<QuotaSnapshot, RepositoryError> {
        let row = sqlx::query(
            "SELECT quota_bytes, used_bytes, reserved_bytes FROM applications WHERE id = $1",
        )
        .bind(application_id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
        Ok(QuotaSnapshot {
            quota_bytes: as_u64(row.try_get("quota_bytes").map_err(database_error)?)?,
            used_bytes: as_u64(row.try_get("used_bytes").map_err(database_error)?)?,
            reserved_bytes: as_u64(row.try_get("reserved_bytes").map_err(database_error)?)?,
        })
    }
}
