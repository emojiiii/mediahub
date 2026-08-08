use std::collections::HashSet;

use async_trait::async_trait;
use mediahub_core::{
    ApplicationId, BucketId, ObjectId, ObjectVersionId, S3Bucket, S3Expiration, S3LifecycleFilter,
    S3LifecycleRule, S3LifecycleRuleStatus, S3VersionId, StorageGcTaskId,
};
use time::{Duration, OffsetDateTime};

use crate::{Clock, RepositoryError};

pub const MAX_S3_LIFECYCLE_BATCH_SIZE: usize = 1_000;
pub const DEFAULT_S3_LIFECYCLE_BATCH_SIZE: usize = 100;
const MAX_S3_LIFECYCLE_BUCKET_PAGE_SIZE: usize = 100;
pub const DEFAULT_S3_LIFECYCLE_GC_MAX_ATTEMPTS: u32 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3CurrentExpirationCandidate {
    pub object_id: ObjectId,
    pub object_key: String,
    pub current_version_id: ObjectVersionId,
    pub version_created_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3NoncurrentExpirationCandidate {
    pub object_id: ObjectId,
    pub object_key: String,
    pub version_id: ObjectVersionId,
    pub became_noncurrent_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3ExpiredDeleteMarkerCandidate {
    pub object_id: ObjectId,
    pub object_key: String,
    pub marker_version_id: ObjectVersionId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3MultipartLifecycleCandidate {
    pub upload_id: String,
    pub object_key: String,
    pub initiated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum S3LifecycleTargetIdentity {
    Current(ObjectId, ObjectVersionId),
    Noncurrent(ObjectVersionId),
    DeleteMarker(ObjectVersionId),
    Multipart(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum S3LifecycleTarget {
    ExpireCurrent {
        object_id: ObjectId,
        object_key: String,
        expected_current_version_id: ObjectVersionId,
        version_created_at: OffsetDateTime,
    },
    ExpireNoncurrent {
        object_id: ObjectId,
        object_key: String,
        version_id: ObjectVersionId,
        expected_became_noncurrent_at: OffsetDateTime,
    },
    RemoveExpiredDeleteMarker {
        object_id: ObjectId,
        object_key: String,
        marker_version_id: ObjectVersionId,
    },
    AbortMultipart {
        upload_id: String,
        object_key: String,
        expected_initiated_at: OffsetDateTime,
    },
}

impl S3LifecycleTarget {
    #[must_use]
    pub fn identity(&self) -> S3LifecycleTargetIdentity {
        match self {
            Self::ExpireCurrent {
                object_id,
                expected_current_version_id,
                ..
            } => S3LifecycleTargetIdentity::Current(*object_id, *expected_current_version_id),
            Self::ExpireNoncurrent { version_id, .. } => {
                S3LifecycleTargetIdentity::Noncurrent(*version_id)
            }
            Self::RemoveExpiredDeleteMarker {
                marker_version_id, ..
            } => S3LifecycleTargetIdentity::DeleteMarker(*marker_version_id),
            Self::AbortMultipart { upload_id, .. } => {
                S3LifecycleTargetIdentity::Multipart(upload_id.clone())
            }
        }
    }

    #[must_use]
    pub fn object_key(&self) -> &str {
        match self {
            Self::ExpireCurrent { object_key, .. }
            | Self::ExpireNoncurrent { object_key, .. }
            | Self::RemoveExpiredDeleteMarker { object_key, .. }
            | Self::AbortMultipart { object_key, .. } => object_key,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecuteS3LifecycleCommand {
    pub application_id: ApplicationId,
    pub bucket_id: BucketId,
    pub expected_configuration_revision: u64,
    pub rule: S3LifecycleRule,
    pub target: S3LifecycleTarget,
    pub evaluated_at: OffsetDateTime,
    pub delete_marker_id: ObjectVersionId,
    pub delete_marker_version_id: S3VersionId,
    pub gc_task_id: StorageGcTaskId,
    pub gc_not_before: OffsetDateTime,
    pub gc_max_attempts: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum S3LifecycleExecutionOutcome {
    Applied,
    AlreadyApplied,
    ConfigurationChanged,
    TargetChanged,
    NotEligible,
    Locked,
    Busy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct S3LifecycleBatchCursor {
    pub after_bucket_id: Option<BucketId>,
    action_round: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct S3LifecycleBatchReceipt {
    pub examined: usize,
    pub applied: usize,
    pub skipped: usize,
    pub locked: usize,
    pub retries: usize,
    pub next_cursor: S3LifecycleBatchCursor,
}

#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3LifecycleRepository: Send + Sync {
    async fn list_s3_lifecycle_buckets(
        &self,
        after_bucket_id: Option<BucketId>,
        limit: usize,
    ) -> Result<Vec<S3Bucket>, RepositoryError>;

    async fn list_s3_current_expiration_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        created_before: Option<OffsetDateTime>,
        evaluated_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3CurrentExpirationCandidate>, RepositoryError>;

    async fn list_s3_noncurrent_expiration_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        became_noncurrent_before: OffsetDateTime,
        evaluated_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3NoncurrentExpirationCandidate>, RepositoryError>;

    async fn list_s3_expired_delete_marker_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        limit: usize,
    ) -> Result<Vec<S3ExpiredDeleteMarkerCandidate>, RepositoryError>;

    async fn list_s3_expiration_delete_marker_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        rule: &S3LifecycleRule,
        evaluated_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3ExpiredDeleteMarkerCandidate>, RepositoryError>;

    async fn list_s3_multipart_lifecycle_candidates(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
        initiated_before: OffsetDateTime,
        evaluated_at: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<S3MultipartLifecycleCandidate>, RepositoryError>;

    async fn execute_s3_lifecycle(
        &self,
        command: &ExecuteS3LifecycleCommand,
    ) -> Result<S3LifecycleExecutionOutcome, RepositoryError>;
}

pub struct S3LifecycleService<R, C> {
    repository: R,
    clock: C,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum S3LifecycleScanAction {
    CurrentExpiration,
    ExpiredDeleteMarker,
    NoncurrentExpiration,
    MultipartAbort,
}

impl<R, C> S3LifecycleService<R, C>
where
    R: S3LifecycleRepository,
    C: Clock,
{
    #[must_use]
    pub const fn new(repository: R, clock: C) -> Self {
        Self { repository, clock }
    }

    pub async fn run_batch(
        &self,
        cursor: S3LifecycleBatchCursor,
        limit: usize,
    ) -> Result<S3LifecycleBatchReceipt, RepositoryError> {
        if !(1..=MAX_S3_LIFECYCLE_BATCH_SIZE).contains(&limit) {
            return Err(RepositoryError::Invariant(
                "S3 lifecycle batch limit must be between 1 and 1000".into(),
            ));
        }
        let now = self.clock.now();
        let mut receipt = S3LifecycleBatchReceipt::default();
        let mut seen = HashSet::new();
        let mut next_cursor = cursor;

        while receipt.examined < limit {
            let page_limit = limit
                .saturating_sub(receipt.examined)
                .min(MAX_S3_LIFECYCLE_BUCKET_PAGE_SIZE);
            let buckets = self
                .repository
                .list_s3_lifecycle_buckets(next_cursor.after_bucket_id, page_limit)
                .await?;
            if buckets.is_empty() {
                if next_cursor.after_bucket_id.is_none() {
                    break;
                }
                next_cursor.after_bucket_id = None;
                next_cursor.action_round = next_cursor.action_round.wrapping_add(1);
                continue;
            }
            let reached_end = buckets.len() < page_limit;

            for bucket in &buckets {
                let configuration = bucket.configuration();
                let selected = configuration
                    .lifecycle_configuration()
                    .and_then(|lifecycle| {
                        select_scan_action(&lifecycle.rules, next_cursor.action_round)
                    });

                // A work unit is one bounded candidate query and, when it returns a
                // candidate, one fenced execution. Empty/disabled configurations still
                // consume a unit so a batch cannot walk an unbounded number of rules.
                receipt.examined += 1;
                next_cursor.after_bucket_id = Some(bucket.id());

                let Some((rule, action)) = selected else {
                    receipt.skipped += 1;
                    if receipt.examined == limit {
                        receipt.next_cursor = next_cursor;
                        return Ok(receipt);
                    }
                    continue;
                };
                let Some(target) = self.scan_action(bucket, rule, action, now).await? else {
                    if receipt.examined == limit {
                        receipt.next_cursor = next_cursor;
                        return Ok(receipt);
                    }
                    continue;
                };
                if !seen.insert(target.identity()) {
                    receipt.skipped += 1;
                    if receipt.examined == limit {
                        receipt.next_cursor = next_cursor;
                        return Ok(receipt);
                    }
                    continue;
                }

                let version_id = ObjectVersionId::new();
                let command = ExecuteS3LifecycleCommand {
                    application_id: bucket.application_id(),
                    bucket_id: bucket.id(),
                    expected_configuration_revision: configuration.revision(),
                    rule: rule.clone(),
                    target,
                    evaluated_at: now,
                    delete_marker_id: version_id,
                    delete_marker_version_id: S3VersionId::new(version_id.to_string())
                        .map_err(|error| RepositoryError::Invariant(error.to_string()))?,
                    gc_task_id: StorageGcTaskId::new(),
                    gc_not_before: now,
                    gc_max_attempts: DEFAULT_S3_LIFECYCLE_GC_MAX_ATTEMPTS,
                };
                match self.repository.execute_s3_lifecycle(&command).await {
                    Ok(S3LifecycleExecutionOutcome::Applied) => receipt.applied += 1,
                    Ok(S3LifecycleExecutionOutcome::Locked) => receipt.locked += 1,
                    Ok(
                        S3LifecycleExecutionOutcome::AlreadyApplied
                        | S3LifecycleExecutionOutcome::ConfigurationChanged
                        | S3LifecycleExecutionOutcome::TargetChanged
                        | S3LifecycleExecutionOutcome::NotEligible
                        | S3LifecycleExecutionOutcome::Busy,
                    ) => receipt.skipped += 1,
                    Err(RepositoryError::Conflict) => receipt.retries += 1,
                    Err(error) => return Err(error),
                }
                if receipt.examined == limit {
                    receipt.next_cursor = next_cursor;
                    return Ok(receipt);
                }
            }

            if reached_end {
                next_cursor.after_bucket_id = None;
                next_cursor.action_round = next_cursor.action_round.wrapping_add(1);
            }
        }
        receipt.next_cursor = next_cursor;
        Ok(receipt)
    }

    async fn scan_action(
        &self,
        bucket: &S3Bucket,
        rule: &S3LifecycleRule,
        action: S3LifecycleScanAction,
        now: OffsetDateTime,
    ) -> Result<Option<S3LifecycleTarget>, RepositoryError> {
        let prefix = lifecycle_prefix(&rule.filter);
        match action {
            S3LifecycleScanAction::CurrentExpiration => match &rule.expiration {
                Some(S3Expiration::Days(days)) => {
                    let cutoff = lifecycle_days_cutoff(now, *days)?;
                    Ok(self
                        .repository
                        .list_s3_current_expiration_candidates(
                            bucket.application_id(),
                            bucket.id(),
                            prefix,
                            Some(cutoff),
                            now,
                            1,
                        )
                        .await?
                        .into_iter()
                        .next()
                        .map(current_target))
                }
                Some(S3Expiration::Date(date)) if *date <= now => Ok(self
                    .repository
                    .list_s3_current_expiration_candidates(
                        bucket.application_id(),
                        bucket.id(),
                        prefix,
                        None,
                        now,
                        1,
                    )
                    .await?
                    .into_iter()
                    .next()
                    .map(current_target)),
                Some(S3Expiration::Date(_) | S3Expiration::ExpiredObjectDeleteMarker) | None => {
                    Ok(None)
                }
            },
            S3LifecycleScanAction::ExpiredDeleteMarker => Ok(self
                .repository
                .list_s3_expiration_delete_marker_candidates(
                    bucket.application_id(),
                    bucket.id(),
                    rule,
                    now,
                    1,
                )
                .await?
                .into_iter()
                .next()
                .map(|candidate| S3LifecycleTarget::RemoveExpiredDeleteMarker {
                    object_id: candidate.object_id,
                    object_key: candidate.object_key,
                    marker_version_id: candidate.marker_version_id,
                })),
            S3LifecycleScanAction::NoncurrentExpiration => {
                let action = rule.noncurrent_version_expiration.ok_or_else(|| {
                    RepositoryError::Invariant(
                        "selected S3 lifecycle noncurrent action is missing".into(),
                    )
                })?;
                let cutoff = lifecycle_days_cutoff(now, action.noncurrent_days)?;
                Ok(self
                    .repository
                    .list_s3_noncurrent_expiration_candidates(
                        bucket.application_id(),
                        bucket.id(),
                        prefix,
                        cutoff,
                        now,
                        1,
                    )
                    .await?
                    .into_iter()
                    .next()
                    .map(|candidate| S3LifecycleTarget::ExpireNoncurrent {
                        object_id: candidate.object_id,
                        object_key: candidate.object_key,
                        version_id: candidate.version_id,
                        expected_became_noncurrent_at: candidate.became_noncurrent_at,
                    }))
            }
            S3LifecycleScanAction::MultipartAbort => {
                let action = rule.abort_incomplete_multipart_upload.ok_or_else(|| {
                    RepositoryError::Invariant(
                        "selected S3 lifecycle multipart action is missing".into(),
                    )
                })?;
                let cutoff = lifecycle_days_cutoff(now, action.days_after_initiation)?;
                Ok(self
                    .repository
                    .list_s3_multipart_lifecycle_candidates(
                        bucket.application_id(),
                        bucket.id(),
                        prefix,
                        cutoff,
                        now,
                        1,
                    )
                    .await?
                    .into_iter()
                    .next()
                    .map(|candidate| S3LifecycleTarget::AbortMultipart {
                        upload_id: candidate.upload_id,
                        object_key: candidate.object_key,
                        expected_initiated_at: candidate.initiated_at,
                    }))
            }
        }
    }
}

fn select_scan_action(
    rules: &[S3LifecycleRule],
    action_round: usize,
) -> Option<(&S3LifecycleRule, S3LifecycleScanAction)> {
    let action_count = rules
        .iter()
        .filter(|rule| rule.status == S3LifecycleRuleStatus::Enabled)
        .map(rule_scan_action_count)
        .sum::<usize>();
    if action_count == 0 {
        return None;
    }
    let mut selected = action_round % action_count;
    for rule in rules
        .iter()
        .filter(|rule| rule.status == S3LifecycleRuleStatus::Enabled)
    {
        for action in rule_scan_actions(rule).into_iter().flatten() {
            if selected == 0 {
                return Some((rule, action));
            }
            selected -= 1;
        }
    }
    None
}

fn rule_scan_action_count(rule: &S3LifecycleRule) -> usize {
    rule_scan_actions(rule).into_iter().flatten().count()
}

fn rule_scan_actions(rule: &S3LifecycleRule) -> [Option<S3LifecycleScanAction>; 4] {
    let (current, marker) = match rule.expiration {
        Some(S3Expiration::Days(_) | S3Expiration::Date(_)) => (
            Some(S3LifecycleScanAction::CurrentExpiration),
            Some(S3LifecycleScanAction::ExpiredDeleteMarker),
        ),
        Some(S3Expiration::ExpiredObjectDeleteMarker) => {
            (None, Some(S3LifecycleScanAction::ExpiredDeleteMarker))
        }
        None => (None, None),
    };
    [
        current,
        marker,
        rule.noncurrent_version_expiration
            .map(|_| S3LifecycleScanAction::NoncurrentExpiration),
        rule.abort_incomplete_multipart_upload
            .map(|_| S3LifecycleScanAction::MultipartAbort),
    ]
}

fn current_target(candidate: S3CurrentExpirationCandidate) -> S3LifecycleTarget {
    S3LifecycleTarget::ExpireCurrent {
        object_id: candidate.object_id,
        object_key: candidate.object_key,
        expected_current_version_id: candidate.current_version_id,
        version_created_at: candidate.version_created_at,
    }
}

#[must_use]
pub fn lifecycle_prefix(filter: &S3LifecycleFilter) -> &str {
    match filter {
        S3LifecycleFilter::Empty => "",
        S3LifecycleFilter::Prefix(prefix) => prefix,
    }
}

pub fn lifecycle_days_cutoff(
    now: OffsetDateTime,
    days: u32,
) -> Result<OffsetDateTime, RepositoryError> {
    if days == 0 {
        return Err(RepositoryError::Invariant(
            "S3 lifecycle days must be positive".into(),
        ));
    }
    let start_today = now.date().midnight().assume_utc();
    start_today
        .checked_sub(Duration::days(i64::from(days)))
        .ok_or_else(|| RepositoryError::Invariant("S3 lifecycle cutoff overflow".into()))
}

#[must_use]
pub fn lifecycle_action_time(now: OffsetDateTime) -> OffsetDateTime {
    now.date().midnight().assume_utc()
}

#[must_use]
pub fn lifecycle_rule_is_current_due(
    rule: &S3LifecycleRule,
    object_key: &str,
    created_at: OffsetDateTime,
    now: OffsetDateTime,
) -> bool {
    if rule.status != S3LifecycleRuleStatus::Enabled
        || !object_key.starts_with(lifecycle_prefix(&rule.filter))
    {
        return false;
    }
    match rule.expiration {
        Some(S3Expiration::Days(days)) => {
            lifecycle_days_cutoff(now, days).is_ok_and(|cutoff| created_at < cutoff)
        }
        Some(S3Expiration::Date(date)) => date <= now,
        _ => false,
    }
}

#[must_use]
pub fn lifecycle_rule_is_noncurrent_due(
    rule: &S3LifecycleRule,
    object_key: &str,
    became_noncurrent_at: OffsetDateTime,
    now: OffsetDateTime,
) -> bool {
    rule.status == S3LifecycleRuleStatus::Enabled
        && object_key.starts_with(lifecycle_prefix(&rule.filter))
        && rule.noncurrent_version_expiration.is_some_and(|action| {
            lifecycle_days_cutoff(now, action.noncurrent_days)
                .is_ok_and(|cutoff| became_noncurrent_at < cutoff)
        })
}

#[must_use]
pub fn lifecycle_rule_removes_expired_marker(rule: &S3LifecycleRule, object_key: &str) -> bool {
    rule.status == S3LifecycleRuleStatus::Enabled
        && object_key.starts_with(lifecycle_prefix(&rule.filter))
        && rule.expiration == Some(S3Expiration::ExpiredObjectDeleteMarker)
}

#[must_use]
pub fn lifecycle_rule_removes_expired_marker_at(
    rule: &S3LifecycleRule,
    object_key: &str,
    marker_created_at: OffsetDateTime,
    now: OffsetDateTime,
) -> bool {
    lifecycle_rule_removes_expired_marker(rule, object_key)
        || lifecycle_rule_is_current_due(rule, object_key, marker_created_at, now)
}

#[must_use]
pub fn lifecycle_rule_aborts_multipart(
    rule: &S3LifecycleRule,
    object_key: &str,
    initiated_at: OffsetDateTime,
    now: OffsetDateTime,
) -> bool {
    rule.status == S3LifecycleRuleStatus::Enabled
        && object_key.starts_with(lifecycle_prefix(&rule.filter))
        && rule
            .abort_incomplete_multipart_upload
            .is_some_and(|action| {
                lifecycle_days_cutoff(now, action.days_after_initiation)
                    .is_ok_and(|cutoff| initiated_at < cutoff)
            })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use futures::executor::block_on;
    use mediahub_core::{
        BucketS3Configuration, PersistedBucketS3Configuration, PersistedS3Bucket,
        S3AbortIncompleteMultipartUpload, S3LifecycleConfiguration, S3NoncurrentVersionExpiration,
    };

    use super::*;
    use crate::FixedClock;

    fn lifecycle_rule(status: S3LifecycleRuleStatus, prefix: &str) -> S3LifecycleRule {
        S3LifecycleRule {
            id: Some("expire-tmp".into()),
            status,
            filter: S3LifecycleFilter::Prefix(prefix.into()),
            expiration: Some(S3Expiration::Days(1)),
            noncurrent_version_expiration: Some(S3NoncurrentVersionExpiration {
                noncurrent_days: 2,
            }),
            abort_incomplete_multipart_upload: Some(S3AbortIncompleteMultipartUpload {
                days_after_initiation: 3,
            }),
        }
    }

    fn lifecycle_bucket(now: OffsetDateTime, rule: S3LifecycleRule) -> S3Bucket {
        let mut configuration =
            BucketS3Configuration::new("us-east-1", false, None, None, now).expect("configuration");
        configuration
            .replace_lifecycle_configuration(
                Some(S3LifecycleConfiguration::new(vec![rule]).expect("lifecycle")),
                now,
            )
            .expect("replace lifecycle");
        S3Bucket::from_persistence(PersistedS3Bucket {
            id: BucketId::new(),
            application_id: ApplicationId::new(),
            name: "lifecycle-test".into(),
            configuration: PersistedBucketS3Configuration {
                region: configuration.region().into(),
                versioning_status: configuration.versioning_status(),
                object_lock_enabled: configuration.object_lock_enabled(),
                default_retention: configuration.default_retention(),
                lifecycle_configuration: configuration.lifecycle_configuration().cloned(),
                revision: configuration.revision(),
                updated_at: configuration.updated_at(),
            },
            created_at: now,
        })
        .expect("bucket")
    }

    #[test]
    fn utc_day_boundaries_and_prefix_status_are_table_driven() {
        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(10) + Duration::hours(12);
        let enabled = lifecycle_rule(S3LifecycleRuleStatus::Enabled, "tmp/");
        let disabled = lifecycle_rule(S3LifecycleRuleStatus::Disabled, "tmp/");
        let cases = [
            ("tmp/old", now - Duration::days(2), true),
            ("tmp/not-old-enough", now - Duration::days(1), false),
            ("tmp/midnight", now.date().midnight().assume_utc(), false),
            ("other/old", now - Duration::days(2), false),
        ];
        for (key, created_at, expected) in cases {
            assert_eq!(
                lifecycle_rule_is_current_due(&enabled, key, created_at, now),
                expected,
                "case {key}"
            );
            assert!(!lifecycle_rule_is_current_due(
                &disabled, key, created_at, now
            ));
        }
        assert_eq!(
            lifecycle_days_cutoff(now, 1).expect("cutoff"),
            now.date().midnight().assume_utc() - Duration::days(1)
        );
        assert_eq!(
            lifecycle_days_cutoff(now, 2).expect("cutoff"),
            now.date().midnight().assume_utc() - Duration::days(2)
        );
        let mut date_rule = enabled.clone();
        date_rule.expiration = Some(S3Expiration::Date(now.date().midnight().assume_utc()));
        assert!(lifecycle_rule_is_current_due(
            &date_rule,
            "tmp/created-after-date",
            now,
            now
        ));
        date_rule.expiration = Some(S3Expiration::Date(
            now.date().midnight().assume_utc() + Duration::days(1),
        ));
        assert!(!lifecycle_rule_is_current_due(
            &date_rule,
            "tmp/not-due",
            now - Duration::days(10),
            now
        ));
        let mut explicit_marker_rule = enabled;
        explicit_marker_rule.expiration = Some(S3Expiration::ExpiredObjectDeleteMarker);
        assert!(lifecycle_rule_removes_expired_marker_at(
            &explicit_marker_rule,
            "tmp/marker",
            now,
            now
        ));
    }

    #[derive(Clone)]
    struct FakeLifecycleRepository {
        buckets: Vec<S3Bucket>,
        current: Vec<(BucketId, S3CurrentExpirationCandidate)>,
        noncurrent: Vec<(BucketId, S3NoncurrentExpirationCandidate)>,
        marker: Vec<(BucketId, S3ExpiredDeleteMarkerCandidate)>,
        multipart: Vec<(BucketId, S3MultipartLifecycleCandidate)>,
        scans: Arc<Mutex<Vec<(BucketId, S3LifecycleScanAction)>>>,
        executed: Arc<Mutex<Vec<ExecuteS3LifecycleCommand>>>,
    }

    impl FakeLifecycleRepository {
        fn empty(buckets: Vec<S3Bucket>) -> Self {
            Self {
                buckets,
                current: Vec::new(),
                noncurrent: Vec::new(),
                marker: Vec::new(),
                multipart: Vec::new(),
                scans: Arc::new(Mutex::new(Vec::new())),
                executed: Arc::new(Mutex::new(Vec::new())),
            }
        }

        fn record_scan(&self, bucket_id: BucketId, action: S3LifecycleScanAction) {
            self.scans
                .lock()
                .expect("scan lock")
                .push((bucket_id, action));
        }
    }

    #[async_trait]
    impl S3LifecycleRepository for FakeLifecycleRepository {
        async fn list_s3_lifecycle_buckets(
            &self,
            after_bucket_id: Option<BucketId>,
            limit: usize,
        ) -> Result<Vec<S3Bucket>, RepositoryError> {
            let mut buckets = self
                .buckets
                .iter()
                .filter(|bucket| after_bucket_id.is_none_or(|after| bucket.id() > after))
                .cloned()
                .collect::<Vec<_>>();
            buckets.sort_by_key(S3Bucket::id);
            buckets.truncate(limit);
            Ok(buckets)
        }

        async fn list_s3_current_expiration_candidates(
            &self,
            application_id: ApplicationId,
            bucket_id: BucketId,
            _prefix: &str,
            _created_before: Option<OffsetDateTime>,
            _evaluated_at: OffsetDateTime,
            limit: usize,
        ) -> Result<Vec<S3CurrentExpirationCandidate>, RepositoryError> {
            assert!(self.buckets.iter().any(|bucket| {
                bucket.application_id() == application_id && bucket.id() == bucket_id
            }));
            self.record_scan(bucket_id, S3LifecycleScanAction::CurrentExpiration);
            Ok(self
                .current
                .iter()
                .filter(|(candidate_bucket_id, _)| *candidate_bucket_id == bucket_id)
                .map(|(_, candidate)| candidate.clone())
                .take(limit)
                .collect())
        }

        async fn list_s3_noncurrent_expiration_candidates(
            &self,
            application_id: ApplicationId,
            bucket_id: BucketId,
            _prefix: &str,
            _became_noncurrent_before: OffsetDateTime,
            _evaluated_at: OffsetDateTime,
            limit: usize,
        ) -> Result<Vec<S3NoncurrentExpirationCandidate>, RepositoryError> {
            assert!(self.buckets.iter().any(|bucket| {
                bucket.application_id() == application_id && bucket.id() == bucket_id
            }));
            self.record_scan(bucket_id, S3LifecycleScanAction::NoncurrentExpiration);
            Ok(self
                .noncurrent
                .iter()
                .filter(|(candidate_bucket_id, _)| *candidate_bucket_id == bucket_id)
                .map(|(_, candidate)| candidate.clone())
                .take(limit)
                .collect())
        }

        async fn list_s3_expired_delete_marker_candidates(
            &self,
            application_id: ApplicationId,
            bucket_id: BucketId,
            _prefix: &str,
            limit: usize,
        ) -> Result<Vec<S3ExpiredDeleteMarkerCandidate>, RepositoryError> {
            assert!(self.buckets.iter().any(|bucket| {
                bucket.application_id() == application_id && bucket.id() == bucket_id
            }));
            self.record_scan(bucket_id, S3LifecycleScanAction::ExpiredDeleteMarker);
            Ok(self
                .marker
                .iter()
                .filter(|(candidate_bucket_id, _)| *candidate_bucket_id == bucket_id)
                .map(|(_, candidate)| candidate.clone())
                .take(limit)
                .collect())
        }

        async fn list_s3_expiration_delete_marker_candidates(
            &self,
            application_id: ApplicationId,
            bucket_id: BucketId,
            rule: &S3LifecycleRule,
            _evaluated_at: OffsetDateTime,
            limit: usize,
        ) -> Result<Vec<S3ExpiredDeleteMarkerCandidate>, RepositoryError> {
            self.list_s3_expired_delete_marker_candidates(
                application_id,
                bucket_id,
                lifecycle_prefix(&rule.filter),
                limit,
            )
            .await
        }

        async fn list_s3_multipart_lifecycle_candidates(
            &self,
            application_id: ApplicationId,
            bucket_id: BucketId,
            _prefix: &str,
            _initiated_before: OffsetDateTime,
            _evaluated_at: OffsetDateTime,
            limit: usize,
        ) -> Result<Vec<S3MultipartLifecycleCandidate>, RepositoryError> {
            assert!(self.buckets.iter().any(|bucket| {
                bucket.application_id() == application_id && bucket.id() == bucket_id
            }));
            self.record_scan(bucket_id, S3LifecycleScanAction::MultipartAbort);
            Ok(self
                .multipart
                .iter()
                .filter(|(candidate_bucket_id, _)| *candidate_bucket_id == bucket_id)
                .map(|(_, candidate)| candidate.clone())
                .take(limit)
                .collect())
        }

        async fn execute_s3_lifecycle(
            &self,
            command: &ExecuteS3LifecycleCommand,
        ) -> Result<S3LifecycleExecutionOutcome, RepositoryError> {
            self.executed
                .lock()
                .expect("executed lock")
                .push(command.clone());
            Ok(S3LifecycleExecutionOutcome::Applied)
        }
    }

    #[test]
    fn fake_clock_drives_a_bounded_current_expiration_batch() {
        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(10) + Duration::hours(12);
        let bucket = lifecycle_bucket(now, lifecycle_rule(S3LifecycleRuleStatus::Enabled, "tmp/"));
        let current = S3CurrentExpirationCandidate {
            object_id: ObjectId::new(),
            object_key: "tmp/old.bin".into(),
            current_version_id: ObjectVersionId::new(),
            version_created_at: now - Duration::days(2),
        };
        let executed = Arc::new(Mutex::new(Vec::new()));
        let repository = FakeLifecycleRepository {
            buckets: vec![bucket.clone()],
            current: vec![(bucket.id(), current.clone())],
            noncurrent: Vec::new(),
            marker: Vec::new(),
            multipart: Vec::new(),
            scans: Arc::new(Mutex::new(Vec::new())),
            executed: Arc::clone(&executed),
        };
        let receipt = block_on(
            S3LifecycleService::new(repository, FixedClock::new(now))
                .run_batch(S3LifecycleBatchCursor::default(), 1),
        )
        .expect("batch");

        assert_eq!(receipt.examined, 1);
        assert_eq!(receipt.applied, 1);
        let commands = executed.lock().expect("executed lock");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].evaluated_at, now);
        assert_eq!(
            commands[0].expected_configuration_revision,
            bucket.configuration().revision()
        );
        assert_eq!(
            commands[0].target.identity(),
            S3LifecycleTargetIdentity::Current(current.object_id, current.current_version_id)
        );
    }

    #[test]
    fn empty_scans_consume_budget_and_action_rounds_are_fair() {
        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(10) + Duration::hours(12);
        let bucket = lifecycle_bucket(now, lifecycle_rule(S3LifecycleRuleStatus::Enabled, "tmp/"));
        let repository = FakeLifecycleRepository::empty(vec![bucket]);
        let scans = Arc::clone(&repository.scans);
        let service = S3LifecycleService::new(repository, FixedClock::new(now));

        let first = block_on(service.run_batch(S3LifecycleBatchCursor::default(), 3))
            .expect("first empty batch");
        assert_eq!(first.examined, 3);
        assert_eq!(first.applied, 0);
        assert_eq!(first.skipped, 0);
        let second = block_on(service.run_batch(first.next_cursor, 2)).expect("second empty batch");
        assert_eq!(second.examined, 2);

        let actions = scans
            .lock()
            .expect("scan lock")
            .iter()
            .map(|(_, action)| *action)
            .collect::<Vec<_>>();
        assert_eq!(
            actions,
            vec![
                S3LifecycleScanAction::CurrentExpiration,
                S3LifecycleScanAction::ExpiredDeleteMarker,
                S3LifecycleScanAction::NoncurrentExpiration,
                S3LifecycleScanAction::MultipartAbort,
                S3LifecycleScanAction::CurrentExpiration,
            ]
        );
    }

    #[test]
    fn current_candidates_do_not_starve_other_actions_or_later_buckets() {
        let now = OffsetDateTime::UNIX_EPOCH + Duration::days(10) + Duration::hours(12);
        let first_bucket =
            lifecycle_bucket(now, lifecycle_rule(S3LifecycleRuleStatus::Enabled, "tmp/"));
        let second_bucket =
            lifecycle_bucket(now, lifecycle_rule(S3LifecycleRuleStatus::Enabled, "tmp/"));
        let mut repository =
            FakeLifecycleRepository::empty(vec![first_bucket.clone(), second_bucket.clone()]);
        for bucket in [&first_bucket, &second_bucket] {
            repository.current.push((
                bucket.id(),
                S3CurrentExpirationCandidate {
                    object_id: ObjectId::new(),
                    object_key: "tmp/current.bin".into(),
                    current_version_id: ObjectVersionId::new(),
                    version_created_at: now - Duration::days(2),
                },
            ));
            repository.noncurrent.push((
                bucket.id(),
                S3NoncurrentExpirationCandidate {
                    object_id: ObjectId::new(),
                    object_key: "tmp/noncurrent.bin".into(),
                    version_id: ObjectVersionId::new(),
                    became_noncurrent_at: now - Duration::days(3),
                },
            ));
            repository.multipart.push((
                bucket.id(),
                S3MultipartLifecycleCandidate {
                    upload_id: format!("multipart-{}", bucket.id()),
                    object_key: "tmp/multipart.bin".into(),
                    initiated_at: now - Duration::days(4),
                },
            ));
        }
        let executed = Arc::clone(&repository.executed);
        let service = S3LifecycleService::new(repository, FixedClock::new(now));
        let mut cursor = S3LifecycleBatchCursor::default();

        let current_round = block_on(service.run_batch(cursor, 2)).expect("current round");
        cursor = current_round.next_cursor;
        assert_eq!(current_round.applied, 2);
        let current_buckets = executed
            .lock()
            .expect("executed lock")
            .iter()
            .map(|command| command.bucket_id)
            .collect::<HashSet<_>>();
        assert_eq!(current_buckets.len(), 2);

        for _ in 0..3 {
            cursor = block_on(service.run_batch(cursor, 2))
                .expect("next fair round")
                .next_cursor;
        }
        let scans = service
            .repository
            .scans
            .lock()
            .expect("scan lock")
            .iter()
            .map(|(_, action)| *action)
            .collect::<HashSet<_>>();
        assert!(scans.contains(&S3LifecycleScanAction::CurrentExpiration));
        assert!(scans.contains(&S3LifecycleScanAction::NoncurrentExpiration));
        assert!(scans.contains(&S3LifecycleScanAction::MultipartAbort));
        let executed = executed.lock().expect("executed lock");
        assert!(
            executed.iter().any(|command| matches!(
                &command.target,
                S3LifecycleTarget::ExpireNoncurrent { .. }
            ))
        );
        assert!(
            executed
                .iter()
                .any(|command| matches!(&command.target, S3LifecycleTarget::AbortMultipart { .. }))
        );
    }
}
