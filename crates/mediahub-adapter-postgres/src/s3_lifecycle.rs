use async_trait::async_trait;
use mediahub_app::{
    ExecuteS3LifecycleCommand, MAX_S3_LIFECYCLE_BATCH_SIZE, RepositoryError,
    S3CurrentExpirationCandidate, S3ExpiredDeleteMarkerCandidate, S3LifecycleExecutionOutcome,
    S3LifecycleRepository, S3LifecycleTarget, S3MultipartLifecycleCandidate, S3MultipartUpload,
    S3MultipartUploadState, S3NoncurrentExpirationCandidate, lifecycle_action_time,
    lifecycle_days_cutoff, lifecycle_prefix, lifecycle_rule_aborts_multipart,
    lifecycle_rule_is_current_due, lifecycle_rule_is_noncurrent_due,
    lifecycle_rule_removes_expired_marker_at,
};
use mediahub_core::{
    ApplicationId, BucketId, NewStorageGcTask, ObjectId, ObjectVersion, ObjectVersionId,
    ObjectVersionPayload, OffsetDateTime, S3Bucket, S3Expiration, S3LifecycleRuleStatus, S3Object,
    S3VersionId, SourceProtocol, StorageGcReason, UploadIntent, VersioningStatus,
};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow};

use crate::{
    PostgresRepository,
    codec::{as_i64, database_error, postgres_time},
    s3_multipart::{
        abort_upload_and_enqueue_cleanup_with_locked_intent, lock_attached_upload_intent,
        lock_upload,
    },
    s3_repository::{
        advance_object_to_delete_marker, delete_lock_reason_at, enqueue_object_version_gc_task,
        hide_exact_deleted_version, insert_object_version, mark_current_version_noncurrent,
        row_to_object_version, row_to_s3_bucket, row_to_s3_object, supersede_active_null_version,
        update_deleted_object_head,
    },
};

#[async_trait]
impl S3LifecycleRepository for PostgresRepository {
    async fn list_s3_lifecycle_buckets(
        &self,
        after_bucket_id: Option<BucketId>,
        limit: usize,
    ) -> Result<Vec<S3Bucket>, RepositoryError> {
        validate_limit(limit)?;
        let rows = sqlx::query(
            "SELECT * FROM buckets
             WHERE s3_lifecycle_configuration IS NOT NULL
               AND ($1::uuid IS NULL OR id > $1)
             ORDER BY id
             LIMIT $2",
        )
        .bind(after_bucket_id.map(BucketId::as_uuid))
        .bind(as_i64(limit as u64)?)
        .fetch_all(self.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_s3_bucket).collect()
    }

