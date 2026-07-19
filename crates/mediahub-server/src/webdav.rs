// WebDAV service wiring and responsibility-oriented implementations.

use std::{
    collections::BTreeMap,
    fmt,
    io::SeekFrom,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::{Buf, Bytes};
use dav_server::{
    DavHandler,
    davpath::DavPath,
    fakels::FakeLs,
    fs::{
        DavDirEntry, DavFile, DavMetaData, FsError, FsFuture, FsResult, FsStream,
        GuardedFileSystem, OpenOptions, ReadDirMeta,
    },
};
use futures_util::stream;
use mediahub_adapter_postgres::PostgresRepository;
use mediahub_app::{
    AccessKeyRepository, ApplicationError, ApplicationRepository, ApplicationSummary, AuditEvent,
    AuditRepository, Clock, MediaListCursor, MediaListQuery, MediaRepository, ObjectStore,
    OutboxEvent, UploadMediaRequest, UploadMediaService,
};
use mediahub_core::{
    ApplicationId, Bucket, BucketId, BucketPolicy, ClientMetadata, DomainError, Media, MediaState,
    OffsetDateTime, Visibility,
};
use subtle::ConstantTimeEq;
use tracing::warn;

use crate::{AppState, MAX_REQUEST_BYTES, RequestId, runtime_storage::RuntimeObjectStore};

const AUTH_CHALLENGE: &str = "Basic realm=\"MediaHub WebDAV\", charset=\"UTF-8\"";
const PAGE_SIZE: usize = 100;

#[derive(Clone)]
pub(crate) struct WebDavService {
    handler: DavHandler<DavCredentials>,
    repository: PostgresRepository,
    access_key_cipher: Arc<mediahub_server::access_key::AccessKeyCipher>,
}

include!("webdav_auth.rs");
include!("webdav_fs.rs");
include!("webdav_file.rs");
include!("webdav_support.rs");
