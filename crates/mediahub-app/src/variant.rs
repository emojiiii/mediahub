use std::fmt;

use async_trait::async_trait;
use mediahub_core::{MediaId, OffsetDateTime, VariantFormat, VariantId, VariantTransform};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::Duration;

use crate::{
    Clock, ImageProcessor, ImageProcessorError, ObjectStore, ObjectStoreError, Redacted,
    RepositoryError, variant_cache_key,
};

pub const DEFAULT_VARIANT_LEASE_SECONDS: i64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantState {
    Generating,
    Ready,
    Failed,
    DeletePending,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewVariant {
    pub id: VariantId,
    pub media_id: MediaId,
    pub transform_key: String,
    pub parameters_json: String,
    pub processor_version: String,
    pub format: VariantFormat,
    pub storage_backend: String,
    pub storage_key: String,
    pub created_at: OffsetDateTime,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VariantRecord {
    pub id: VariantId,
    pub media_id: MediaId,
    pub transform_key: String,
    pub parameters_json: String,
    pub processor_version: String,
    pub format: VariantFormat,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: Option<u64>,
    pub storage_backend: String,
    pub storage_key: String,
    pub state: VariantState,
    pub last_error: Option<String>,
    pub last_accessed_at: Option<OffsetDateTime>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Clone, PartialEq, Eq)]
pub enum VariantClaim {
    Generate {
        variant: VariantRecord,
        lease_token: String,
    },
    Ready(VariantRecord),
    InProgress,
}

#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait VariantRepository: Send + Sync {
    /// Inserts a generating row or atomically leases an expired/failed row.
    /// Existing ready rows are returned without changing their storage facts.
    async fn claim_variant(
        &self,
        variant: NewVariant,
        lease_token: &str,
        leased_until: OffsetDateTime,
    ) -> Result<VariantClaim, RepositoryError>;

    /// Marks a generated object ready only while `lease_token` still owns the
    /// row. `None` means the lease was lost and the caller must not publish its
    /// database result.
    #[allow(clippy::too_many_arguments)]
    async fn complete_variant(
        &self,
        variant_id: VariantId,
        lease_token: &str,
        width: u32,
        height: u32,
        size: u64,
        completed_at: OffsetDateTime,
    ) -> Result<Option<VariantRecord>, RepositoryError>;

    async fn fail_variant(
        &self,
        variant_id: VariantId,
        lease_token: &str,
        error_summary: &str,
        failed_at: OffsetDateTime,
    ) -> Result<(), RepositoryError>;
}

#[derive(Clone, PartialEq, Eq)]
pub struct GenerateVariantRequest {
    pub media_id: MediaId,
    pub media_sha256: String,
    pub source_content: Vec<u8>,
    pub transform: VariantTransform,
}

#[derive(Clone, PartialEq, Eq)]
pub struct VariantReceipt {
    pub variant: VariantRecord,
    pub content: Vec<u8>,
    pub cache_hit: bool,
}

impl fmt::Debug for VariantClaim {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Generate {
                variant,
                lease_token,
            } => formatter
                .debug_struct("Generate")
                .field("variant", variant)
                .field("lease_token", &Redacted(lease_token))
                .finish(),
            Self::Ready(variant) => formatter.debug_tuple("Ready").field(variant).finish(),
            Self::InProgress => formatter.write_str("InProgress"),
        }
    }
}

impl fmt::Debug for GenerateVariantRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GenerateVariantRequest")
            .field("media_id", &self.media_id)
            .field("media_sha256", &self.media_sha256)
            .field("source_content_bytes", &self.source_content.len())
            .field("transform", &self.transform)
            .finish()
    }
}

impl fmt::Debug for VariantReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VariantReceipt")
            .field("variant", &self.variant)
            .field("content_bytes", &self.content.len())
            .field("cache_hit", &self.cache_hit)
            .finish()
    }
}

#[derive(Clone)]
pub struct VariantService<R, S, P, C> {
    repository: R,
    storage: S,
    processor: P,
    clock: C,
}

