// PostgreSQL bucket/media repository wiring.

use async_trait::async_trait;
use mediahub_app::{
    BucketRepository, LeasedMediaUpload, MediaDirectoryListCursor, MediaDirectoryListQuery,
    MediaDirectoryPage, MediaListQuery, MediaPage, MediaRepository, OutboxEvent, RepositoryError,
    S3MediaListQuery, S3MediaPage,
};
use mediahub_core::{
    ApplicationId, Bucket, BucketId, BucketPolicy, Media, MediaId, MediaState, OffsetDateTime,
    PersistedMedia,
};
use serde_json::Value;
use sqlx::{Postgres, QueryBuilder, Row, Transaction, types::Json};

use crate::{
    PostgresRepository,
    codec::{
        as_i64, database_error, media_state_name, postgres_time, row_to_bucket, row_to_media,
        visibility_name,
    },
    outbox::insert_outbox,
};

include!("media_buckets.rs");
include!("media_queries.rs");
include!("media_mutations.rs");
include!("media_support.rs");
