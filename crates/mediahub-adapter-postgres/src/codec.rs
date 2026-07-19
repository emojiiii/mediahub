use mediahub_app::{OutboxEvent, RepositoryError};
use mediahub_core::{
    ApplicationId, Bucket, BucketId, BucketPolicy, ClientMetadata, LifecycleRule, Media, MediaId,
    MediaState, PersistedMedia, PersistedSystemMetadata, PersistedUploadSession, UploadSession,
    UploadSessionId, UploadSessionState, Visibility,
};
use serde_json::{Map, Value};
use sqlx::{Row, postgres::PgRow, types::Json};

use mediahub_core::OffsetDateTime;

pub(crate) fn as_i64(value: u64) -> Result<i64, RepositoryError> {
    i64::try_from(value)
        .map_err(|_| RepositoryError::Invariant("numeric value exceeds PostgreSQL BIGINT".into()))
}

pub(crate) fn as_u64(value: i64) -> Result<u64, RepositoryError> {
    u64::try_from(value)
        .map_err(|_| RepositoryError::Invariant("persisted numeric value is negative".into()))
}

pub(crate) fn as_u32(value: i32) -> Result<u32, RepositoryError> {
    u32::try_from(value)
        .map_err(|_| RepositoryError::Invariant("persisted numeric value is negative".into()))
}

pub(crate) fn postgres_time(value: OffsetDateTime) -> OffsetDateTime {
    let micros = value.nanosecond() / 1_000;
    value
        .replace_nanosecond(micros * 1_000)
        .expect("truncated nanoseconds remain valid")
}

pub(crate) const fn visibility_name(value: Visibility) -> &'static str {
    match value {
        Visibility::Public => "public",
        Visibility::Private => "private",
    }
}

pub(crate) fn parse_visibility(value: &str) -> Result<Visibility, RepositoryError> {
    match value {
        "public" => Ok(Visibility::Public),
        "private" => Ok(Visibility::Private),
        _ => Err(RepositoryError::Invariant(
            "persisted visibility is invalid".into(),
        )),
    }
}

pub(crate) const fn media_state_name(value: MediaState) -> &'static str {
    match value {
        MediaState::Uploading => "uploading",
        MediaState::Active => "active",
        MediaState::ArchivePending => "archive_pending",
        MediaState::Archived => "archived",
        MediaState::DeletePending => "delete_pending",
        MediaState::Deleted => "deleted",
        MediaState::Quarantined => "quarantined",
    }
}

fn parse_media_state(value: &str) -> Result<MediaState, RepositoryError> {
    match value {
        "uploading" => Ok(MediaState::Uploading),
        "active" => Ok(MediaState::Active),
        "archive_pending" => Ok(MediaState::ArchivePending),
        "archived" => Ok(MediaState::Archived),
        "delete_pending" => Ok(MediaState::DeletePending),
        "deleted" => Ok(MediaState::Deleted),
        "quarantined" => Ok(MediaState::Quarantined),
        _ => Err(RepositoryError::Invariant(
            "persisted media state is invalid".into(),
        )),
    }
}

pub(crate) const fn upload_state_name(value: UploadSessionState) -> &'static str {
    match value {
        UploadSessionState::Pending => "pending",
        UploadSessionState::Completed => "completed",
        UploadSessionState::Cancelled => "cancelled",
        UploadSessionState::Expired => "expired",
    }
}

fn parse_upload_state(value: &str) -> Result<UploadSessionState, RepositoryError> {
    match value {
        "pending" => Ok(UploadSessionState::Pending),
        "completed" => Ok(UploadSessionState::Completed),
        "cancelled" => Ok(UploadSessionState::Cancelled),
        "expired" => Ok(UploadSessionState::Expired),
        _ => Err(RepositoryError::Invariant(
            "persisted upload session state is invalid".into(),
        )),
    }
}

