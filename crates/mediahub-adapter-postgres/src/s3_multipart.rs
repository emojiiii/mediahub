// PostgreSQL S3 multipart repository wiring.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use mediahub_app::{
    CompletedS3MultipartPart, DEFAULT_S3_MULTIPART_GC_MAX_ATTEMPTS,
    MAX_S3_MULTIPART_ACTIVE_UPLOADS_PER_APPLICATION, MAX_S3_MULTIPART_EXPIRY_LIMIT,
    NewS3MultipartPart, NewS3MultipartUpload, RepositoryError, S3MultipartAbort,
    S3MultipartCompletionClaim, S3MultipartCompletionRelease, S3MultipartExpiredUpload,
    S3MultipartManifest, S3MultipartManifestError, S3MultipartPart, S3MultipartPartPut,
    S3MultipartRepository, S3MultipartUpload, S3MultipartUploadState, is_multipart_etag,
};
use mediahub_core::{
    ApplicationId, BucketId, Checksum, EntityTag, NewStorageGcTask, ObjectId, ObjectVersionId,
    OffsetDateTime, S3ObjectTagSet, StorageGcReason, StorageGcTaskId, UploadIntent, UploadIntentId,
    UploadIntentState,
};
use sqlx::{Postgres, Row, Transaction, postgres::PgRow, types::Json};

use crate::{
    PostgresRepository,
    codec::{as_i64, as_u64, database_error, postgres_time},
    s3_repository::{insert_storage_gc_task, lock_upload_intent},
};

include!("multipart_lifecycle.rs");
include!("multipart_helpers.rs");
