use async_trait::async_trait;
use mediahub_app::{
    NewWebhookEndpoint, RepositoryError, WebhookDeliveryHistoryItem, WebhookDeliveryHistoryPage,
    WebhookDeliveryHistoryQuery, WebhookDeliveryHistoryStatus, WebhookEndpoint,
    WebhookEndpointRepository, WebhookEndpointUpdate,
};
use mediahub_core::{ApplicationId, OffsetDateTime};
use sqlx::{Postgres, QueryBuilder, Row, postgres::PgRow, types::Json};

use crate::{
    PostgresRepository,
    codec::{as_i64, as_u32, database_error, postgres_time},
};

#[async_trait]
impl WebhookEndpointRepository for PostgresRepository {
    async fn create_webhook_endpoint(
        &self,
        endpoint: &NewWebhookEndpoint,
    ) -> Result<(), RepositoryError> {
        validate_secret_key_version(endpoint.secret_key_version)?;
        let created_at = postgres_time(endpoint.created_at);
        sqlx::query(
            "INSERT INTO webhook_endpoints \
             (id, application_id, url, secret_ciphertext, secret_key_version, subscribed_events, \
              enabled, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)",
        )
        .bind(&endpoint.id)
        .bind(endpoint.application_id.as_uuid())
        .bind(&endpoint.url)
        .bind(&endpoint.secret_ciphertext)
        .bind(as_i32(endpoint.secret_key_version)?)
        .bind(Json(endpoint.subscribed_events.clone()))
        .bind(endpoint.enabled)
        .bind(created_at)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(())
    }