pub(crate) fn row_to_bucket(row: PgRow) -> Result<Bucket, RepositoryError> {
    let allowed = row
        .try_get::<Json<Vec<String>>, _>("allowed_mime_types")
        .map_err(database_error)?
        .0;
    let lifecycle_rules = row
        .try_get::<Option<Json<Vec<LifecycleRule>>>, _>("lifecycle_policy")
        .map_err(database_error)?
        .map(|value| value.0)
        .unwrap_or_default();
    let policy = BucketPolicy::new(
        parse_visibility(
            &row.try_get::<String, _>("visibility")
                .map_err(database_error)?,
        )?,
        row.try_get::<Option<i64>, _>("default_ttl_seconds")
            .map_err(database_error)?
            .map(as_u64)
            .transpose()?,
        row.try_get::<Option<i64>, _>("max_object_bytes")
            .map_err(database_error)?
            .map(as_u64)
            .transpose()?,
        allowed,
    )
    .and_then(|policy| policy.with_lifecycle_rules(lifecycle_rules))
    .map_err(invariant)?;
    let created_at = row.try_get("created_at").map_err(database_error)?;
    let mut bucket = Bucket::new(
        BucketId::from_uuid(row.try_get("id").map_err(database_error)?),
        ApplicationId::from_uuid(row.try_get("application_id").map_err(database_error)?),
        row.try_get::<String, _>("name").map_err(database_error)?,
        policy.clone(),
        created_at,
    )
    .map_err(invariant)?;
    bucket.update_policy(policy, row.try_get("updated_at").map_err(database_error)?);
    Ok(bucket)
}

