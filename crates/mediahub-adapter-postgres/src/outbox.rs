use async_trait::async_trait;
use mediahub_app::{
    LeasedWebhookDelivery, OutboxEvent, OutboxRepository, RepositoryError, WebhookDelivery,
    WebhookDeliveryEndpoint, WebhookDeliveryFailureDisposition, WebhookDeliveryRepository,
};
use mediahub_core::{ApplicationId, OffsetDateTime};
use sqlx::{Postgres, Row, Transaction, types::Json};
use uuid::Uuid;

use crate::{
    PostgresRepository,
    codec::{as_i64, as_u32, database_error, row_to_outbox},
};

#[derive(Clone, Debug, PartialEq)]
pub struct LeasedOutboxEvent {
    pub event: OutboxEvent,
    pub lease_token: String,
    pub leased_until: OffsetDateTime,
}

impl PostgresRepository {
    /// Claims events with row locks that skip work already held by another
    /// PostgreSQL worker. The returned token fences acknowledgements.
    pub async fn claim_outbox_events(
        &self,
        now: OffsetDateTime,
        leased_until: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<LeasedOutboxEvent>, RepositoryError> {
        validate_claim(now, leased_until, limit)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let lease_token = Uuid::new_v4();
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let rows = sqlx::query(
            "WITH claimable AS ( \
                 SELECT id FROM outbox_events \
                 WHERE delivered_at IS NULL AND available_at <= $1 \
                   AND (leased_until IS NULL OR leased_until <= $1) \
                 ORDER BY available_at, id FOR UPDATE SKIP LOCKED LIMIT $2 \
             ) \
             UPDATE outbox_events AS event SET leased_until = $3, lease_token = $4 \
             FROM claimable WHERE event.id = claimable.id RETURNING event.*",
        )
        .bind(now)
        .bind(as_i64(limit as u64)?)
        .bind(leased_until)
        .bind(lease_token)
        .fetch_all(&mut *transaction)
        .await
        .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)?;
        rows.into_iter()
            .map(|row| {
                Ok(LeasedOutboxEvent {
                    event: row_to_outbox(row)?,
                    lease_token: lease_token.to_string(),
                    leased_until,
                })
            })
            .collect()
    }

    pub async fn mark_outbox_delivered(
        &self,
        event_id: &str,
        lease_token: &str,
        delivered_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        let lease_token = parse_lease_token(lease_token)?;
        let result = sqlx::query(
            "UPDATE outbox_events SET delivered_at = $1, leased_until = NULL, lease_token = NULL \
             WHERE id = $2 AND delivered_at IS NULL AND lease_token = $3 AND leased_until > $1",
        )
        .bind(delivered_at)
        .bind(event_id)
        .bind(lease_token)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn record_outbox_failure(
        &self,
        event_id: &str,
        lease_token: &str,
        failed_at: OffsetDateTime,
        retry_at: OffsetDateTime,
        last_error: &str,
    ) -> Result<bool, RepositoryError> {
        if retry_at <= failed_at || last_error.is_empty() {
            return Err(RepositoryError::Invariant(
                "outbox retry and error summary are invalid".into(),
            ));
        }
        let lease_token = parse_lease_token(lease_token)?;
        let result = sqlx::query(
            "UPDATE outbox_events SET attempts = attempts + 1, available_at = $1, \
                 leased_until = NULL, lease_token = NULL, last_error = $2 \
             WHERE id = $3 AND delivered_at IS NULL AND lease_token = $4 \
               AND leased_until > $5",
        )
        .bind(retry_at)
        .bind(last_error)
        .bind(event_id)
        .bind(lease_token)
        .bind(failed_at)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }
}

#[async_trait]
impl OutboxRepository for PostgresRepository {
    async fn list_pending(
        &self,
        now: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<OutboxEvent>, RepositoryError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT * FROM outbox_events WHERE delivered_at IS NULL AND available_at <= $1 \
             AND (leased_until IS NULL OR leased_until <= $1) \
             ORDER BY available_at, id LIMIT $2",
        )
        .bind(now)
        .bind(as_i64(limit as u64)?)
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_outbox).collect()
    }

    async fn mark_delivered(
        &self,
        event_id: &str,
        delivered_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let result = sqlx::query(
            "UPDATE outbox_events SET delivered_at = $1, leased_until = NULL, lease_token = NULL \
             WHERE id = $2",
        )
        .bind(delivered_at)
        .bind(event_id)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        affected(result.rows_affected())
    }

    async fn mark_failed(
        &self,
        event_id: &str,
        retry_at: OffsetDateTime,
    ) -> Result<(), RepositoryError> {
        let result = sqlx::query(
            "UPDATE outbox_events SET attempts = attempts + 1, available_at = $1, \
             leased_until = NULL, lease_token = NULL WHERE id = $2",
        )
        .bind(retry_at)
        .bind(event_id)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        affected(result.rows_affected())
    }
}

