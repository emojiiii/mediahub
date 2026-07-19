// Multipart locking, validation, and row conversion helpers.

async fn lock_upload(
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
) -> Result<(), RepositoryError> {
    if upload.state != S3MultipartUploadState::Completing {
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

async fn multipart_reserved_bytes(
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

async fn adjust_reserved_bytes(
    transaction: &mut Transaction<'_, Postgres>,
    application_id: ApplicationId,
    delta: i64,
) -> Result<(), RepositoryError> {
    if delta == 0 {
        return Ok(());
    }
    let changed = if delta > 0 {
        sqlx::query(
            "UPDATE applications SET reserved_bytes = reserved_bytes + $1 WHERE id = $2 \
             AND quota_bytes - used_bytes - reserved_bytes >= $1",
        )
        .bind(delta)
        .bind(application_id.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?
    } else {
        let released = delta.checked_neg().ok_or_else(|| {
            RepositoryError::Invariant("multipart reservation delta overflow".into())
        })?;
        sqlx::query(
            "UPDATE applications SET reserved_bytes = reserved_bytes - $1 WHERE id = $2 \
             AND reserved_bytes >= $1",
        )
        .bind(released)
        .bind(application_id.as_uuid())
        .execute(&mut **transaction)
        .await
        .map_err(database_error)?
    };
    if changed.rows_affected() == 1 {
        Ok(())
    } else if delta > 0 {
        Err(RepositoryError::QuotaExceeded)
    } else {
        Err(RepositoryError::Invariant(
            "multipart upload has no matching quota reservation".into(),
        ))
    }
}

async fn abort_and_release_parts(
    transaction: &mut Transaction<'_, Postgres>,
    upload: &S3MultipartUpload,
    now: OffsetDateTime,
) -> Result<(), RepositoryError> {
    if !matches!(
        upload.state,
        S3MultipartUploadState::Pending | S3MultipartUploadState::Completing
    ) {
        return Err(RepositoryError::Conflict);
    }
    let reserved = multipart_reserved_bytes(transaction, &upload.upload_id).await?;
    adjust_reserved_bytes(transaction, upload.application_id, -reserved).await?;
    mark_aborted(transaction, &upload.upload_id, now).await
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

fn row_to_multipart_upload(row: PgRow) -> Result<S3MultipartUpload, RepositoryError> {
    Ok(S3MultipartUpload {
        upload_id: row.try_get("upload_id").map_err(database_error)?,
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        bucket_id: BucketId::from_uuid(row.try_get("bucket_id").map_err(database_error)?),
        object_key: row.try_get("object_key").map_err(database_error)?,
        content_type: row.try_get("content_type").map_err(database_error)?,
        visibility_override: row
            .try_get::<Option<String>, _>("visibility_override")
            .map_err(database_error)?
            .map(|value| parse_visibility(&value))
            .transpose()?,
        state: parse_state(&row.try_get::<String, _>("state").map_err(database_error)?)?,
        expires_at: row.try_get("expires_at").map_err(database_error)?,
        completion_lease_until: row
            .try_get("completion_lease_until")
            .map_err(database_error)?,
        media_id: row
            .try_get::<Option<uuid::Uuid>, _>("media_id")
            .map_err(database_error)?
            .map(MediaId::from_uuid),
        final_etag: row.try_get("final_etag").map_err(database_error)?,
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
