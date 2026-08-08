// S3 HTTP gateway imports and responsibility-oriented implementations.

use std::{
    borrow::Cow,
    sync::{Arc, atomic::Ordering},
};

use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{Extension, OriginalUri, Path, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode, Uri,
        header::{
            ACCEPT_RANGES, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, ETAG, IF_MATCH,
            IF_NONE_MATCH, LAST_MODIFIED, LOCATION, RANGE,
        },
    },
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use mediahub_app::{
    AbortStagedPutRequest, AccessKeyRepository, ApplicationRepository, BeginPutObjectRequest,
    CompletePutObjectRequest, CompletedS3MultipartPart, DEFAULT_S3_MULTIPART_GC_MAX_ATTEMPTS,
    DeleteObjectReceipt, DeleteObjectRequest, NewS3MultipartPart, NewS3MultipartUpload,
    ObjectStore, ObjectStoreError, RepositoryError, S3BucketRepository, S3DeleteLockReason,
    S3MultipartAbort, S3MultipartCompletionClaim, S3MultipartManifest, S3MultipartManifestError,
    S3MultipartPartPut, S3MultipartRepository, S3MultipartUpload, S3MultipartUploadState,
    S3ObjectListItem, S3ObjectListQuery, S3ObjectPage, S3ObjectRepository, S3ObjectRequest,
    S3ObjectService, S3ObjectServiceError, S3UploadIntentRepository, StorageGcRepository,
    StreamingUploadError, is_lowercase_md5_hex,
};
use mediahub_core::{
    ApplicationId, Bucket, BucketId, BucketPolicy, Checksum, ChecksumAlgorithm, EntityTag,
    NewStorageGcTask, ObjectVersion, ObjectVersionPayload, OffsetDateTime, S3VersionId,
    StorageGcReason, StorageGcTaskId, StoredObjectVersion, UploadIntent, UploadIntentId,
    UploadIntentState, VersioningStatus, Visibility,
};
use quick_xml::{Reader, events::Event};
use sha2::{Digest, Sha256};
use tracing::warn;

use super::s3_gateway::{ParsedSigV4, SigV4Error};
use super::s3_list::{
    ContinuationTokenCodec, ListObject, ListObjectsV2Query, ListObjectsV2Result, S3ListError,
};
use super::s3_multipart_storage::{multipart_entity_tag, new_multipart_part_storage_key};
use super::s3_xml::{
    DeleteObjectError, DeleteResult, DeletedObject, ListPartsResult, ListedPart, ObjectAcl,
    S3XmlError, complete_multipart_upload_result_xml, delete_result_xml, get_object_acl_xml,
    initiate_multipart_upload_result_xml, list_parts_result_xml,
    parse_complete_multipart_upload_xml, parse_delete_objects_xml, validate_content_md5,
};
use super::{
    ApiError, AppState, ApplicationAuth, HmacIdentity, MAX_ERROR_RESPONSE_BYTES,
    MAX_UPLOAD_OBJECT_BYTES, RequestId, SystemClock, entity_tag_header_value, local_file_body,
    normalized_mime, record_audit, validate_upload_expected_size,
};

const MIN_S3_MULTIPART_PART_BYTES: u64 = 5 * 1024 * 1024;
const S3_MULTIPART_UPLOAD_SECONDS: i64 = 24 * 60 * 60;
const S3_MULTIPART_COMPLETION_LEASE_SECONDS: i64 = 5 * 60;

include!("s3_http_bucket.rs");
include!("s3_http_configuration.rs");
include!("s3_http_core.rs");
include!("s3_http_multipart.rs");
include!("s3_http_object.rs");
include!("s3_http_support.rs");

#[cfg(test)]
include!("s3_http_bucket_tests.rs");
#[cfg(test)]
include!("s3_http_configuration_tests.rs");
#[cfg(test)]
include!("s3_http_object_tests.rs");
#[cfg(test)]
include!("s3_http_delete_tests.rs");
#[cfg(test)]
include!("s3_http_list_tests.rs");