#[async_trait]
impl WebhookDeliveryRepository for PostgresRepository {
    async fn materialize_webhook_deliveries(&self, event_id: &str) -> Result<u64, RepositoryError> {
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let created = materialize_deliveries(&mut transaction, event_id).await?;
        transaction.commit().await.map_err(database_error)?;
        Ok(created)
    }

    async fn finalize_unsubscribed_outbox_events(
        &self,
        limit: usize,
    ) -> Result<u64, RepositoryError> {
        if limit == 0 {
            return Ok(0);
        }
        let result = sqlx::query(
            "WITH finalizable AS ( \
                 SELECT event.id FROM outbox_events AS event \
                 WHERE event.delivered_at IS NULL \
                   AND NOT EXISTS (SELECT 1 FROM webhook_deliveries AS delivery \
                                   WHERE delivery.event_id = event.id) \
                 ORDER BY event.created_at, event.id \
                 FOR UPDATE SKIP LOCKED LIMIT $1 \
             ) \
             UPDATE outbox_events AS event SET delivered_at = event.created_at, \
                    leased_until = NULL, lease_token = NULL \
             FROM finalizable WHERE event.id = finalizable.id",
        )
        .bind(as_i64(limit as u64)?)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected())
    }

    async fn claim_webhook_deliveries(
        &self,
        now: OffsetDateTime,
        lease_until: OffsetDateTime,
        limit: usize,
    ) -> Result<Vec<LeasedWebhookDelivery>, RepositoryError> {
        validate_claim(now, lease_until, limit)?;
        if limit == 0 {
            return Ok(Vec::new());
        }
        let lease_token = Uuid::new_v4();
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        sqlx::query(
            "WITH claimable AS ( \
                 SELECT delivery.event_id, delivery.endpoint_id \
                 FROM webhook_deliveries AS delivery \
                 JOIN webhook_endpoints AS endpoint ON endpoint.id = delivery.endpoint_id \
                 WHERE endpoint.enabled = TRUE AND delivery.delivered_at IS NULL \
                   AND delivery.dead_lettered_at IS NULL AND delivery.next_attempt_at <= $1 \
                   AND (delivery.leased_until IS NULL OR delivery.leased_until <= $1) \
                 ORDER BY delivery.next_attempt_at, delivery.event_id, delivery.endpoint_id \
                 FOR UPDATE OF delivery SKIP LOCKED LIMIT $2 \
             ) \
             UPDATE webhook_deliveries AS delivery \
             SET leased_until = $3, lease_token = $4, updated_at = $1 \
             FROM claimable WHERE delivery.event_id = claimable.event_id \
               AND delivery.endpoint_id = claimable.endpoint_id",
        )
        .bind(now)
        .bind(as_i64(limit as u64)?)
        .bind(lease_until)
        .bind(lease_token)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        let rows = sqlx::query(DELIVERY_SELECT)
            .bind(lease_token)
            .fetch_all(&mut *transaction)
            .await
            .map_err(database_error)?;
        transaction.commit().await.map_err(database_error)?;
        rows.into_iter()
            .map(|row| row_to_delivery(row, lease_token, lease_until))
            .collect()
    }

    async fn mark_webhook_delivery_delivered(
        &self,
        event_id: &str,
        endpoint_id: &str,
        lease_token: &str,
        delivered_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        <Self as WebhookDeliveryRepository>::mark_webhook_delivery_delivered_with_status(
            self,
            event_id,
            endpoint_id,
            lease_token,
            delivered_at,
            None,
        )
        .await
    }

    async fn mark_webhook_delivery_delivered_with_status(
        &self,
        event_id: &str,
        endpoint_id: &str,
        lease_token: &str,
        delivered_at: OffsetDateTime,
        response_status: Option<u16>,
    ) -> Result<bool, RepositoryError> {
        validate_response_status(response_status)?;
        let lease_token = parse_lease_token(lease_token)?;
        let mut transaction = self.pool.begin().await.map_err(database_error)?;
        let result = sqlx::query(
            "UPDATE webhook_deliveries SET delivered_at = $1, next_attempt_at = NULL, \
                 leased_until = NULL, lease_token = NULL, last_error = NULL, \
                 last_response_status = $2, updated_at = $1 \
             WHERE event_id = $3 AND endpoint_id = $4 AND delivered_at IS NULL \
               AND dead_lettered_at IS NULL AND lease_token = $5 AND leased_until > $1",
        )
        .bind(delivered_at)
        .bind(response_status.map(i32::from))
        .bind(event_id)
        .bind(endpoint_id)
        .bind(lease_token)
        .execute(&mut *transaction)
        .await
        .map_err(database_error)?;
        if result.rows_affected() == 1 {
            sqlx::query(
                "UPDATE outbox_events SET delivered_at = $1, leased_until = NULL, lease_token = NULL \
                 WHERE id = $2 AND delivered_at IS NULL \
                   AND EXISTS (SELECT 1 FROM webhook_deliveries WHERE event_id = $2) \
                   AND NOT EXISTS (SELECT 1 FROM webhook_deliveries \
                                   WHERE event_id = $2 AND delivered_at IS NULL)",
            )
            .bind(delivered_at)
            .bind(event_id)
            .execute(&mut *transaction)
            .await
            .map_err(database_error)?;
        }
        transaction.commit().await.map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn record_webhook_delivery_failure(
        &self,
        event_id: &str,
        endpoint_id: &str,
        lease_token: &str,
        failed_at: OffsetDateTime,
        retry_at: OffsetDateTime,
        max_attempts: u32,
        last_error: &str,
    ) -> Result<Option<WebhookDeliveryFailureDisposition>, RepositoryError> {
        <Self as WebhookDeliveryRepository>::record_webhook_delivery_failure_with_status(
            self,
            event_id,
            endpoint_id,
            lease_token,
            failed_at,
            retry_at,
            max_attempts,
            None,
            last_error,
        )
        .await
    }

    async fn record_webhook_delivery_failure_with_status(
        &self,
        event_id: &str,
        endpoint_id: &str,
        lease_token: &str,
        failed_at: OffsetDateTime,
        retry_at: OffsetDateTime,
        max_attempts: u32,
        response_status: Option<u16>,
        last_error: &str,
    ) -> Result<Option<WebhookDeliveryFailureDisposition>, RepositoryError> {
        if max_attempts == 0 || retry_at <= failed_at || last_error.is_empty() {
            return Err(RepositoryError::Invariant(
                "webhook failure retry contract is invalid".into(),
            ));
        }
        validate_response_status(response_status)?;
        let lease_token = parse_lease_token(lease_token)?;
        let row = sqlx::query(
            "UPDATE webhook_deliveries SET attempts = attempts + 1, \
                 next_attempt_at = CASE WHEN attempts + 1 >= $1 THEN NULL ELSE $2 END, \
                 dead_lettered_at = CASE WHEN attempts + 1 >= $1 THEN $3 ELSE NULL END, \
                 leased_until = NULL, lease_token = NULL, last_error = $4, \
                 last_response_status = $5, updated_at = $3 \
             WHERE event_id = $6 AND endpoint_id = $7 AND delivered_at IS NULL \
               AND dead_lettered_at IS NULL AND lease_token = $8 AND leased_until > $3 \
             RETURNING attempts, next_attempt_at, dead_lettered_at",
        )
        .bind(i32::try_from(max_attempts).map_err(|_| {
            RepositoryError::Invariant("max attempts exceeds PostgreSQL INTEGER".into())
        })?)
        .bind(retry_at)
        .bind(failed_at)
        .bind(last_error)
        .bind(response_status.map(i32::from))
        .bind(event_id)
        .bind(endpoint_id)
        .bind(lease_token)
        .fetch_optional(&self.pool)
        .await
        .map_err(database_error)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let attempt_count = as_u32(row.try_get("attempts").map_err(database_error)?)?;
        let dead_lettered_at: Option<OffsetDateTime> =
            row.try_get("dead_lettered_at").map_err(database_error)?;
        Ok(Some(if let Some(dead_lettered_at) = dead_lettered_at {
            WebhookDeliveryFailureDisposition::DeadLettered {
                attempt_count,
                dead_lettered_at,
            }
        } else {
            WebhookDeliveryFailureDisposition::RetryScheduled {
                attempt_count,
                next_attempt_at: row.try_get("next_attempt_at").map_err(database_error)?,
            }
        }))
    }
}

