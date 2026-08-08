// Multipart locking, validation, persistent cleanup, and row conversion helpers.

pub(crate) async fn lock_upload(
    transaction: &mut Transaction<'_, Postgres>,
    upload_id: &str,
) -> Result<S3MultipartUpload, RepositoryError> {
    let row = sqlx::query("SELECT * FROM s3_multipart_uploads WHERE upload_id = $1 FOR UPDATE")
        .bind(upload_id)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?
        .ok_or(RepositoryError::NotFound)?;
    row_to_multipart_upload(row)
}

async fn load_completion_manifest(
    transaction: &mut Transaction<'_, Postgres>,
    upload_id: &str,
) -> Result<Vec<CompletedS3MultipartPart>, RepositoryError> {
    let manifest = sqlx::query_scalar::<_, Json<Vec<CompletedS3MultipartPart>>>(
        "SELECT completion_manifest FROM s3_multipart_uploads WHERE upload_id = $1",
    )
    .bind(upload_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .ok_or_else(|| {
        RepositoryError::Invariant("completing multipart upload has no manifest".into())
    })?;
    Ok(manifest.0)
}

async fn validate_completion_owner(
    transaction: &mut Transaction<'_, Postgres>,
    upload: &S3MultipartUpload,
    completion_token: &str,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    if upload.state != S3MultipartUploadState::Completing
        || upload
            .completion_lease_until
            .is_none_or(|lease_until| lease_until <= now)
    {
        return Err(RepositoryError::Conflict);
    }
    let owns_claim = sqlx::query_scalar::<_, bool>(
        "SELECT completion_token = $1 FROM s3_multipart_uploads WHERE upload_id = $2",
    )
    .bind(completion_token)
    .bind(&upload.upload_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)?;
    if owns_claim {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn multipart_stored_bytes(
    transaction: &mut Transaction<'_, Postgres>,
    upload_id: &str,
) -> Result<i64, RepositoryError> {
    sqlx::query_scalar(
        "SELECT COALESCE(SUM(size_bytes), 0)::BIGINT FROM s3_multipart_parts WHERE upload_id = $1",
    )
    .bind(upload_id)
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)
}

async fn mark_aborted(
    transaction: &mut Transaction<'_, Postgres>,
    upload_id: &str,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let changed = sqlx::query(
        "UPDATE s3_multipart_uploads SET state = 'aborted', completion_token = NULL, \
         completion_lease_until = NULL, completion_manifest = NULL, aborted_at = $1, \
         updated_at = $1 WHERE upload_id = $2 AND state IN ('pending', 'completing')",
    )
    .bind(now)
    .bind(upload_id)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    if changed.rows_affected() == 1 {
        Ok(())
    } else {
        Err(RepositoryError::Conflict)
    }
}

async fn list_parts(
    transaction: &mut Transaction<'_, Postgres>,
    upload_id: &str,
) -> Result<Vec<S3MultipartPart>, RepositoryError> {
    let rows =
        sqlx::query("SELECT * FROM s3_multipart_parts WHERE upload_id = $1 ORDER BY part_number")
            .bind(upload_id)
            .fetch_all(&mut **transaction)
            .await
            .map_err(database_error)?;
    rows.into_iter().map(row_to_multipart_part).collect()
}

async fn list_storage_keys(
    transaction: &mut Transaction<'_, Postgres>,
    upload_id: &str,
) -> Result<Vec<String>, RepositoryError> {
    sqlx::query_scalar(
        "SELECT storage_key FROM s3_multipart_parts WHERE upload_id = $1 ORDER BY part_number",
    )
    .bind(upload_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(database_error)
}

fn multipart_gc_task(
    upload: &S3MultipartUpload,
    upload_intent_id: Option<UploadIntentId>,
    storage_key: String,
    now: OffsetDateTime,
) -> NewStorageGcTask {
    NewStorageGcTask {
        id: StorageGcTaskId::new(),
        application_id: upload.application_id,
        bucket_id: upload.bucket_id,
        object_version_id: None,
        upload_intent_id,
        multipart_upload_id: Some(upload.upload_id.clone()),
        storage_backend: upload.storage_backend.clone(),
        storage_key,
        reason: StorageGcReason::MultipartTemporary,
        not_before: now,
        max_attempts: DEFAULT_S3_MULTIPART_GC_MAX_ATTEMPTS,
        created_at: now,
    }
}

async fn enqueue_multipart_storage_keys(
    transaction: &mut Transaction<'_, Postgres>,
    upload: &S3MultipartUpload,
    upload_intent_id: Option<UploadIntentId>,
    storage_keys: impl IntoIterator<Item = String>,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    for storage_key in storage_keys {
        let task = multipart_gc_task(upload, upload_intent_id, storage_key, now);
        insert_storage_gc_task(transaction, &task).await?;
    }
    Ok(())
}

fn validate_intent_identity(
    upload: &S3MultipartUpload,
    intent: &UploadIntent,
) -> Result<(), RepositoryError> {
    if intent.application_id() != upload.application_id
        || intent.bucket_id() != upload.bucket_id
        || intent.object_key() != upload.object_key
        || intent.storage_backend() != upload.storage_backend
        || intent.content_type() != Some(upload.content_type.as_str())
        || intent.user_metadata() != &upload.user_metadata
    {
        return Err(RepositoryError::Conflict);
    }
    Ok(())
}

fn validate_attachable_intent(
    upload: &S3MultipartUpload,
    intent: &UploadIntent,
    completion_token: &str,
    total_size: u64,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    validate_intent_identity(upload, intent)?;
    if intent.state() != UploadIntentState::Committing
        || intent.lease_token() != Some(completion_token)
        || intent
            .lease_until()
            .is_none_or(|lease_until| lease_until <= now)
        || intent.expected_size_bytes() != total_size
        || intent.size_bytes() != Some(total_size)
        || intent
            .entity_tag()
            .is_none_or(|etag| !is_multipart_etag(etag.as_str()))
        || intent.checksum().is_none()
    {
        return Err(RepositoryError::Conflict);
    }
    Ok(())
}

async fn enqueue_attached_intent_cleanup(
    transaction: &mut Transaction<'_, Postgres>,
    upload: &S3MultipartUpload,
    intent: &UploadIntent,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    validate_intent_identity(upload, intent)?;
    enqueue_multipart_storage_keys(
        transaction,
        upload,
        Some(intent.id()),
        [
            intent.temporary_storage_key().to_owned(),
            intent.final_storage_key().to_owned(),
        ],
        now,
    )
    .await
}

pub(crate) async fn lock_attached_upload_intent(
    transaction: &mut Transaction<'_, Postgres>,
    upload: &S3MultipartUpload,
) -> Result<Option<UploadIntent>, RepositoryError> {
    let Some(intent_id) = upload.upload_intent_id else {
        return Ok(None);
    };
    let intent = lock_upload_intent(transaction, intent_id).await?;
    validate_intent_identity(upload, &intent)?;
    Ok(Some(intent))
}

async fn abort_locked_attached_intent(
    transaction: &mut Transaction<'_, Postgres>,
    upload: &S3MultipartUpload,
    locked_intent: Option<&UploadIntent>,
    completion_token: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let intent = match (upload.upload_intent_id, locked_intent) {
        (None, None) => return Ok(()),
        (Some(expected_id), Some(intent)) if intent.id() == expected_id => intent,
        _ => return Err(RepositoryError::Conflict),
    };
    validate_intent_identity(upload, intent)?;
    if intent.state() == UploadIntentState::Committed {
        return Err(RepositoryError::Conflict);
    }
    if completion_token.is_some()
        && intent.state() == UploadIntentState::Committing
        && intent.lease_token() != completion_token
    {
        return Err(RepositoryError::Conflict);
    }
    enqueue_attached_intent_cleanup(transaction, upload, intent, now).await?;
    if !matches!(
        intent.state(),
        UploadIntentState::Aborted | UploadIntentState::Expired
    ) {
        let changed = sqlx::query(
            "UPDATE s3_upload_intents SET state = 'aborted', lease_token = NULL, \
             lease_until = NULL, updated_at = $2 WHERE id = $1 AND state <> 'committed'",
        )
        .bind(intent.id().as_uuid())
        .bind(now)
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?;
        if changed.rows_affected() != 1 {
            return Err(RepositoryError::Conflict);
        }
    }
    Ok(())
}

async fn abort_attached_intent(
    transaction: &mut Transaction<'_, Postgres>,
    upload: &S3MultipartUpload,
    completion_token: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let intent = lock_attached_upload_intent(transaction, upload).await?;
    abort_locked_attached_intent(transaction, upload, intent.as_ref(), completion_token, now).await
}

pub(crate) async fn abort_upload_and_enqueue_cleanup(
    transaction: &mut Transaction<'_, Postgres>,
    upload: &S3MultipartUpload,
    completion_token: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    let intent = lock_attached_upload_intent(transaction, upload).await?;
    abort_upload_and_enqueue_cleanup_with_locked_intent(
        transaction,
        upload,
        intent.as_ref(),
        completion_token,
        now,
    )
    .await
}

pub(crate) async fn abort_upload_and_enqueue_cleanup_with_locked_intent(
    transaction: &mut Transaction<'_, Postgres>,
    upload: &S3MultipartUpload,
    locked_intent: Option<&UploadIntent>,
    completion_token: Option<&str>,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    if !matches!(
        upload.state,
        S3MultipartUploadState::Pending | S3MultipartUploadState::Completing
    ) {
        return Err(RepositoryError::Conflict);
    }
    abort_locked_attached_intent(transaction, upload, locked_intent, completion_token, now).await?;
    let storage_keys = list_storage_keys(transaction, &upload.upload_id).await?;
    enqueue_multipart_storage_keys(transaction, upload, None, storage_keys, now).await?;
    mark_aborted(transaction, &upload.upload_id, now).await
}

fn validate_manifest(
    manifest: &[CompletedS3MultipartPart],
    parts: &[S3MultipartPart],
) -> Result<Vec<S3MultipartPart>, S3MultipartManifestError> {
    if manifest.is_empty() {
        return Err(S3MultipartManifestError::Empty);
    }
    let mut previous = None;
    for item in manifest {
        if !(1..=10_000).contains(&item.part_number) {
            return Err(S3MultipartManifestError::InvalidPartNumber(
                item.part_number,
            ));
        }
        if previous.is_some_and(|number| number >= item.part_number) {
            return Err(S3MultipartManifestError::InvalidPartOrder);
        }
        previous = Some(item.part_number);
    }
    let by_number = parts
        .iter()
        .map(|part| (part.part_number, part))
        .collect::<BTreeMap<_, _>>();
    manifest
        .iter()
        .map(|item| {
            let part = by_number
                .get(&item.part_number)
                .ok_or(S3MultipartManifestError::MissingPart(item.part_number))?;
            if part.etag != item.etag {
                return Err(S3MultipartManifestError::EtagMismatch(item.part_number));
            }
            Ok((*part).clone())
        })
        .collect()
}

fn selected_total_size(parts: &[S3MultipartPart]) -> Result<u64, RepositoryError> {
    parts.iter().try_fold(0_u64, |total, part| {
        total
            .checked_add(part.size)
            .ok_or_else(|| RepositoryError::Invariant("multipart total size exceeds u64".into()))
    })
}

pub(crate) fn row_to_multipart_upload(row: PgRow) -> Result<S3MultipartUpload, RepositoryError> {
    let final_etag = row
        .try_get::<Option<String>, _>("final_etag")
        .map_err(database_error)?
        .map(EntityTag::new)
        .transpose()
        .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
    let checksum_algorithm = row
        .try_get::<Option<String>, _>("final_checksum_algorithm")
        .map_err(database_error)?;
    let checksum_value = row
        .try_get::<Option<String>, _>("final_checksum_value")
        .map_err(database_error)?;
    let final_checksum = match (checksum_algorithm.as_deref(), checksum_value) {
        (None, None) => None,
        (Some("sha256"), Some(value)) => Some(
            Checksum::sha256_hex(value)
                .map_err(|error| RepositoryError::Invariant(error.to_string()))?,
        ),
        _ => {
            return Err(RepositoryError::Invariant(
                "persisted multipart checksum fields are inconsistent".into(),
            ));
        }
    };
    Ok(S3MultipartUpload {
        upload_id: row.try_get("upload_id").map_err(database_error)?,
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_id: BucketId::from_uuid(row.try_get("bucket_id").map_err(database_error)?),
        object_key: row.try_get("object_key").map_err(database_error)?,
        content_type: row.try_get("content_type").map_err(database_error)?,
        user_metadata: row
            .try_get::<Json<serde_json::Value>, _>("user_metadata")
            .map_err(database_error)?
            .0,
        object_tags: {
            let tags = row
                .try_get::<Json<S3ObjectTagSet>, _>("object_tags")
                .map_err(database_error)?
                .0;
            tags.validate()
                .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
            tags
        },
        storage_backend: row.try_get("storage_backend").map_err(database_error)?,
        state: parse_state(&row.try_get::<String, _>("state").map_err(database_error)?)?,
        expires_at: row.try_get("expires_at").map_err(database_error)?,
        completion_lease_until: row
            .try_get("completion_lease_until")
            .map_err(database_error)?,
        upload_intent_id: row
            .try_get::<Option<uuid::Uuid>, _>("upload_intent_id")
            .map_err(database_error)?
            .map(UploadIntentId::from_uuid),
        object_id: row
            .try_get::<Option<uuid::Uuid>, _>("object_id")
            .map_err(database_error)?
            .map(ObjectId::from_uuid),
        object_version_id: row
            .try_get::<Option<uuid::Uuid>, _>("object_version_id")
            .map_err(database_error)?
            .map(ObjectVersionId::from_uuid),
        final_etag,
        final_checksum,
        completed_at: row.try_get("completed_at").map_err(database_error)?,
        aborted_at: row.try_get("aborted_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_multipart_part(row: PgRow) -> Result<S3MultipartPart, RepositoryError> {
    let part_number = row
        .try_get::<i32, _>("part_number")
        .map_err(database_error)?;
    Ok(S3MultipartPart {
        upload_id: row.try_get("upload_id").map_err(database_error)?,
        part_number: u16::try_from(part_number).map_err(|_| {
            RepositoryError::Invariant("persisted multipart part number is invalid".into())
        })?,
        size: as_u64(row.try_get("size_bytes").map_err(database_error)?)?,
        sha256: row.try_get("sha256").map_err(database_error)?,
        md5: row.try_get("md5").map_err(database_error)?,
        etag: row.try_get("etag").map_err(database_error)?,
        storage_key: row.try_get("storage_key").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn parse_state(value: &str) -> Result<S3MultipartUploadState, RepositoryError> {
    match value {
        "pending" => Ok(S3MultipartUploadState::Pending),
        "completing" => Ok(S3MultipartUploadState::Completing),
        "completed" => Ok(S3MultipartUploadState::Completed),
        "aborted" => Ok(S3MultipartUploadState::Aborted),
        _ => Err(RepositoryError::Invariant(
            "persisted multipart upload state is invalid".into(),
        )),
    }
}
