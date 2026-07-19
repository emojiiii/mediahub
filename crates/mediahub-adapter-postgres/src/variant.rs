use async_trait::async_trait;
use mediahub_app::{
    NewVariant, RepositoryError, VariantClaim, VariantRecord, VariantRepository, VariantState,
};
use mediahub_core::{MediaId, OffsetDateTime, VariantFormat, VariantId};
use sqlx::{Row, postgres::PgRow};
use uuid::Uuid;

use crate::{
    PostgresRepository,
    codec::{as_i64, as_u64, database_error},
};

#[async_trait]
impl VariantRepository for PostgresRepository {
    async fn claim_variant(
        &self,
        variant: NewVariant,
        lease_token: &str,
        leased_until: OffsetDateTime,
    ) -> Result<VariantClaim, RepositoryError> {
        let lease_token = parse_token(lease_token)?;
        if leased_until <= variant.created_at {
            return Err(RepositoryError::Invariant(
                "variant lease must end after creation".into(),
            ));
        }
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let inserted = sqlx::query(
            "INSERT INTO variants (id, media_id, transform_key, parameters_json, \
             processor_version, format, storage_backend, storage_key, status, generation_token, \
             generation_lease_until, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'generating', $9, $10, $11, $11) \
             ON CONFLICT (media_id, transform_key) DO NOTHING",
        )
        .bind(variant.id.as_uuid())
        .bind(variant.media_id.as_uuid())
        .bind(&variant.transform_key)
        .bind(&variant.parameters_json)
        .bind(&variant.processor_version)
        .bind(format_name(variant.format))
        .bind(&variant.storage_backend)
        .bind(&variant.storage_key)
        .bind(lease_token)
        .bind(leased_until)
        .bind(variant.created_at)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if inserted.rows_affected() == 1 {
            let record = find_variant(&mut transaction, variant.id).await?;
            transaction.commit().await.map_err(database_error)?;
            return Ok(VariantClaim::Generate {
                variant: record,
                lease_token: lease_token.to_string(),
            });
        }

        let row = sqlx::query(
            "SELECT * FROM variants WHERE media_id = $1 AND transform_key = $2 FOR UPDATE",
        )
        .bind(variant.media_id.as_uuid())
        .bind(&variant.transform_key)
        .fetch_one(&mut *transaction)
        .await
        .map_err(database_error)?;
        let existing = row_to_variant(row)?;
        if existing.state == VariantState::Ready {
            transaction.commit().await.map_err(database_error)?;
            return Ok(VariantClaim::Ready(existing));
        }
        let reclaimed = sqlx::query(
            "UPDATE variants SET status = 'generating', generation_token = $1, \
             generation_lease_until = $2, last_error = NULL, updated_at = CURRENT_TIMESTAMP \
             WHERE id = $3 AND (status = 'failed' OR \
                 (status = 'generating' AND generation_lease_until <= CURRENT_TIMESTAMP))",
        )
        .bind(lease_token)
        .bind(leased_until)
        .bind(existing.id.as_uuid())
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if reclaimed.rows_affected() == 0 {
            transaction.commit().await.map_err(database_error)?;
            return Ok(VariantClaim::InProgress);
        }
        let record = find_variant(&mut transaction, existing.id).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(VariantClaim::Generate {
            variant: record,
            lease_token: lease_token.to_string(),
        })
    }

    async fn complete_variant(
        &self,
        variant_id: VariantId,
        lease_token: &str,
        width: u32,
        height: u32,
        size: u64,
        completed_at: OffsetDateTime,
    ) -> Result<Option<VariantRecord>, RepositoryError> {
        if width == 0 || height == 0 {
            return Err(RepositoryError::Invariant(
                "variant dimensions must be positive".into(),
            ));
        }
        let lease_token = parse_token(lease_token)?;
        let row = sqlx::query(
            "UPDATE variants SET status = 'ready', width = $1, height = $2, size_bytes = $3, \
             generation_token = NULL, generation_lease_until = NULL, last_error = NULL, \
             last_accessed_at = $4, updated_at = $4 \
             WHERE id = $5 AND status = 'generating' AND generation_token = $6 \
               AND generation_lease_until > $4 RETURNING *",
        )
        .bind(i32::try_from(width).map_err(|_| {
            RepositoryError::Invariant("variant width exceeds PostgreSQL INTEGER".into())
        })?)
        .bind(i32::try_from(height).map_err(|_| {
            RepositoryError::Invariant("variant height exceeds PostgreSQL INTEGER".into())
        })?)
        .bind(as_i64(size)?)
        .bind(completed_at)
        .bind(variant_id.as_uuid())
        .bind(lease_token)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        row.map(row_to_variant).transpose()
    }

