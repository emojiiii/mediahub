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
