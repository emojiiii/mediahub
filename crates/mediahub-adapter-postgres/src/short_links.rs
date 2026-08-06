use mediahub_app::RepositoryError;
use mediahub_core::{ApplicationId, MediaId, OffsetDateTime};
use sqlx::Row;

use crate::{PostgresRepository, codec::database_error};

/// Persisted public redirect metadata. The target path never contains a
/// capability token; object visibility and expiry are checked again on lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortLinkRecord {
    pub code: String,
    pub target_path: String,
    pub expires_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
}

impl PostgresRepository {
    pub async fn create_short_link(
        &self,
        code: &str,
        application_id: ApplicationId,
        media_id: MediaId,
        target_path: &str,
        expires_at: Option<OffsetDateTime>,
        created_at: OffsetDateTime,
    ) -> Result<ShortLinkRecord, RepositoryError> {
        let row = sqlx::query(
            "INSERT INTO short_links \
             (code, application_id, media_id, target_path, expires_at, created_at) \
             VALUES ($1, $2, $3, $4, $5, $6) \
             ON CONFLICT (application_id, media_id) DO UPDATE SET \
                 target_path = EXCLUDED.target_path, expires_at = EXCLUDED.expires_at \
             RETURNING code, target_path, expires_at, created_at",
        )
        .bind(code)
        .bind(application_id.as_uuid())
        .bind(media_id.as_uuid())
        .bind(target_path)
        .bind(expires_at)
        .bind(created_at)
        .fetch_one(self.pool())
        .await
        .map_err(database_error)?;
        Ok(ShortLinkRecord {
            code: row.try_get("code").map_err(database_error)?,
            target_path: row.try_get("target_path").map_err(database_error)?,
            expires_at: row.try_get("expires_at").map_err(database_error)?,
            created_at: row.try_get("created_at").map_err(database_error)?,
        })
    }

    /// Finds a live short link and only returns it while its object remains a
    /// readable public object. This makes deletion, object expiry, and a later
    /// visibility change invalidate the link without a cleanup race.
    pub async fn find_public_short_link(
        &self,
        code: &str,
        now: OffsetDateTime,
    ) -> Result<Option<ShortLinkRecord>, RepositoryError> {
        let row = sqlx::query(
            "SELECT sl.code, sl.target_path, sl.expires_at, sl.created_at \
             FROM short_links sl \
             JOIN media m ON m.id = sl.media_id \
             JOIN buckets b ON b.id = m.bucket_id \
             WHERE sl.code = $1 \
               AND (sl.expires_at IS NULL OR sl.expires_at > $2) \
               AND m.application_id = sl.application_id \
               AND m.state = 'active' \
               AND (m.expires_at IS NULL OR m.expires_at > $2) \
               AND COALESCE(m.visibility_override, b.visibility) = 'public'",
        )
        .bind(code)
        .bind(now)
        .fetch_optional(self.pool())
        .await
        .map_err(database_error)?;
        row.map(|row| {
            Ok(ShortLinkRecord {
                code: row.try_get("code").map_err(database_error)?,
                target_path: row.try_get("target_path").map_err(database_error)?,
                expires_at: row.try_get("expires_at").map_err(database_error)?,
                created_at: row.try_get("created_at").map_err(database_error)?,
            })
        })
        .transpose()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn short_link_migration_uses_cascades_and_expiry_index() {
        let migration = include_str!("../migrations/0011_short_links.sql");
        assert!(migration.contains("CREATE TABLE short_links"));
        assert!(migration.contains("REFERENCES applications(id) ON DELETE CASCADE"));
        assert!(migration.contains("REFERENCES media(id) ON DELETE CASCADE"));
        assert!(migration.contains("short_links_expiry_idx"));
    }
}