    async fn list_webhook_endpoints(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<WebhookEndpoint>, RepositoryError> {
        let rows = sqlx::query(
            "SELECT * FROM webhook_endpoints WHERE application_id = $1 \
             ORDER BY created_at DESC, id DESC",
        )
        .bind(application_id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(database_error)?;
        rows.into_iter().map(row_to_webhook_endpoint).collect()
    }

    async fn find_webhook_endpoint(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
    ) -> Result<Option<WebhookEndpoint>, RepositoryError> {
        let row =
            sqlx::query("SELECT * FROM webhook_endpoints WHERE application_id = $1 AND id = $2")
                .bind(application_id.as_uuid())
                .bind(endpoint_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(database_error)?;
        row.map(row_to_webhook_endpoint).transpose()
    }

    async fn update_webhook_endpoint(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
        update: &WebhookEndpointUpdate,
    ) -> Result<bool, RepositoryError> {
        validate_secret_key_version(update.secret_key_version)?;
        let result = sqlx::query(
            "UPDATE webhook_endpoints SET url = $1, secret_ciphertext = $2, \
                    secret_key_version = $3, subscribed_events = $4, enabled = $5, updated_at = $6 \
             WHERE application_id = $7 AND id = $8",
        )
        .bind(&update.url)
        .bind(&update.secret_ciphertext)
        .bind(as_i32(update.secret_key_version)?)
        .bind(Json(update.subscribed_events.clone()))
        .bind(update.enabled)
        .bind(postgres_time(update.updated_at))
        .bind(application_id.as_uuid())
        .bind(endpoint_id)
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn delete_webhook_endpoint(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
    ) -> Result<bool, RepositoryError> {
        let result =
            sqlx::query("DELETE FROM webhook_endpoints WHERE application_id = $1 AND id = $2")
                .bind(application_id.as_uuid())
                .bind(endpoint_id)
                .execute(&self.pool)
                .await
                .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }

    async fn list_webhook_delivery_history(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
        query: &WebhookDeliveryHistoryQuery,
    ) -> Result<WebhookDeliveryHistoryPage, RepositoryError> {
        if query.limit == 0 || query.limit > 100 {
            return Err(RepositoryError::Invariant(
                "webhook delivery page limit must be between 1 and 100".into(),
            ));
        }
        let mut sql = QueryBuilder::<Postgres>::new(
            "SELECT delivery.event_id, delivery.endpoint_id, event.event_type, \
                    delivery.attempts, delivery.next_attempt_at, delivery.delivered_at, \
                    delivery.dead_lettered_at, delivery.last_error, delivery.last_response_status, \
                    delivery.replay_count, delivery.last_replayed_at, delivery.created_at, \
                    delivery.updated_at, delivery.history_id \
             FROM webhook_deliveries AS delivery \
             JOIN webhook_endpoints AS endpoint ON endpoint.id = delivery.endpoint_id \
             JOIN outbox_events AS event ON event.id = delivery.event_id \
             WHERE endpoint.application_id = ",
        );
        sql.push_bind(application_id.as_uuid())
            .push(" AND delivery.endpoint_id = ")
            .push_bind(endpoint_id.to_owned());
        match query.status {
            Some(WebhookDeliveryHistoryStatus::Pending) => {
                sql.push(
                    " AND delivery.delivered_at IS NULL AND delivery.dead_lettered_at IS NULL",
                );
            }
            Some(WebhookDeliveryHistoryStatus::Delivered) => {
                sql.push(" AND delivery.delivered_at IS NOT NULL");
            }
            Some(WebhookDeliveryHistoryStatus::DeadLettered) => {
                sql.push(" AND delivery.dead_lettered_at IS NOT NULL");
            }
            None => {}
        }
        if let Some(cursor) = query.cursor {
            let updated_at = postgres_time(cursor.updated_at);
            sql.push(" AND (delivery.updated_at < ")
                .push_bind(updated_at)
                .push(" OR (delivery.updated_at = ")
                .push_bind(updated_at)
                .push(" AND delivery.history_id < ")
                .push_bind(cursor.row_id)
                .push("))");
        }
        sql.push(" ORDER BY delivery.updated_at DESC, delivery.history_id DESC LIMIT ")
            .push_bind(as_i64(u64::try_from(query.limit + 1).map_err(|_| {
                RepositoryError::Invariant("webhook delivery page limit is too large".into())
            })?)?);
        let mut rows = sql
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(database_error)?;
        let has_more = rows.len() > query.limit;
        rows.truncate(query.limit);
        Ok(WebhookDeliveryHistoryPage {
            items: rows
                .into_iter()
                .map(row_to_webhook_delivery_history)
                .collect::<Result<Vec<_>, _>>()?,
            has_more,
        })
    }

    async fn replay_webhook_delivery(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
        event_id: &str,
        replayed_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError> {
        let replayed_at = postgres_time(replayed_at);
        let result = sqlx::query(
            "UPDATE webhook_deliveries AS delivery \
             SET attempts = 0, next_attempt_at = $1, delivered_at = NULL, \
                 dead_lettered_at = NULL, leased_until = NULL, lease_token = NULL, \
                 last_error = NULL, last_response_status = NULL, \
                 replay_count = replay_count + 1, last_replayed_at = $1, updated_at = $1 \
             FROM webhook_endpoints AS endpoint \
             WHERE delivery.endpoint_id = $2 AND delivery.event_id = $3 \
               AND (delivery.delivered_at IS NOT NULL OR delivery.dead_lettered_at IS NOT NULL) \
               AND endpoint.id = delivery.endpoint_id AND endpoint.application_id = $4",
        )
        .bind(replayed_at)
        .bind(endpoint_id)
        .bind(event_id)
        .bind(application_id.as_uuid())
        .execute(&self.pool)
        .await
        .map_err(database_error)?;
        Ok(result.rows_affected() == 1)
    }
}

fn row_to_webhook_endpoint(row: PgRow) -> Result<WebhookEndpoint, RepositoryError> {
    let secret_key_version = as_u32(row.try_get("secret_key_version").map_err(database_error)?)?;
    validate_secret_key_version(secret_key_version)?;
    Ok(WebhookEndpoint {
        id: row.try_get("id").map_err(database_error)?,
        application_id: ApplicationId::from_uuid(
            row.try_get("application_id").map_err(database_error)?,
        ),
        url: row.try_get("url").map_err(database_error)?,
        secret_ciphertext: row.try_get("secret_ciphertext").map_err(database_error)?,
        secret_key_version,
        subscribed_events: row
            .try_get::<Json<Vec<String>>, _>("subscribed_events")
            .map_err(database_error)?
            .0,
        enabled: row.try_get("enabled").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn row_to_webhook_delivery_history(
    row: PgRow,
) -> Result<WebhookDeliveryHistoryItem, RepositoryError> {
    let delivered_at = row.try_get("delivered_at").map_err(database_error)?;
    let dead_lettered_at = row.try_get("dead_lettered_at").map_err(database_error)?;
    let status = match (delivered_at, dead_lettered_at) {
        (Some(_), None) => WebhookDeliveryHistoryStatus::Delivered,
        (None, Some(_)) => WebhookDeliveryHistoryStatus::DeadLettered,
        (None, None) => WebhookDeliveryHistoryStatus::Pending,
        (Some(_), Some(_)) => {
            return Err(RepositoryError::Invariant(
                "webhook delivery cannot be delivered and dead-lettered".into(),
            ));
        }
    };
    let last_response_status = row
        .try_get::<Option<i32>, _>("last_response_status")
        .map_err(database_error)?
        .map(|status| {
            u16::try_from(status)
                .ok()
                .filter(|status| (100..=599).contains(status))
                .ok_or_else(|| {
                    RepositoryError::Invariant("webhook delivery response status is invalid".into())
                })
        })
        .transpose()?;
    Ok(WebhookDeliveryHistoryItem {
        event_id: row.try_get("event_id").map_err(database_error)?,
        endpoint_id: row.try_get("endpoint_id").map_err(database_error)?,
        event_type: row.try_get("event_type").map_err(database_error)?,
        row_id: row.try_get("history_id").map_err(database_error)?,
        attempt_count: as_u32(row.try_get("attempts").map_err(database_error)?)?,
        status,
        last_response_status,
        last_error: row.try_get("last_error").map_err(database_error)?,
        next_attempt_at: row.try_get("next_attempt_at").map_err(database_error)?,
        delivered_at,
        dead_lettered_at,
        replay_count: as_u32(row.try_get("replay_count").map_err(database_error)?)?,
        last_replayed_at: row.try_get("last_replayed_at").map_err(database_error)?,
        created_at: row.try_get("created_at").map_err(database_error)?,
        updated_at: row.try_get("updated_at").map_err(database_error)?,
    })
}

fn validate_secret_key_version(version: u32) -> Result<(), RepositoryError> {
    if version == 0 {
        Err(RepositoryError::Invariant(
            "webhook secret key version must be positive".into(),
        ))
    } else {
        Ok(())
    }
}

fn as_i32(value: u32) -> Result<i32, RepositoryError> {
    i32::try_from(value)
        .map_err(|_| RepositoryError::Invariant("webhook key version is too large".into()))
}
