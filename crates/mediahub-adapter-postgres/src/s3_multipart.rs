// PostgreSQL S3 multipart repository wiring.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use mediahub_app::{
    CompletedS3MultipartPart, MAX_S3_MULTIPART_ACTIVE_UPLOADS_PER_APPLICATION,
    MAX_S3_MULTIPART_EXPIRY_LIMIT, NewS3MultipartPart, NewS3MultipartUpload, OutboxEvent,
    RepositoryError, S3MultipartAbort, S3MultipartCompletionClaim, S3MultipartCompletionFinish,
    S3MultipartCompletionRelease, S3MultipartExpiredUpload, S3MultipartManifest,
    S3MultipartManifestError, S3MultipartPart, S3MultipartPartPut, S3MultipartRepository,
    S3MultipartUpload, S3MultipartUploadState,
};
use mediahub_core::{ApplicationId, BucketId, Media, MediaId, MediaState, OffsetDateTime};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow, types::Json};

use crate::{
    PostgresRepository,
    codec::row_to_media,
    codec::{as_i64, as_u64, database_error, parse_visibility, postgres_time, visibility_name},
    media::{commit_upload_in_transaction, insert_media, lock_object_identity},
};

include!("multipart_lifecycle.rs");
include!("multipart_helpers.rs");
