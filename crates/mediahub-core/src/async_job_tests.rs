// Async-job domain tests.

    use time::Duration;

    use super::*;

    fn new_job(max_attempts: u32) -> AsyncJob {
        AsyncJob::new(
            NewAsyncJob {
                id: AsyncJobId::new(),
                application_id: ApplicationId::new(),
                operation_scope: "media.batch".to_owned(),
                idempotency_key: "batch-2026-07-15".to_owned(),
                request_hash: "a".repeat(64),
                request_id: Some("request-1".to_owned()),
                action: AsyncJobAction::Delete,
                total_items: 2,
                max_attempts,
            },
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("valid job")
    }

    #[test]
    fn action_serialization_is_stable() {
        let action = AsyncJobAction::UpdateVisibility {
            visibility: Visibility::Private,
        };
        let encoded = serde_json::to_value(action).expect("serialize action");
        assert_eq!(
            encoded,
            serde_json::json!({"type": "update_visibility", "visibility": "private"})
        );
    }

    #[test]
    fn retry_then_partial_success_completion() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut job = new_job(2);
        job.claim("lease-1", now + Duration::seconds(30), now)
            .expect("first claim");
        let disposition = job
            .fail(
                "lease-1",
                "temporary storage outage",
                Some(now + Duration::seconds(10)),
                now + Duration::seconds(1),
            )
            .expect("schedule retry");
        assert!(matches!(
            disposition,
            AsyncJobFailureDisposition::RetryScheduled { .. }
        ));
        assert!(!job.is_claimable_at(now + Duration::seconds(9)));

        job.claim(
            "lease-2",
            now + Duration::seconds(40),
            now + Duration::seconds(10),
        )
        .expect("retry claim");
        job.complete("lease-2", 1, 1, now + Duration::seconds(11))
            .expect("partial item failure still completes the job");
        assert_eq!(job.state(), AsyncJobState::Completed);
        assert_eq!(job.succeeded_items(), 1);
        assert_eq!(job.failed_items(), 1);
    }

    #[test]
    fn stale_worker_cannot_acknowledge_reclaimed_job() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut job = new_job(2);
        job.claim("old", now + Duration::seconds(5), now)
            .expect("initial claim");
        job.claim(
            "new",
            now + Duration::seconds(15),
            now + Duration::seconds(5),
        )
        .expect("expired lease reclaimed");
        assert_eq!(
            job.complete("old", 2, 0, now + Duration::seconds(6)),
            Err(AsyncJobError::StaleLease)
        );
    }

    #[test]
    fn final_attempt_failure_is_terminal() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut job = new_job(1);
        job.claim("only-attempt", now + Duration::seconds(30), now)
            .expect("claim");
        assert_eq!(
            job.fail(
                "only-attempt",
                "permanent failure",
                None,
                now + Duration::seconds(1)
            ),
            Ok(AsyncJobFailureDisposition::Terminal)
        );
        assert_eq!(job.state(), AsyncJobState::Failed);
        assert!(!job.is_claimable_at(now + Duration::hours(1)));
    }

    #[test]
    fn cancellation_is_idempotent() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut job = new_job(3);
        assert_eq!(job.cancel(now), Ok(AsyncJobTransition::Applied));
        assert_eq!(
            job.cancel(now + Duration::seconds(1)),
            Ok(AsyncJobTransition::AlreadyApplied)
        );
    }

    #[test]
    fn persistence_round_trip_preserves_running_lease() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut job = new_job(3);
        job.claim("lease", now + Duration::seconds(30), now)
            .expect("claim");
        let restored = AsyncJob::from_persistence(job.to_persisted()).expect("restore");
        assert_eq!(restored, job);
    }

    #[test]
    fn lease_duration_and_batch_limits_are_enforced() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut job = new_job(2);
        assert_eq!(
            job.claim("too-long", now + Duration::hours(2), now),
            Err(AsyncJobError::LeaseTooLong)
        );

        let mut input = NewAsyncJob {
            id: AsyncJobId::new(),
            application_id: ApplicationId::new(),
            operation_scope: "media.batch".to_owned(),
            idempotency_key: "key".to_owned(),
            request_hash: "a".repeat(64),
            request_id: None,
            action: AsyncJobAction::Delete,
            total_items: MAX_ASYNC_JOB_ITEMS + 1,
            max_attempts: 1,
        };
        assert_eq!(
            AsyncJob::new(input.clone(), now),
            Err(AsyncJobError::TooManyItems)
        );
        input.total_items = 1;
        input.max_attempts = MAX_ASYNC_JOB_ATTEMPTS + 1;
        assert_eq!(
            AsyncJob::new(input, now),
            Err(AsyncJobError::TooManyAttempts)
        );
    }

    #[test]
    fn public_job_serialization_omits_internal_lease_state() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut job = new_job(2);
        job.claim("private-lease-token", now + Duration::seconds(30), now)
            .expect("claim job");

        let encoded = serde_json::to_value(job).expect("serialize job");
        assert!(encoded.get("lease_token").is_none());
        assert!(encoded.get("leased_until").is_none());
    }

    #[test]
    fn invalid_persisted_state_is_rejected() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut persisted = new_job(2).to_persisted();
        persisted.state = AsyncJobState::Completed;
        persisted.completed_at = Some(now + Duration::seconds(1));

        assert_eq!(
            AsyncJob::from_persistence(persisted),
            Err(AsyncJobError::InvalidPersistedJob)
        );
    }

    #[test]
    fn persisted_terminal_timestamps_are_mutually_exclusive() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let terminal_at = now + Duration::seconds(1);

        let mut completed = new_job(1);
        completed
            .claim("completed-lease", now + Duration::seconds(30), now)
            .expect("claim completed job");
        completed
            .complete("completed-lease", 2, 0, terminal_at)
            .expect("complete job");
        let completed = completed.to_persisted();
        assert!(AsyncJob::from_persistence(completed.clone()).is_ok());
        for invalid in [
            PersistedAsyncJob {
                failed_at: Some(terminal_at),
                ..completed.clone()
            },
            PersistedAsyncJob {
                cancelled_at: Some(terminal_at),
                ..completed
            },
        ] {
            assert_eq!(
                AsyncJob::from_persistence(invalid),
                Err(AsyncJobError::InvalidPersistedJob)
            );
        }

        let mut failed = new_job(1);
        failed
            .claim("failed-lease", now + Duration::seconds(30), now)
            .expect("claim failed job");
        failed
            .fail("failed-lease", "terminal failure", None, terminal_at)
            .expect("fail job");
        let failed = failed.to_persisted();
        assert!(AsyncJob::from_persistence(failed.clone()).is_ok());
        for invalid in [
            PersistedAsyncJob {
                completed_at: Some(terminal_at),
                ..failed.clone()
            },
            PersistedAsyncJob {
                cancelled_at: Some(terminal_at),
                ..failed
            },
        ] {
            assert_eq!(
                AsyncJob::from_persistence(invalid),
                Err(AsyncJobError::InvalidPersistedJob)
            );
        }

        let mut cancelled = new_job(1);
        cancelled.cancel(terminal_at).expect("cancel job");
        let cancelled = cancelled.to_persisted();
        assert!(AsyncJob::from_persistence(cancelled.clone()).is_ok());
        for invalid in [
            PersistedAsyncJob {
                completed_at: Some(terminal_at),
                ..cancelled.clone()
            },
            PersistedAsyncJob {
                failed_at: Some(terminal_at),
                ..cancelled
            },
        ] {
            assert_eq!(
                AsyncJob::from_persistence(invalid),
                Err(AsyncJobError::InvalidPersistedJob)
            );
        }
    }

    #[test]
    fn persisted_running_lease_time_relationships_are_enforced() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut running = new_job(2);
        running
            .claim("lease", now + Duration::seconds(30), now)
            .expect("claim job");
        let running = running.to_persisted();
        assert!(AsyncJob::from_persistence(running.clone()).is_ok());

        for invalid in [
            PersistedAsyncJob {
                leased_until: Some(running.updated_at),
                ..running.clone()
            },
            PersistedAsyncJob {
                leased_until: Some(running.updated_at - Duration::seconds(1)),
                ..running.clone()
            },
            PersistedAsyncJob {
                leased_until: Some(
                    running.updated_at
                        + Duration::seconds(MAX_ASYNC_JOB_LEASE_SECONDS + 1),
                ),
                ..running
            },
        ] {
            assert_eq!(
                AsyncJob::from_persistence(invalid),
                Err(AsyncJobError::InvalidPersistedJob)
            );
        }
    }

    #[test]
    fn active_lease_can_be_renewed_with_same_fencing_token() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut job = new_job(2);
        job.claim("lease", now + Duration::seconds(30), now)
            .expect("claim");
        job.renew_lease("lease", now + Duration::seconds(60), now + Duration::seconds(1))
            .expect("renew");
        assert_eq!(job.leased_until(), Some(now + Duration::seconds(60)));
        assert_eq!(
            job.renew_lease("stale", now + Duration::seconds(90), now + Duration::seconds(2)),
            Err(AsyncJobError::StaleLease)
        );
    }

    #[test]
    fn async_job_debug_output_redacts_idempotency_and_lease_secrets() {
        let now = OffsetDateTime::UNIX_EPOCH;
        let mut job = new_job(2);
        job.claim("lease-secret", now + Duration::seconds(30), now)
            .expect("claim");
        let debug = format!("{job:?}");

        assert!(!debug.contains("batch-2026-07-15"));
        assert!(!debug.contains("lease-secret"));
        assert!(debug.contains("<redacted>"));
    }