impl<R, S, P, C> VariantService<R, S, P, C>
where
    R: VariantRepository,
    S: ObjectStore,
    P: ImageProcessor,
    C: Clock,
{
    #[must_use]
    pub const fn new(repository: R, storage: S, processor: P, clock: C) -> Self {
        Self {
            repository,
            storage,
            processor,
            clock,
        }
    }

    pub async fn generate(
        &self,
        request: GenerateVariantRequest,
    ) -> Result<VariantReceipt, VariantApplicationError> {
        let now = self.clock.now();
        let processor_version = self.processor.processor_version();
        let transform_key =
            variant_cache_key(&request.media_sha256, &request.transform, processor_version);
        let variant_id = VariantId::new();
        let lease_token = VariantId::new().to_string();
        let storage_key = format!(
            "cache/image/{transform_key}.{}",
            request.transform.format().extension()
        );
        let parameters_json = serde_json::to_string(&request.transform)
            .map_err(|_| VariantApplicationError::InvalidTransform)?;
        let claim = self
            .repository
            .claim_variant(
                NewVariant {
                    id: variant_id,
                    media_id: request.media_id,
                    transform_key,
                    parameters_json,
                    processor_version: processor_version.to_owned(),
                    format: request.transform.format(),
                    storage_backend: self.storage.backend_name().to_owned(),
                    storage_key,
                    created_at: now,
                },
                &lease_token,
                now + Duration::seconds(DEFAULT_VARIANT_LEASE_SECONDS),
            )
            .await?;

        let (variant, lease_token) = match claim {
            VariantClaim::Ready(variant) => {
                let content = self.storage.read(&variant.storage_key).await?;
                return Ok(VariantReceipt {
                    variant,
                    content,
                    cache_hit: true,
                });
            }
            VariantClaim::InProgress => {
                return Err(VariantApplicationError::GenerationInProgress);
            }
            VariantClaim::Generate {
                variant,
                lease_token,
            } => (variant, lease_token),
        };

        let processed = match self
            .processor
            .process(&request.source_content, &request.transform)
            .await
        {
            Ok(processed) => processed,
            Err(error) => {
                self.record_failure(variant.id, &lease_token, &error.to_string())
                    .await;
                return Err(error.into());
            }
        };
        let temporary_key = format!("temp/variants/{}/{lease_token}", variant.id);
        if let Err(error) = self
            .storage
            .put_temporary(&temporary_key, &processed.content, &processed.mime)
            .await
        {
            self.record_failure(variant.id, &lease_token, &error.to_string())
                .await;
            return Err(error.into());
        }
        match self
            .storage
            .commit_temporary(&temporary_key, &variant.storage_key)
            .await
        {
            Ok(()) | Err(ObjectStoreError::AlreadyExists) => {}
            Err(error) => {
                let _ = self.storage.delete(&temporary_key).await;
                self.record_failure(variant.id, &lease_token, &error.to_string())
                    .await;
                return Err(error.into());
            }
        }

        let size = u64::try_from(processed.content.len())
            .map_err(|_| VariantApplicationError::OutputTooLarge)?;
        let ready = self
            .repository
            .complete_variant(
                variant.id,
                &lease_token,
                processed.width,
                processed.height,
                size,
                self.clock.now(),
            )
            .await?
            .ok_or(VariantApplicationError::LeaseLost)?;
        Ok(VariantReceipt {
            variant: ready,
            content: processed.content,
            cache_hit: false,
        })
    }

    async fn record_failure(&self, variant_id: VariantId, lease_token: &str, summary: &str) {
        let _ = self
            .repository
            .fail_variant(variant_id, lease_token, summary, self.clock.now())
            .await;
    }
}

#[derive(Debug, Error)]
pub enum VariantApplicationError {
    #[error("variant transform could not be serialized")]
    InvalidTransform,
    #[error("variant generation is already in progress")]
    GenerationInProgress,
    #[error("variant generation lease was lost")]
    LeaseLost,
    #[error("variant output exceeds the supported size")]
    OutputTooLarge,
    #[error(transparent)]
    Processor(#[from] ImageProcessorError),
    #[error(transparent)]
    Storage(#[from] ObjectStoreError),
    #[error(transparent)]
    Repository(#[from] RepositoryError),
}
