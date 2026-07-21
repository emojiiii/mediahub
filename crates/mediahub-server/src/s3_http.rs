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
        header::{CONTENT_TYPE, ETAG},
    },
    response::{IntoResponse, Response},
};
use mediahub_app::{
    AccessKeyRepository, ApplicationError, ApplicationRepository, CancelUploadSessionRequest,
    CompleteUploadSessionRequest, CompletedS3MultipartPart, CreateUploadSessionRequest,
    MediaRepository, NewS3MultipartPart, NewS3MultipartUpload, ObjectStore, ObjectStoreError,
    OutboxEvent, S3MediaListQuery, S3MultipartAbort, S3MultipartCompletionClaim,
    S3MultipartCompletionFinish, S3MultipartManifestError, S3MultipartPartPut,
    S3MultipartRepository, S3MultipartUpload, S3MultipartUploadState, StagedUploadMediaRequest,
    StreamingUploadError, UploadMediaService, UploadSessionRepository, UploadSessionStorage,
};
use mediahub_core::{
    ApplicationId, BucketId, ClientMetadata, Media, MediaState, OffsetDateTime, UploadSessionId,
    Visibility,
};
use sha2::{Digest, Sha256};
use tracing::warn;

use super::s3_gateway::{ParsedSigV4, SigV4Error};
use super::s3_list::{
    ContinuationTokenCodec, ListObject, ListObjectsV2Query, ListObjectsV2Result, S3ListError,
};
use super::s3_multipart_storage::{
    cleanup_multipart_storage, new_multipart_completion_storage_key, new_multipart_part_storage_key,
};
use super::s3_xml::{
    DeleteObjectError, DeleteResult, DeletedObject, ListPartsResult, ListedPart, ObjectAcl,
    S3XmlError, complete_multipart_upload_result_xml, delete_result_xml, get_object_acl_xml,
    initiate_multipart_upload_result_xml, list_parts_result_xml,
    parse_complete_multipart_upload_xml, parse_delete_objects_xml, validate_content_md5,
};
use super::{
    ApiError, AppState, ApplicationAuth, HmacIdentity, MAX_ERROR_RESPONSE_BYTES,
    MAX_UPLOAD_OBJECT_BYTES, ReadMediaQuery, RequestId, SystemClock, entity_tag_header_value,
    normalized_mime, read_media_bytes, record_audit, upload_session_service,
    validate_upload_expected_size,
};

const MIN_S3_MULTIPART_PART_BYTES: u64 = 5 * 1024 * 1024;
const S3_MULTIPART_UPLOAD_SECONDS: i64 = 24 * 60 * 60;
const S3_MULTIPART_COMPLETION_LEASE_SECONDS: i64 = 5 * 60;

include!("s3_http_core.rs");
include!("s3_http_multipart.rs");
include!("s3_http_support.rs");