    async fn list_s3_current_expiration_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        created_before: Option<OffsetDateTime>,
        _evaluated_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3CurrentExpirationCandidate>, RepositoryError> {
        validate_limit(limit)?;
        let rows = sqlx::query(
            "SELECT o.id AS object_id, o.object_key, o.current_version_id,
                    ov.created_at AS version_created_at
             FROM objects o
             JOIN object_versions ov ON ov.id = o.current_version_id
             WHERE o.application_id = $1 AND o.bucket_id = $2
               AND ov.application_id = $1 AND ov.bucket_id = $2
               AND o.object_key COLLATE \"C\" >= $3
               AND ($4::text IS NULL OR o.object_key COLLATE \"C\" < ($4 COLLATE \"C\"))
               AND ov.state = 'committed' AND ov.superseded_at IS NULL
               AND NOT ov.is_delete_marker
               AND ($5::timestamptz IS NULL OR ov.created_at < $5)
             ORDER BY o.object_key COLLATE \"C\", o.id
             LIMIT $6",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_id.as_uuid())
        .bind(prefix)
        .bind(lifecycle_prefix_upper_bound(prefix))
        .bind(created_before.map(postgres_time))
        .bind(as_i64(limit as u64)?)
        .fetch_all(self.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_current_candidate).collect()
    }

    async fn list_s3_noncurrent_expiration_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        became_noncurrent_before: OffsetDateTime,
        evaluated_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3NoncurrentExpirationCandidate>, RepositoryError> {
        validate_limit(limit)?;
        let rows = sqlx::query(
            "SELECT o.id AS object_id, o.object_key, ov.id AS version_id,
                    ov.became_noncurrent_at
             FROM object_versions ov
             JOIN objects o ON o.id = ov.object_id
             WHERE ov.application_id = $1 AND ov.bucket_id = $2
               AND o.application_id = $1 AND o.bucket_id = $2
               AND o.object_key COLLATE \"C\" >= $3
               AND ($4::text IS NULL OR o.object_key COLLATE \"C\" < ($4 COLLATE \"C\"))
               AND ov.state = 'committed' AND ov.superseded_at IS NULL
               AND NOT ov.is_delete_marker
               AND ov.became_noncurrent_at < $5
               AND o.current_version_id IS DISTINCT FROM ov.id
               AND NOT ov.legal_hold
               AND (ov.retain_until IS NULL OR ov.retain_until <= $6)
             ORDER BY ov.became_noncurrent_at, ov.id
             LIMIT $7",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_id.as_uuid())
        .bind(prefix)
        .bind(lifecycle_prefix_upper_bound(prefix))
        .bind(postgres_time(became_noncurrent_before))
        .bind(postgres_time(evaluated_at))
        .bind(as_i64(limit as u64)?)
        .fetch_all(self.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_noncurrent_candidate).collect()
    }

    async fn list_s3_expired_delete_marker_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<S3ExpiredDeleteMarkerCandidate>, RepositoryError> {
        validate_limit(limit)?;
        let rows = sqlx::query(
            "SELECT o.id AS object_id, o.object_key, marker.id AS marker_version_id
             FROM objects o
             JOIN object_versions marker ON marker.id = o.current_version_id
             WHERE o.application_id = $1 AND o.bucket_id = $2
               AND marker.application_id = $1 AND marker.bucket_id = $2
               AND o.object_key COLLATE \"C\" >= $3
               AND ($4::text IS NULL OR o.object_key COLLATE \"C\" < ($4 COLLATE \"C\"))
               AND marker.state = 'committed' AND marker.superseded_at IS NULL
               AND marker.is_delete_marker
               AND NOT EXISTS (
                   SELECT 1 FROM object_versions other
                   WHERE other.object_id = o.id AND other.id <> marker.id
                     AND other.state = 'committed' AND other.superseded_at IS NULL
             )
             ORDER BY o.object_key COLLATE \"C\", o.id
             LIMIT $5",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_id.as_uuid())
        .bind(prefix)
        .bind(lifecycle_prefix_upper_bound(prefix))
        .bind(as_i64(limit as u64)?)
        .fetch_all(self.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_marker_candidate).collect()
    }

    async fn list_s3_expiration_delete_marker_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        rule: &mediahub_core::S3LifecycleRule,
        evaluated_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3ExpiredDeleteMarkerCandidate>, RepositoryError> {
        validate_limit(limit)?;
        if rule.status != S3LifecycleRuleStatus::Enabled {
            return Ok(Vec::new());
        }
        let marker_created_before = match rule.expiration.as_ref() {
            Some(S3Expiration::ExpiredObjectDeleteMarker) => None,
            Some(S3Expiration::Days(days)) => Some(lifecycle_days_cutoff(evaluated_at, *days)?),
            Some(S3Expiration::Date(date)) if *date <= evaluated_at => None,
            Some(S3Expiration::Date(_)) | None => return Ok(Vec::new()),
        };
        let prefix = lifecycle_prefix(&rule.filter);
        let rows = sqlx::query(
            "SELECT o.id AS object_id, o.object_key, marker.id AS marker_version_id
             FROM object_versions marker
             JOIN objects o ON o.id = marker.object_id AND o.current_version_id = marker.id
             WHERE marker.application_id = $1 AND marker.bucket_id = $2
               AND o.application_id = $1 AND o.bucket_id = $2
               AND o.object_key COLLATE \"C\" >= $3
               AND ($4::text IS NULL OR o.object_key COLLATE \"C\" < ($4 COLLATE \"C\"))
               AND marker.state = 'committed' AND marker.superseded_at IS NULL
               AND marker.is_delete_marker
               AND ($5::timestamptz IS NULL OR marker.created_at < $5)
               AND NOT EXISTS (
                   SELECT 1 FROM object_versions other
                   WHERE other.object_id = o.id AND other.id <> marker.id
                     AND other.state = 'committed' AND other.superseded_at IS NULL
             )
             ORDER BY marker.created_at, marker.id
             LIMIT $6",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_id.as_uuid())
        .bind(prefix)
        .bind(lifecycle_prefix_upper_bound(prefix))
        .bind(marker_created_before.map(postgres_time))
        .bind(as_i64(limit as u64)?)
        .fetch_all(self.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_marker_candidate).collect()
    }

    async fn list_s3_multipart_lifecycle_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        initiated_before: OffsetDateTime,
        evaluated_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3MultipartLifecycleCandidate>, RepositoryError> {
        validate_limit(limit)?;
        let rows = sqlx::query(
            "SELECT upload_id, object_key, created_at
             FROM s3_multipart_uploads
             WHERE application_id = $1 AND bucket_id = $2
               AND object_key COLLATE \"C\" >= $3
               AND ($4::text IS NULL OR object_key COLLATE \"C\" < ($4 COLLATE \"C\"))
               AND created_at < $5
               AND (state = 'pending'
                    OR (state = 'completing' AND completion_lease_until <= $6))
             ORDER BY created_at, upload_id COLLATE \"C\"
             LIMIT $7",
        )
        .bind(application_id.as_uuid())
        .bind(bucket_id.as_uuid())
        .bind(prefix)
        .bind(lifecycle_prefix_upper_bound(prefix))
        .bind(postgres_time(initiated_before))
        .bind(postgres_time(evaluated_at))
        .bind(as_i64(limit as u64)?)
        .fetch_all(self.pool())
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_multipart_candidate).collect()
    }

    async fn execute_s3_lifecycle(
        &self,
        command: &ExecuteS3LifecycleCommand,
    ) -> Result<S3LifecycleExecutionOutcome, RepositoryError> {
        if command.gc_max_attempts == 0 {
            return Err(RepositoryError::Invariant(
                "S3 lifecycle GC attempts must be positive".into(),
            ));
        }
        let mut transaction = self.pool().begin().await.map_err(database_error)?;
        let outcome = if let S3LifecycleTarget::AbortMultipart {
            upload_id,
            object_key,
            expected_initiated_at,
        } = &command.target
        {
            // CompleteMultipartUpload owns this same order. Locking the upload before the bucket
            // prevents the background worker from forming bucket -> upload / upload -> bucket ABBA.
            let upload = match lock_upload(&mut transaction, upload_id).await {
                Ok(upload) => upload,
                Err(RepositoryError::NotFound) => {
                    return Ok(S3LifecycleExecutionOutcome::AlreadyApplied);
                }
                Err(error) => return Err(error),
            };
            if upload.application_id != command.application_id
                || upload.bucket_id != command.bucket_id
                || upload.object_key.as_str() != object_key
                || upload.created_at != *expected_initiated_at
            {
                return Ok(S3LifecycleExecutionOutcome::NotEligible);
            }
            let locked_intent = lock_attached_upload_intent(&mut transaction, &upload).await?;
            let Some(_bucket) = lock_lifecycle_bucket(&mut transaction, command).await? else {
                return Ok(S3LifecycleExecutionOutcome::ConfigurationChanged);
            };
            execute_multipart_abort(&mut transaction, command, &upload, locked_intent.as_ref())
                .await?
        } else {
            let Some(bucket) = lock_lifecycle_bucket(&mut transaction, command).await? else {
                return Ok(S3LifecycleExecutionOutcome::ConfigurationChanged);
            };
            match &command.target {
                S3LifecycleTarget::ExpireCurrent { .. } => {
                    execute_current_expiration(&mut transaction, &bucket, command).await?
                }
                S3LifecycleTarget::ExpireNoncurrent { .. } => {
                    execute_noncurrent_expiration(&mut transaction, command).await?
                }
                S3LifecycleTarget::RemoveExpiredDeleteMarker { .. } => {
                    execute_expired_marker(&mut transaction, command).await?
                }
                S3LifecycleTarget::AbortMultipart { .. } => unreachable!(
                    "multipart lifecycle is dispatched before the bucket-first object path"
                ),
            }
        };
        transaction.commit().await.map_err(database_error)?;
        Ok(outcome)
    }
}

async fn execute_current_expiration(
    transaction: &mut Transaction<'_, Postgres>,
    bucket: &S3Bucket,
    command: &ExecuteS3LifecycleCommand,
) -> Result<S3LifecycleExecutionOutcome, RepositoryError> {
    let S3LifecycleTarget::ExpireCurrent {
        object_id,
        object_key,
        expected_current_version_id,
        version_created_at,
    } = &command.target
    else {
        return Err(RepositoryError::Invariant(
            "current lifecycle target is inconsistent".into(),
        ));
    };
    let Some(object) = lock_object(transaction, command, *object_id, object_key).await? else {
        return Ok(S3LifecycleExecutionOutcome::AlreadyApplied);
    };
    if object.current_version_id() != Some(*expected_current_version_id) {
        return Ok(S3LifecycleExecutionOutcome::TargetChanged);
    }
    let Some(target) = lock_active_version(
        transaction,
        command,
        object.id(),
        *expected_current_version_id,
    )
    .await?
    else {
        return Ok(S3LifecycleExecutionOutcome::TargetChanged);
    };
    if target.created_at() != *version_created_at
        || target.is_delete_marker()
        || !lifecycle_rule_is_current_due(
            &command.rule,
            object_key,
            target.created_at(),
            command.evaluated_at,
        )
    {
        return Ok(S3LifecycleExecutionOutcome::NotEligible);
    }
    match bucket.configuration().versioning_status() {
        VersioningStatus::Enabled => {
            let action_time = lifecycle_action_time(command.evaluated_at);
            let marker = lifecycle_delete_marker(&object, false, command)?;
            mark_current_version_noncurrent(transaction, &object, action_time).await?;
            insert_object_version(transaction, &marker).await?;
            advance_object_to_delete_marker(transaction, &object, &marker, action_time).await?;
        }
        VersioningStatus::Suspended => {
            let locked_active_null = if target.is_null_version() {
                None
            } else {
                lock_active_null_version(transaction, command, object.id()).await?
            };
            let active_null = if target.is_null_version() {
                Some(&target)
            } else {
                locked_active_null.as_ref()
            };
            if active_null.is_some_and(|version| {
                delete_lock_reason_at(version, command.evaluated_at, false).is_some()
            }) {
                return Ok(S3LifecycleExecutionOutcome::Locked);
            }
            let action_time = lifecycle_action_time(command.evaluated_at);
            let marker = lifecycle_delete_marker(&object, true, command)?;
            mark_current_version_noncurrent(transaction, &object, action_time).await?;
            if let Some(version) = active_null {
                enqueue_lifecycle_gc(transaction, version, command).await?;
                supersede_active_null_version(transaction, Some(version), command.evaluated_at)
                    .await?;
            }
            insert_object_version(transaction, &marker).await?;
            advance_object_to_delete_marker(transaction, &object, &marker, action_time).await?;
        }
        VersioningStatus::Unversioned => {
            if !target.is_null_version() {
                return Ok(S3LifecycleExecutionOutcome::TargetChanged);
            }
            if delete_lock_reason_at(&target, command.evaluated_at, false).is_some() {
                return Ok(S3LifecycleExecutionOutcome::Locked);
            }
            mark_current_version_noncurrent(transaction, &object, command.evaluated_at).await?;
            enqueue_lifecycle_gc(transaction, &target, command).await?;
            supersede_active_null_version(transaction, Some(&target), command.evaluated_at).await?;
            update_deleted_object_head(transaction, &object, None, command.evaluated_at).await?;
        }
    }
    Ok(S3LifecycleExecutionOutcome::Applied)
}

async fn execute_noncurrent_expiration(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ExecuteS3LifecycleCommand,
) -> Result<S3LifecycleExecutionOutcome, RepositoryError> {
    let S3LifecycleTarget::ExpireNoncurrent {
        object_id,
        object_key,
        version_id,
        expected_became_noncurrent_at,
    } = &command.target
    else {
        return Err(RepositoryError::Invariant(
            "noncurrent lifecycle target is inconsistent".into(),
        ));
    };
    let Some(object) = lock_object(transaction, command, *object_id, object_key).await? else {
        return Ok(S3LifecycleExecutionOutcome::AlreadyApplied);
    };
    if object.current_version_id() == Some(*version_id) {
        return Ok(S3LifecycleExecutionOutcome::TargetChanged);
    }
    let Some(version) = lock_active_version(transaction, command, object.id(), *version_id).await?
    else {
        return Ok(S3LifecycleExecutionOutcome::AlreadyApplied);
    };
    if version.is_delete_marker()
        || version.became_noncurrent_at() != Some(*expected_became_noncurrent_at)
        || !lifecycle_rule_is_noncurrent_due(
            &command.rule,
            object_key,
            *expected_became_noncurrent_at,
            command.evaluated_at,
        )
    {
        return Ok(S3LifecycleExecutionOutcome::NotEligible);
    }
    if delete_lock_reason_at(&version, command.evaluated_at, false).is_some() {
        return Ok(S3LifecycleExecutionOutcome::Locked);
    }
    hide_exact_deleted_version(transaction, &version, command.evaluated_at).await?;
    enqueue_lifecycle_gc(transaction, &version, command).await?;
    Ok(S3LifecycleExecutionOutcome::Applied)
}

async fn execute_expired_marker(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ExecuteS3LifecycleCommand,
) -> Result<S3LifecycleExecutionOutcome, RepositoryError> {
    let S3LifecycleTarget::RemoveExpiredDeleteMarker {
        object_id,
        object_key,
        marker_version_id,
    } = &command.target
    else {
        return Err(RepositoryError::Invariant(
            "delete-marker lifecycle target is inconsistent".into(),
        ));
    };
    let Some(object) = lock_object(transaction, command, *object_id, object_key).await? else {
        return Ok(S3LifecycleExecutionOutcome::AlreadyApplied);
    };
    if object.current_version_id() != Some(*marker_version_id) {
        return Ok(S3LifecycleExecutionOutcome::TargetChanged);
    }
    let Some(marker) =
        lock_active_version(transaction, command, object.id(), *marker_version_id).await?
    else {
        return Ok(S3LifecycleExecutionOutcome::AlreadyApplied);
    };
    if !marker.is_delete_marker()
        || !lifecycle_rule_removes_expired_marker_at(
            &command.rule,
            object_key,
            marker.created_at(),
            command.evaluated_at,
        )
        || has_other_active_version(transaction, object.id(), marker.id()).await?
    {
        return Ok(S3LifecycleExecutionOutcome::NotEligible);
    }
    hide_exact_deleted_version(transaction, &marker, command.evaluated_at).await?;
    update_deleted_object_head(transaction, &object, None, command.evaluated_at).await?;
    Ok(S3LifecycleExecutionOutcome::Applied)
}

async fn execute_multipart_abort(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ExecuteS3LifecycleCommand,
    upload: &S3MultipartUpload,
    locked_intent: Option<&UploadIntent>,
) -> Result<S3LifecycleExecutionOutcome, RepositoryError> {
    let S3LifecycleTarget::AbortMultipart {
        upload_id,
        object_key,
        expected_initiated_at,
    } = &command.target
    else {
        return Err(RepositoryError::Invariant(
            "multipart lifecycle target is inconsistent".into(),
        ));
    };
    if upload.upload_id.as_str() != upload_id
        || upload.application_id != command.application_id
        || upload.bucket_id != command.bucket_id
        || upload.object_key.as_str() != object_key
        || upload.created_at != *expected_initiated_at
        || !lifecycle_rule_aborts_multipart(
            &command.rule,
            object_key,
            upload.created_at,
            command.evaluated_at,
        )
    {
        return Ok(S3LifecycleExecutionOutcome::NotEligible);
    }
    match upload.state {
        S3MultipartUploadState::Aborted => {
            return Ok(S3LifecycleExecutionOutcome::AlreadyApplied);
        }
        S3MultipartUploadState::Completed => {
            return Ok(S3LifecycleExecutionOutcome::TargetChanged);
        }
        S3MultipartUploadState::Completing
            if upload
                .completion_lease_until
                .is_some_and(|lease_until| lease_until > command.evaluated_at) =>
        {
            return Ok(S3LifecycleExecutionOutcome::Busy);
        }
        S3MultipartUploadState::Pending | S3MultipartUploadState::Completing => {}
    }
    abort_upload_and_enqueue_cleanup_with_locked_intent(
        transaction,
        upload,
        locked_intent,
        None,
        command.evaluated_at,
    )
    .await?;
    Ok(S3LifecycleExecutionOutcome::Applied)
}

async fn lock_object(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ExecuteS3LifecycleCommand,
    object_id: ObjectId,
    object_key: &str,
) -> Result<Option<S3Object>, RepositoryError> {
    sqlx::query(
        "SELECT * FROM objects
         WHERE id = $1 AND application_id = $2 AND bucket_id = $3 AND object_key = $4
         FOR UPDATE",
    )
    .bind(object_id.as_uuid())
    .bind(command.application_id.as_uuid())
    .bind(command.bucket_id.as_uuid())
    .bind(object_key)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .map(row_to_s3_object)
    .transpose()
}

async fn lock_lifecycle_bucket(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ExecuteS3LifecycleCommand,
) -> Result<Option<S3Bucket>, RepositoryError> {
    let row = sqlx::query("SELECT * FROM buckets WHERE id = $1 AND application_id = $2 FOR SHARE")
        .bind(command.bucket_id.as_uuid())
        .bind(command.application_id.as_uuid())
        .fetch_optional(&mut **transaction)
        .await
        .map_err(database_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let bucket = row_to_s3_bucket(row)?;
    let configuration_matches = bucket.configuration().revision()
        == command.expected_configuration_revision
        && bucket
            .configuration()
            .lifecycle_configuration()
            .is_some_and(|configuration| configuration.rules.contains(&command.rule));
    Ok(configuration_matches.then_some(bucket))
}

async fn lock_active_version(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ExecuteS3LifecycleCommand,
    object_id: ObjectId,
    version_id: ObjectVersionId,
) -> Result<Option<ObjectVersion>, RepositoryError> {
    sqlx::query(
        "SELECT * FROM object_versions
         WHERE id = $1 AND object_id = $2 AND application_id = $3 AND bucket_id = $4
           AND state = 'committed' AND superseded_at IS NULL
         FOR UPDATE",
    )
    .bind(version_id.as_uuid())
    .bind(object_id.as_uuid())
    .bind(command.application_id.as_uuid())
    .bind(command.bucket_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .map(row_to_object_version)
    .transpose()
}

async fn lock_active_null_version(
    transaction: &mut Transaction<'_, Postgres>,
    command: &ExecuteS3LifecycleCommand,
    object_id: ObjectId,
) -> Result<Option<ObjectVersion>, RepositoryError> {
    sqlx::query(
        "SELECT * FROM object_versions
         WHERE object_id = $1 AND application_id = $2 AND bucket_id = $3
           AND external_version_id = 'null'
           AND state = 'committed' AND superseded_at IS NULL
         FOR UPDATE",
    )
    .bind(object_id.as_uuid())
    .bind(command.application_id.as_uuid())
    .bind(command.bucket_id.as_uuid())
    .fetch_optional(&mut **transaction)
    .await
    .map_err(database_error)?
    .map(row_to_object_version)
    .transpose()
}

async fn has_other_active_version(
    transaction: &mut Transaction<'_, Postgres>,
    object_id: ObjectId,
    marker_id: ObjectVersionId,
) -> Result<bool, RepositoryError> {
    sqlx::query_scalar(
        "SELECT EXISTS(
             SELECT 1 FROM object_versions
             WHERE object_id = $1 AND id <> $2
               AND state = 'committed' AND superseded_at IS NULL
         )",
    )
    .bind(object_id.as_uuid())
    .bind(marker_id.as_uuid())
    .fetch_one(&mut **transaction)
    .await
    .map_err(database_error)
}

fn lifecycle_delete_marker(
    object: &S3Object,
    null_version: bool,
    command: &ExecuteS3LifecycleCommand,
) -> Result<ObjectVersion, RepositoryError> {
    let generation = object
        .generation()
        .checked_add(1)
        .ok_or_else(|| RepositoryError::Invariant("object generation overflow".into()))?;
    let external_version_id = if null_version {
        S3VersionId::new("null")
    } else {
        Ok(command.delete_marker_version_id.clone())
    }
    .map_err(|error| RepositoryError::Invariant(error.to_string()))?;
    ObjectVersion::new_delete_marker(
        command.delete_marker_id,
        object.id(),
        command.application_id,
        command.bucket_id,
        external_version_id,
        generation,
        null_version,
        "lifecycle",
        SourceProtocol::S3,
        lifecycle_action_time(command.evaluated_at),
    )
    .map_err(|error| RepositoryError::Invariant(error.to_string()))
}

async fn enqueue_lifecycle_gc(
    transaction: &mut Transaction<'_, Postgres>,
    version: &ObjectVersion,
    command: &ExecuteS3LifecycleCommand,
) -> Result<(), RepositoryError> {
    let ObjectVersionPayload::Object(payload) = version.payload() else {
        return Ok(());
    };
    enqueue_object_version_gc_task(
        transaction,
        &NewStorageGcTask {
            id: command.gc_task_id,
            application_id: version.application_id(),
            bucket_id: version.bucket_id(),
            object_version_id: Some(version.id()),
            upload_intent_id: None,
            multipart_upload_id: None,
            storage_backend: payload.storage_backend().to_owned(),
            storage_key: payload.storage_key().to_owned(),
            reason: StorageGcReason::LifecycleExpiration,
            not_before: command.gc_not_before,
            max_attempts: command.gc_max_attempts,
            created_at: command.evaluated_at,
        },
    )
    .await
}

fn lifecycle_prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut upper = prefix.to_owned();
    while let Some((index, character)) = upper.char_indices().next_back() {
        upper.truncate(index);
        let scalar = u32::from(character);
        let next_scalar = match scalar {
            0x10_ffff => continue,
            0xd7ff => 0xe000,
            value => value + 1,
        };
        if let Some(next) = char::from_u32(next_scalar) {
            upper.push(next);
            return Some(upper);
        }
    }
    None
}

fn row_to_current_candidate(row: PgRow) -> Result<S3CurrentExpirationCandidate, RepositoryError> {
    Ok(S3CurrentExpirationCandidate {
        object_id: ObjectId::from_uuid(row.try_get("object_id").map_err(database_error)?),
        object_key: row.try_get("object_key").map_err(database_error)?,
        current_version_id: ObjectVersionId::from_uuid(
            row.try_get("current_version_id").map_err(database_error)?,
        ),
        version_created_at: row.try_get("version_created_at").map_err(database_error)?,
    })
}

fn row_to_noncurrent_candidate(
    row: PgRow,
) -> Result<S3NoncurrentExpirationCandidate, RepositoryError> {
    Ok(S3NoncurrentExpirationCandidate {
        object_id: ObjectId::from_uuid(row.try_get("object_id").map_err(database_error)?),
        object_key: row.try_get("object_key").map_err(database_error)?,
        version_id: ObjectVersionId::from_uuid(row.try_get("version_id").map_err(database_error)?),
        became_noncurrent_at: row
            .try_get("became_noncurrent_at")
            .map_err(database_error)?,
    })
}

fn row_to_marker_candidate(row: PgRow) -> Result<S3ExpiredDeleteMarkerCandidate, RepositoryError> {
    Ok(S3ExpiredDeleteMarkerCandidate {
        object_id: ObjectId::from_uuid(row.try_get("object_id").map_err(database_error)?),
        object_key: row.try_get("object_key").map_err(database_error)?,
        marker_version_id: ObjectVersionId::from_uuid(
            row.try_get("marker_version_id").map_err(database_error)?,
        ),
    })
}

fn row_to_multipart_candidate(
    row: PgRow,
) -> Result<S3MultipartLifecycleCandidate, RepositoryError> {
    Ok(S3MultipartLifecycleCandidate {
        upload_id: row.try_get("upload_id").map_err(database_error)?,
        object_key: row.try_get("object_key").map_err(database_error)?,
        initiated_at: row.try_get("created_at").map_err(database_error)?,
    })
}

fn validate_limit(limit: usize) -> Result<(), RepositoryError> {
    if (1..=MAX_S3_LIFECYCLE_BATCH_SIZE).contains(&limit) {
        Ok(())
    } else {
        Err(RepositoryError::Invariant(
            "S3 lifecycle repository limit must be between 1 and 1000".into(),
        ))
    }
}
