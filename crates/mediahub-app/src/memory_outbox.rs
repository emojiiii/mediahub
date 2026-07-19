// In-memory outbox repository and clock.

#[async_trait]
impl OutboxRepository for InMemoryMediaRepository {
    async fn list_pending(
        &self,
        now: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<OutboxEvent>, RepositoryError> {
        let state = self.lock()?;
        Ok(state
            .outbox
            .values()
            .filter(|event| {
                event.delivered_at.is_none()
                    && event.next_attempt_at.is_none_or(|retry_at| retry_at <= now)
            })
            .take(limit)
            .cloned()
            .collect())
    }

    async fn mark_delivered(
        &self,
        event_id: &str,
        delivered_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let mut state = self.lock()?;
        let event = state
            .outbox
            .get_mut(event_id)
            .ok_or(RepositoryError::NotFound)?;
        event.delivered_at = Some(delivered_at);
        event.next_attempt_at = None;
        Ok(())
    }

    async fn mark_failed(
        &self,
        event_id: &str,
        retry_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let mut state = self.lock()?;
        let event = state
            .outbox
            .get_mut(event_id)
            .ok_or(RepositoryError::NotFound)?;
        event.attempt_count = event.attempt_count.saturating_add(1);
        event.next_attempt_at = Some(retry_at);
        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FixedClock {
    now: OffsetDateTime,
}

impl FixedClock {
    #[must_use]
    pub const fn new(now: OffsetDateTime) -> Self {
        Self { now }
    }
}

impl Clock for FixedClock {
    fn now(&self) -> OffsetDateTime {
        self.now
    }
}
