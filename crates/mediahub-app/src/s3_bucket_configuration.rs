use async_trait::async_trait;
use mediahub_core::{ApplicationId, OffsetDateTime, S3BucketTagSet, S3CorsConfiguration};

use crate::RepositoryError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3BucketCorsSnapshot {
    pub configuration: Option<S3CorsConfiguration>,
    pub revision: u64,
    pub updated_at: Option<OffsetDateTime>,
}

impl S3BucketCorsSnapshot {
    pub fn new(
        configuration: Option<S3CorsConfiguration>,
        revision: u64,
        updated_at: Option<OffsetDateTime>,
    ) -> Result<Self, RepositoryError> {
        if (revision == 0) != updated_at.is_none() || (revision == 0 && configuration.is_some()) {
            return Err(RepositoryError::Invariant(
                "S3 Bucket CORS document, revision, and update timestamp disagree".into(),
            ));
        }
        if let Some(configuration) = &configuration {
            configuration.validate().map_err(|error| {
                RepositoryError::Invariant(format!(
                    "persisted S3 Bucket CORS configuration is invalid: {error}"
                ))
            })?;
        }
        Ok(Self {
            configuration,
            revision,
            updated_at,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3BucketTaggingSnapshot {
    pub tags: Option<S3BucketTagSet>,
    pub revision: u64,
    pub updated_at: Option<OffsetDateTime>,
}

impl S3BucketTaggingSnapshot {
    pub fn new(
        tags: Option<S3BucketTagSet>,
        revision: u64,
        updated_at: Option<OffsetDateTime>,
    ) -> Result<Self, RepositoryError> {
        if (revision == 0) != updated_at.is_none() || (revision == 0 && tags.is_some()) {
            return Err(RepositoryError::Invariant(
                "S3 Bucket Tagging document, revision, and update timestamp disagree".into(),
            ));
        }
        if let Some(tags) = &tags {
            tags.validate().map_err(|error| {
                RepositoryError::Invariant(format!(
                    "persisted S3 Bucket Tagging document is invalid: {error}"
                ))
            })?;
        }
        Ok(Self {
            tags,
            revision,
            updated_at,
        })
    }
}

#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3BucketCorsRepository: Send + Sync {
    async fn get_s3_bucket_cors(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
    ) -> Result<Option<S3BucketCorsSnapshot>, RepositoryError>;

    async fn put_s3_bucket_cors(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        configuration: S3CorsConfiguration,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketCorsSnapshot>, RepositoryError>;

    async fn delete_s3_bucket_cors(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketCorsSnapshot>, RepositoryError>;
}

#[allow(clippy::missing_errors_doc)]
#[async_trait]
pub trait S3BucketTaggingRepository: Send + Sync {
    async fn get_s3_bucket_tagging(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
    ) -> Result<Option<S3BucketTaggingSnapshot>, RepositoryError>;

    async fn put_s3_bucket_tagging(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        tags: S3BucketTagSet,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketTaggingSnapshot>, RepositoryError>;

    async fn delete_s3_bucket_tagging(
        &self,
        application_id: ApplicationId,
        bucket_name: &str,
        updated_at: OffsetDateTime,
    ) -> Result<Option<S3BucketTaggingSnapshot>, RepositoryError>;
}

#[cfg(test)]
mod tests {
    use mediahub_core::S3BucketTagSet;

    use super::*;

    #[test]
    fn deleted_and_never_written_states_are_distinct_and_validated() {
        assert!(S3BucketCorsSnapshot::new(None, 0, None).is_ok());
        assert!(S3BucketCorsSnapshot::new(None, 1, Some(OffsetDateTime::UNIX_EPOCH)).is_ok());
        assert!(S3BucketCorsSnapshot::new(None, 1, None).is_err());
        assert!(
            S3BucketTaggingSnapshot::new(
                Some(S3BucketTagSet::empty()),
                1,
                Some(OffsetDateTime::UNIX_EPOCH),
            )
            .is_ok()
        );
        assert!(S3BucketTaggingSnapshot::new(None, 0, Some(OffsetDateTime::UNIX_EPOCH)).is_err());
    }
}