pub(crate) async fn insert_outbox(
    transaction: &mut Transaction<'_, Postgres>,
    event: &OutboxEvent,
) -> Result<(), RepositoryError> {
    sqlx::query(
        "INSERT INTO outbox_events (id, application_id, event_type, aggregate_id, payload, \
         attempts, available_at, delivered_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) ON CONFLICT (id) DO NOTHING",
    )
    .bind(&event.id)
    .bind(event.application_id.as_uuid())
    .bind(&event.event_type)
    .bind(&event.aggregate_id)
    .bind(Json(event.payload.clone()))
    .bind(i32::try_from(event.attempt_count).map_err(|_| {
        RepositoryError::Invariant("outbox attempts exceeds PostgreSQL INTEGER".into())
    })?)
    .bind(event.next_attempt_at.unwrap_or(event.created_at))
    .bind(event.delivered_at)
    .bind(event.created_at)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    materialize_deliveries(transaction, &event.id).await?;
    Ok(())
}

async fn materialize_deliveries(
    transaction: &mut Transaction<'_, Postgres>,
    event_id: &str,
) -> Result<u64, RepositoryError> {
    let result = sqlx::query(
        "INSERT INTO webhook_deliveries \
         (event_id, endpoint_id, attempts, next_attempt_at, created_at, updated_at) \
         SELECT event.id, endpoint.id, 0, event.available_at, event.created_at, event.created_at \
         FROM outbox_events AS event JOIN webhook_endpoints AS endpoint \
           ON endpoint.application_id = event.application_id AND endpoint.enabled = TRUE \
         WHERE event.id = $1 AND event.delivered_at IS NULL \
           AND endpoint.subscribed_events ? event.event_type \
         ON CONFLICT (event_id, endpoint_id) DO NOTHING",
    )
    .bind(event_id)
    .execute(&mut **transaction)
    .await
    .map_err(database_error)?;
    Ok(result.rows_affected())
}