pub(crate) fn row_to_media(row: PgRow) -> Result<Media, RepositoryError> {
    let user = json_object(row.try_get("user_metadata").map_err(database_error)?)?;
    let ai = json_object(row.try_get("ai_metadata").map_err(database_error)?)?;
    let metadata = ClientMetadata::new(user, ai).map_err(invariant)?;
    Media::from_persistence(PersistedMedia {
        id: MediaId::from_uuid(row.try_get("id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_id: BucketId::from_uuid(row.try_get("bucket_id").map_err(database_error)?),
        object_key: row.try_get("object_key").map_err(database_error)?,
        original_name: row.try_get("original_name").map_err(database_error)?,
        display_name: row.try_get("display_name").map_err(database_error)?,
        extension: row.try_get("extension").map_err(database_error)?,
        storage_backend: row.try_get("storage_backend").map_err(database_error)?,
        storage_key: row.try_get("storage_key").map_err(database_error)?,
        state: parse_media_state(&row.try_get::<String, _>("state").map_err(database_error)?)?,
        visibility_override: row
            .try_get::<Option<String>, _>("visibility_override")
            .map_err(database_error)?
            .map(|value| parse_visibility(&value))
            .transpose()?,
        system_metadata: PersistedSystemMetadata {
            mime: row.try_get("content_type").map_err(database_error)?,
            size: as_u64(row.try_get("size_bytes").map_err(database_error)?)?,
            width: row
                .try_get::<Option<i32>, _>("width")
                .map_err(database_error)?
                .map(as_u32)
                .transpose()?,
            height: row
                .try_get::<Option<i32>, _>("height")
                .map_err(database_error)?
                .map(as_u32)
                .transpose()?,
            duration_ms: row
                .try_get::<Option<i64>, _>("duration_ms")
                .map_err(database_error)?
                .map(as_u64)
                .transpose()?,
            sha256: row.try_get("sha256").map_err(database_error)?,
        },
        client_metadata: metadata,
        metadata_version: as_u32(row.try_get("metadata_version").map_err(database_error)?)?,
        revision: as_u64(row.try_get("revision").map_err(database_error)?)?,
        expire_at: row.try_get("expires_at").map_err(database_error)?,
        archived_at: row.try_get("archived_at").map_err(database_error)?,
        deleted_at: row.try_get("deleted_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
    .map_err(invariant)
}

pub(crate) fn row_to_upload_session(row: PgRow) -> Result<UploadSession, RepositoryError> {
    let user = json_object(row.try_get("user_metadata").map_err(database_error)?)?;
    let ai = json_object(row.try_get("ai_metadata").map_err(database_error)?)?;
    UploadSession::from_persistence(PersistedUploadSession {
        id: UploadSessionId::from_uuid(row.try_get("id").map_err(database_error)?),
        media_id: MediaId::from_uuid(row.try_get("media_id").map_err(database_error)?),
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_id: BucketId::from_uuid(row.try_get("bucket_id").map_err(database_error)?),
        object_key: row.try_get("object_key").map_err(database_error)?,
        original_name: row.try_get("original_name").map_err(database_error)?,
        display_name: row.try_get("display_name").map_err(database_error)?,
        extension: row.try_get("extension").map_err(database_error)?,
        expected_size: as_u64(row.try_get("expected_size_bytes").map_err(database_error)?)?,
        expected_mime: row.try_get("expected_mime").map_err(database_error)?,
        storage_backend: row.try_get("storage_backend").map_err(database_error)?,
        storage_key: row.try_get("storage_key").map_err(database_error)?,
        visibility_override: row
            .try_get::<Option<String>, _>("visibility_override")
            .map_err(database_error)?
            .map(|value| parse_visibility(&value))
            .transpose()?,
        media_expires_at: row.try_get("media_expires_at").map_err(database_error)?,
        client_metadata: ClientMetadata::new(user, ai).map_err(invariant)?,
        session_expires_at: row.try_get("session_expires_at").map_err(database_error)?,
        state: parse_upload_state(&row.try_get::<String, _>("state").map_err(database_error)?)?,
        completed_at: row.try_get("completed_at").map_err(database_error)?,
        cancelled_at: row.try_get("cancelled_at").map_err(database_error)?,
        expired_at: row.try_get("expired_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
    .map_err(invariant)
}

pub(crate) fn row_to_outbox(row: PgRow) -> Result<OutboxEvent, RepositoryError> {
    Ok(OutboxEvent {
        id: row.try_get("id").map_err(database_error)?,
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        event_type: row.try_get("event_type").map_err(database_error)?,
        aggregate_id: row.try_get("aggregate_id").map_err(database_error)?,
        payload: row
            .try_get::<Json<Value>, _>("payload")
            .map_err(database_error)?
            .0,
        created_at: row.try_get("created_at").map_err(database_error)?,
        delivered_at: row.try_get("delivered_at").map_err(database_error)?,
        next_attempt_at: Some(row.try_get("available_at").map_err(database_error)?),
        attempt_count: as_u32(row.try_get("attempts").map_err(database_error)?)?,
    })
}

fn json_object(value: Json<Value>) -> Result<Map<String, Value>, RepositoryError> {
    value
        .0
        .as_object()
        .cloned()
        .ok_or_else(|| RepositoryError::Invariant("persisted metadata is not a JSON object".into()))
}

fn invariant(error: impl std::fmt::Display) -> RepositoryError {
    RepositoryError::Invariant(error.to_string())
}

pub(crate) fn database_error(error: sqlx::Error) -> RepositoryError {
    let sqlx::Error::Database(database) = &error else {
        return RepositoryError::Unavailable(error.to_string());
    };
    classify_sqlstate(database.code().as_deref(), database.message())
        .unwrap_or_else(|| RepositoryError::Unavailable(error.to_string()))
}

fn classify_sqlstate(code: Option<&str>, message: &str) -> Option<RepositoryError> {
    match code {
        Some("23505" | "40001" | "40P01") => Some(RepositoryError::Conflict),
        Some("23503") => Some(RepositoryError::NotFound),
        Some("23514") => Some(RepositoryError::Invariant(message.to_owned())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_conversion_never_wraps() {
        assert!(matches!(
            as_i64(u64::MAX),
            Err(RepositoryError::Invariant(_))
        ));
        assert!(matches!(as_u64(-1), Err(RepositoryError::Invariant(_))));
    }

    #[test]
    fn enum_encoding_matches_shared_storage_contract() {
        assert_eq!(visibility_name(Visibility::Private), "private");
        assert_eq!(
            media_state_name(MediaState::DeletePending),
            "delete_pending"
        );
        assert_eq!(
            upload_state_name(UploadSessionState::Completed),
            "completed"
        );
    }

    #[test]
    fn postgres_sqlstates_map_to_stable_repository_errors() {
        assert_eq!(
            classify_sqlstate(Some("23505"), "unique"),
            Some(RepositoryError::Conflict)
        );
        assert_eq!(
            classify_sqlstate(Some("40001"), "serialization"),
            Some(RepositoryError::Conflict)
        );
        assert_eq!(
            classify_sqlstate(Some("23503"), "foreign key"),
            Some(RepositoryError::NotFound)
        );
        assert!(classify_sqlstate(Some("08006"), "connection").is_none());
    }
}
