use async_trait::async_trait;
use mediahub_core::{ApplicationId, OffsetDateTime};

use crate::RepositoryError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewWebhookEndpoint {
    pub id: String,
    pub application_id: ApplicationId,
    pub url: String,
    pub secret_ciphertext: String,
    pub secret_key_version: u32,
    pub subscribed_events: Vec<String>,
    pub enabled: bool,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookEndpoint {
    pub id: String,
    pub application_id: ApplicationId,
    pub url: String,
    pub secret_ciphertext: String,
    pub secret_key_version: u32,
    pub subscribed_events: Vec<String>,
    pub enabled: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookEndpointUpdate {
    pub url: String,
    pub secret_ciphertext: String,
    pub secret_key_version: u32,
    pub subscribed_events: Vec<String>,
    pub enabled: bool,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WebhookDeliveryHistoryStatus {
    Pending,
    Delivered,
    DeadLettered,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebhookDeliveryHistoryCursor {
    pub updated_at: OffsetDateTime,
    pub row_id: i64,
}

#[derive(Clone, Debug)]
pub struct WebhookDeliveryHistoryQuery {
    pub status: Option<WebhookDeliveryHistoryStatus>,
    pub cursor: Option<WebhookDeliveryHistoryCursor>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookDeliveryHistoryItem {
    pub event_id: String,
    pub endpoint_id: String,
    pub event_type: String,
    pub row_id: i64,
    pub attempt_count: u32,
    pub status: WebhookDeliveryHistoryStatus,
    pub last_response_status: Option<u16>,
    pub last_error: Option<String>,
    pub next_attempt_at: Option<OffsetDateTime>,
    pub delivered_at: Option<OffsetDateTime>,
    pub dead_lettered_at: Option<OffsetDateTime>,
    pub replay_count: u32,
    pub last_replayed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebhookDeliveryHistoryPage {
    pub items: Vec<WebhookDeliveryHistoryItem>,
    pub has_more: bool,
}

#[async_trait]
pub trait WebhookEndpointRepository: Send + Sync {
    async fn create_webhook_endpoint(
        &self,
        endpoint: &NewWebhookEndpoint,
    ) -> Result<(), RepositoryError>;

    async fn list_webhook_endpoints(
        &self,
        application_id: ApplicationId,
    ) -> Result<Vec<WebhookEndpoint>, RepositoryError>;

    async fn find_webhook_endpoint(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
    ) -> Result<Option<WebhookEndpoint>, RepositoryError>;

    async fn update_webhook_endpoint(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
        update: &WebhookEndpointUpdate,
    ) -> Result<bool, RepositoryError>;

    async fn delete_webhook_endpoint(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
    ) -> Result<bool, RepositoryError>;

    async fn list_webhook_delivery_history(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
        query: &WebhookDeliveryHistoryQuery,
    ) -> Result<WebhookDeliveryHistoryPage, RepositoryError>;

    async fn replay_webhook_delivery(
        &self,
        application_id: ApplicationId,
        endpoint_id: &str,
        event_id: &str,
        replayed_at: OffsetDateTime,
    ) -> Result<bool, RepositoryError>;
}