const DELIVERY_SELECT: &str = "SELECT event.id AS event_id, event.application_id AS event_application_id, \
     event.event_type AS event_type, event.aggregate_id AS aggregate_id, \
     event.payload AS payload, event.attempts AS event_attempts, \
     event.available_at AS event_available_at, event.delivered_at AS event_delivered_at, \
     event.created_at AS event_created_at, \
     endpoint.id AS endpoint_id, endpoint.application_id AS endpoint_application_id, \
     endpoint.url AS endpoint_url, endpoint.secret_ciphertext AS endpoint_secret, \
     endpoint.secret_key_version AS endpoint_key_version, delivery.attempts AS attempts, \
     delivery.next_attempt_at AS next_attempt_at, delivery.delivered_at AS delivered_at, \
     delivery.dead_lettered_at AS dead_lettered_at, delivery.last_error AS last_error, \
     delivery.created_at AS delivery_created_at, delivery.updated_at AS delivery_updated_at \
     FROM webhook_deliveries AS delivery \
     JOIN outbox_events AS event ON event.id = delivery.event_id \
     JOIN webhook_endpoints AS endpoint ON endpoint.id = delivery.endpoint_id \
     WHERE delivery.lease_token = $1 \
     ORDER BY delivery.next_attempt_at, delivery.event_id, delivery.endpoint_id";