    async fn fail_variant(
        &self,
        variant_id: VariantId,
        lease_token: &str,
        error_summary: &str,
        failed_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        if error_summary.is_empty() {
            return Err(RepositoryError::Invariant(
                "variant error summary cannot be empty".into(),
            ));
        }
        let lease_token = parse_token(lease_token)?;
        let result = sqlx::query(
            "UPDATE variants SET status = 'failed', generation_token = NULL, \
             generation_lease_until = NULL, last_error = $1, updated_at = $2 \
             WHERE id = $3 AND status = 'generating' AND generation_token = $4 \
               AND generation_lease_until > $2",
        )
        .bind(error_summary)
        .bind(failed_at)
        .bind(variant_id.as_uuid())
        .bind(lease_token)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 1 {
            Ok(())
        } else {
            Err(RepositoryError::Conflict)
        }
    }
}

async fn find_variant(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    variant_id: VariantId,
) -> Result<VariantRecord, RepositoryError> {
    let row = sqlx::query("SELECT * FROM variants WHERE id = $1")
        .bind(variant_id.as_uuid())
        .fetch_one(&mut **transaction)
        .await
        .map_err(database_error)?;
    row_to_variant(row)
}

fn row_to_variant(row: PgRow) -> Result<VariantRecord, RepositoryError> {
    Ok(VariantRecord {
        id: VariantId::from_uuid(row.try_get("id").map_err(database_error)?),
        media_id: MediaId::from_uuid(row.try_get("media_id").map_err(database_error)?),
        transform_key: row.try_get("transform_key").map_err(database_error)?,
        parameters_json: row.try_get("parameters_json").map_err(database_error)?,
        processor_version: row.try_get("processor_version").map_err(database_error)?,
        format: parse_format(&row.try_get::<String, _>("format").map_err(database_error)?)?,
        width: row
            .try_get::<Option<i32>, _>("width")
            .map_err(database_error)?
            .map(|value| {
                u32::try_from(value)
                    .map_err(|_| RepositoryError::Invariant("variant width is invalid".into()))
            })
            .transpose()?,
        height: row
            .try_get::<Option<i32>, _>("height")
            .map_err(database_error)?
            .map(|value| {
                u32::try_from(value)
                    .map_err(|_| RepositoryError::Invariant("variant height is invalid".into()))
            })
            .transpose()?,
        size: row
            .try_get::<Option<i64>, _>("size_bytes")
            .map_err(database_error)?
            .map(as_u64)
            .transpose()?,
        storage_backend: row.try_get("storage_backend").map_err(database_error)?,
        storage_key: row.try_get("storage_key").map_err(database_error)?,
        state: parse_state(&row.try_get::<String, _>("status").map_err(database_error)?)?,
        last_error: row.try_get("last_error").map_err(database_error)?,
        last_accessed_at: row.try_get("last_accessed_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

const fn format_name(format: VariantFormat) -> &'static str {
    match format {
        VariantFormat::Jpeg => "jpeg",
        VariantFormat::Png => "png",
        VariantFormat::Webp => "webp",
    }
}

fn parse_format(value: &str) -> Result<VariantFormat, RepositoryError> {
    match value {
        "jpeg" => Ok(VariantFormat::Jpeg),
        "png" => Ok(VariantFormat::Png),
        "webp" => Ok(VariantFormat::Webp),
        _ => Err(RepositoryError::Invariant(
            "persisted variant format is invalid".into(),
        )),
    }
}

fn parse_state(value: &str) -> Result<VariantState, RepositoryError> {
    match value {
        "generating" => Ok(VariantState::Generating),
        "ready" => Ok(VariantState::Ready),
        "failed" => Ok(VariantState::Failed),
        "delete_pending" => Ok(VariantState::DeletePending),
        _ => Err(RepositoryError::Invariant(
            "persisted variant state is invalid".into(),
        )),
    }
}

fn parse_token(value: &str) -> Result<Uuid, RepositoryError> {
    Uuid::parse_str(value)
        .map_err(|_| RepositoryError::Invariant("variant lease token is not a UUID".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_variant_formats_exclude_avif() {
        assert_eq!(format_name(VariantFormat::Jpeg), "jpeg");
        assert_eq!(format_name(VariantFormat::Png), "png");
        assert_eq!(format_name(VariantFormat::Webp), "webp");
        assert!(matches!(
            parse_format("avif"),
            Err(RepositoryError::Invariant(_))
        ));
    }
}
