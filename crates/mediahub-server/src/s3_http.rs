// S3 HTTP gateway imports and responsibility-oriented implementations.

use std::{
    borrow::Cow,
    net::SocketAddr,
    sync::{Arc, atomic::Ordering},
};

use axum::{
    body::{Body, Bytes, to_bytes},
    extract::{ConnectInfo, Extension, OriginalUri, Path, State},
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
    NewS3ObjectLock, ObjectStore, ObjectStoreError, PrepareClaimedUploadCommitRequest,
    PutObjectLegalHoldRequest, PutObjectRetentionRequest, PutObjectTaggingRequest, RepositoryError,
    S3AuthorizationOutcome, S3AuthorizationPrincipal, S3AuthorizationRequest,
    S3AuthorizationService, S3BucketIdentity, S3BucketPolicyDocument, S3BucketPolicyRepository,
    S3BucketRepository, S3DeleteLockReason, S3IdentityPolicyRepository, S3ListingRepository,
    S3MultipartAbort, S3MultipartCompletionClaim, S3MultipartManifest, S3MultipartManifestError,
    S3MultipartPartPut, S3MultipartRepository, S3MultipartUpload, S3MultipartUploadListQuery,
    S3MultipartUploadPage, S3MultipartUploadState, S3ObjectListItem, S3ObjectListQuery,
    S3ObjectPage, S3ObjectRepository, S3ObjectRequest, S3ObjectService, S3ObjectServiceError,
    S3ObjectVersionListQuery, S3ObjectVersionPage, S3SignedPrincipal, S3UploadIntentRepository,
    StorageGcRepository, StreamingUploadError, is_lowercase_md5_hex,
};
use mediahub_core::{
    ApplicationId, Bucket, BucketId, Checksum, ChecksumAlgorithm, DefaultRetention,
    DefaultRetentionPeriod, EntityTag, MAX_S3_POLICY_BYTES, NewStorageGcTask, ObjectRetention,
    ObjectVersion, ObjectVersionPayload, OffsetDateTime, RetentionMode, S3Bucket, S3BucketPolicy,
    S3IdentityPolicy, S3IdentityPolicyRequest, S3ObjectTag, S3ObjectTagSet, S3PolicyAction,
    S3PolicyDecision, S3PolicyQuery, S3PolicyResourceScope, S3VersionId, StorageGcReason,
    StorageGcTaskId, StoredObjectVersion, UploadIntent, UploadIntentId, UploadIntentState,
    VersioningStatus, Visibility,
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
    DeleteObjectError, DeleteResult, DeletedObject, ListMultipartUploadsResult,
    ListObjectVersionsResult, ListPartsResult, ListedMultipartUpload, ListedObjectVersion,
    ListedPart, ListedVersionKind, MAX_S3_XML_BODY_BYTES, ObjectAcl, S3TaggingXmlError, S3XmlError,
    complete_multipart_upload_result_xml, copy_object_result_xml, copy_part_result_xml,
    delete_result_xml, get_object_acl_xml, initiate_multipart_upload_result_xml,
    list_multipart_uploads_result_xml, list_object_versions_result_xml, list_parts_result_xml,
    object_tagging_xml, parse_complete_multipart_upload_xml, parse_delete_objects_xml,
    parse_object_tagging_xml, validate_content_md5,
};
use super::{
    ApiError, AppState, ApplicationAuth, HmacIdentity, MAX_ERROR_RESPONSE_BYTES,
    MAX_UPLOAD_OBJECT_BYTES, RequestId, SystemClock, entity_tag_header_value, local_file_body,
    normalized_mime, record_audit,
};

const MIN_S3_MULTIPART_PART_BYTES: u64 = 5 * 1024 * 1024;
const S3_MULTIPART_UPLOAD_SECONDS: i64 = 24 * 60 * 60;
const S3_MULTIPART_COMPLETION_LEASE_SECONDS: i64 = 5 * 60;

include!("s3_http_bucket.rs");
include!("s3_http_configuration.rs");
include!("s3_http_policy.rs");
include!("s3_data_authorization.rs");
include!("s3_http_tagging.rs");
include!("s3_http_object_lock.rs");
include!("s3_http_object_version_lock.rs");
include!("s3_http_copy.rs");
include!("s3_http_core.rs");
include!("s3_http_multipart.rs");
include!("s3_http_object.rs");
include!("s3_http_support.rs");

#[cfg(test)]
include!("s3_http_bucket_tests.rs");
#[cfg(test)]
include!("s3_http_configuration_tests.rs");
#[cfg(test)]
include!("s3_http_policy_tests.rs");
#[cfg(test)]
include!("s3_http_object_lock_tests.rs");
#[cfg(test)]
include!("s3_http_object_version_lock_tests.rs");
#[cfg(test)]
include!("s3_http_object_tests.rs");
#[cfg(test)]
include!("s3_http_delete_tests.rs");
#[cfg(test)]
include!("s3_http_list_tests.rs");
#[cfg(test)]
include!("s3_http_copy_tests.rs");
#[cfg(test)]
include!("s3_http_tagging_tests.rs");
#[cfg(test)]
include!("s3_http_listing_tests.rs");
#[cfg(test)]
include!("s3_data_authorization_tests.rs");