fn row_to_delivery(
    row: sqlx::postgres::PgRow,
    lease_token: Uuid,
    leased_until: OffsetDateTime,
) -> Result<LeasedWebhookDelivery, RepositoryError> {
    let application_id = ApplicationId::from_uuid(
        row.try_get("event_application_id")
            .map_err(database_error)?,
    );
    let endpoint_application_id = ApplicationId::from_uuid(
        row.try_get("endpoint_application_id")
            .map_err(database_error)?,
    );
    if application_id != endpoint_application_id {
        return Err(RepositoryError::Invariant(
            "webhook delivery crosses application boundary".into(),
        ));
    }
    let delivery_attempt_count = as_u32(row.try_get("attempts").map_err(database_error)?)?;
    Ok(LeasedWebhookDelivery {
        delivery: WebhookDelivery {
            event: OutboxEvent {
                id: row.try_get("event_id").map_err(database_error)?,
                application_id,
                event_type: row.try_get("event_type").map_err(database_error)?,
                aggregate_id: row.try_get("aggregate_id").map_err(database_error)?,
                payload: row
                    .try_get::<Json<serde_json::Value>, _>("payload")
                    .map_err(database_error)?
                    .0,
                created_at: row.try_get("event_created_at").map_err(database_error)?,
                delivered_at: row.try_get("event_delivered_at").map_err(database_error)?,
                next_attempt_at: Some(row.try_get("event_available_at").map_err(database_error)?),
                attempt_count: as_u32(row.try_get("event_attempts").map_err(database_error)?)?,
            },
            endpoint: WebhookDeliveryEndpoint {
                id: row.try_get("endpoint_id").map_err(database_error)?,
                application_id: endpoint_application_id,
                url: row.try_get("endpoint_url").map_err(database_error)?,
                secret_ciphertext: row.try_get("endpoint_secret").map_err(database_error)?,
                secret_key_version: as_u32(
                    row.try_get("endpoint_key_version")
                        .map_err(database_error)?,
                )?,
            },
            attempt_count: delivery_attempt_count,
            next_attempt_at: row.try_get("next_attempt_at").map_err(database_error)?,
            delivered_at: row.try_get("delivered_at").map_err(database_error)?,
            dead_lettered_at: row.try_get("dead_lettered_at").map_err(database_error)?,
            last_error: row.try_get("last_error").map_err(database_error)?,
            created_at: row.try_get("delivery_created_at").map_err(database_error)?,
            updated_at: row.try_get("delivery_updated_at").map_err(database_error)?,
        },
        lease_token: lease_token.to_string(),
        leased_until,
    })
}

fn validate_claim(
    now: OffsetDateTime,
    lease_until: OffsetDateTime,
    _limit: usize,
) -> Result<(), RepositoryError> {
    if lease_until <= now {
        return Err(RepositoryError::Invariant(
            "lease must end in the future".into(),
        ));
    }
    Ok(())
}

fn parse_uuid(value: &str, field: &str) -> Result<Uuid, RepositoryError> {
    Uuid::parse_str(value).map_err(|_| RepositoryError::Invariant(format!("{field} is not a UUID")))
}

fn parse_lease_token(value: &str) -> Result<Uuid, RepositoryError> {
    parse_uuid(value, "lease token")
}

fn validate_response_status(response_status: Option<u16>) -> Result<(), RepositoryError> {
    if response_status.is_some_and(|status| !(100..=599).contains(&status)) {
        Err(RepositoryError::Invariant(
            "webhook delivery response status is invalid".into(),
        ))
    } else {
        Ok(())
    }
}

fn affected(rows: u64) -> Result<(), RepositoryError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(RepositoryError::NotFound)
    }
}
