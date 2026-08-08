// ObjectVersion-backed WebDAV resources and file handles.

type DavObjectService = S3ObjectService<
    PostgresRepository,
    PostgresRepository,
    PostgresRepository,
    RuntimeObjectStore,
    SystemClock,
>;

struct DavUpload {
    bucket_name: String,
    object_key: String,
    content_type: String,
    content: Vec<u8>,
}

enum DavResource {
    Root,
    Application,
    Bucket {
        bucket_name: String,
    },
    Object {
        bucket_name: String,
        object_key: String,
        collection: bool,
    },
}

impl DavResource {
    fn parse(path: &DavPath, credentials: &DavCredentials) -> FsResult<Self> {
        let path_text = std::str::from_utf8(path.as_bytes()).map_err(|_| FsError::Forbidden)?;
        let collection = path.is_collection();
        let segments = path_text
            .trim_matches('/')
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        match segments.as_slice() {
            [] => Ok(Self::Root),
            [app_id] if *app_id == credentials.application.app_id => Ok(Self::Application),
            [app_id, bucket_name] if *app_id == credentials.application.app_id => {
                Ok(Self::Bucket {
                    bucket_name: (*bucket_name).to_owned(),
                })
            }
            [app_id, bucket_name, object_segments @ ..]
                if *app_id == credentials.application.app_id =>
            {
                let object_key = object_segments.join("/");
                if object_key.is_empty() {
                    return Err(FsError::Forbidden);
                }
                Ok(Self::Object {
                    bucket_name: (*bucket_name).to_owned(),
                    object_key,
                    collection,
                })
            }
            _ => Err(FsError::NotFound),
        }
    }
}

struct MediaHubDavFile {
    mode: FileMode,
    position: u64,
}

enum FileMode {
    Read(Box<DavReadFile>),
    Write(Box<DavWriteFile>),
}

struct DavReadFile {
    object_store: RuntimeObjectStore,
    version: ObjectVersion,
}

struct DavWriteFile {
    filesystem: MediaHubDavFs,
    credentials: DavCredentials,
    bucket_name: String,
    object_key: String,
    content_type: String,
    expected_size: Option<u64>,
    content: Vec<u8>,
    committed: Option<CompletePutObjectReceipt>,
}

impl fmt::Debug for MediaHubDavFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MediaHubDavFile")
            .field(
                "mode",
                &match self.mode {
                    FileMode::Read(_) => "read",
                    FileMode::Write(_) => "write",
                },
            )
            .field("position", &self.position)
            .finish()
    }
}

impl MediaHubDavFile {
    fn read(object_store: RuntimeObjectStore, version: ObjectVersion) -> Self {
        Self {
            mode: FileMode::Read(Box::new(DavReadFile {
                object_store,
                version,
            })),
            position: 0,
        }
    }

    fn write(
        filesystem: MediaHubDavFs,
        credentials: DavCredentials,
        bucket_name: String,
        object_key: String,
        content_type: String,
        expected_size: Option<u64>,
    ) -> Self {
        Self {
            mode: FileMode::Write(Box::new(DavWriteFile {
                filesystem,
                credentials,
                bucket_name,
                object_key,
                content_type,
                expected_size,
                content: Vec::new(),
                committed: None,
            })),
            position: 0,
        }
    }

    fn len(&self) -> FsResult<u64> {
        match &self.mode {
            FileMode::Read(file) => Ok(stored_payload(&file.version)?.size_bytes()),
            FileMode::Write(file) => file.committed.as_ref().map_or_else(
                || Ok(file.content.len() as u64),
                |receipt| Ok(stored_payload(&receipt.version)?.size_bytes()),
            ),
        }
    }
}

impl DavFile for MediaHubDavFile {
    fn metadata(&'_ mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        Box::pin(async move {
            let metadata = match &self.mode {
                FileMode::Read(file) => DavMetadata::from_object_version(&file.version)?,
                FileMode::Write(file) => file.committed.as_ref().map_or_else(
                    || {
                        Ok(DavMetadata::file(
                            file.content.len() as u64,
                            SystemTime::now(),
                            None,
                        ))
                    },
                    |receipt| DavMetadata::from_object_version(&receipt.version),
                )?,
            };
            Ok(Box::new(metadata) as Box<dyn DavMetaData>)
        })
    }

    fn write_buf(&'_ mut self, mut buffer: Box<dyn Buf + Send>) -> FsFuture<'_, ()> {
        let mut bytes = Vec::with_capacity(buffer.remaining());
        while buffer.has_remaining() {
            let chunk = buffer.chunk();
            bytes.extend_from_slice(chunk);
            let length = chunk.len();
            buffer.advance(length);
        }
        self.write_bytes(Bytes::from(bytes))
    }

    fn write_bytes(&'_ mut self, buffer: Bytes) -> FsFuture<'_, ()> {
        Box::pin(async move {
            let FileMode::Write(file) = &mut self.mode else {
                return Err(FsError::Forbidden);
            };
            let DavWriteFile {
                content,
                expected_size,
                committed,
                ..
            } = file.as_mut();
            if committed.is_some() {
                return Err(FsError::Forbidden);
            }
            let start = usize::try_from(self.position).map_err(|_| FsError::TooLarge)?;
            let end = start.checked_add(buffer.len()).ok_or(FsError::TooLarge)?;
            if end > MAX_REQUEST_BYTES
                || expected_size.is_some_and(|expected| end as u64 > expected)
            {
                return Err(FsError::TooLarge);
            }
            if content.len() < end {
                content.resize(end, 0);
            }
            content[start..end].copy_from_slice(&buffer);
            self.position = end as u64;
            Ok(())
        })
    }

    fn read_bytes(&'_ mut self, count: usize) -> FsFuture<'_, Bytes> {
        Box::pin(async move {
            let FileMode::Read(file) = &self.mode else {
                return Err(FsError::Forbidden);
            };
            let payload = stored_payload(&file.version)?;
            let start = self.position.min(payload.size_bytes());
            let end = start.saturating_add(count as u64).min(payload.size_bytes());
            if start == end {
                return Ok(Bytes::new());
            }
            let content = file
                .object_store
                .read_range(payload.storage_key(), start..end)
                .await
                .map_err(|_| FsError::GeneralFailure)?;
            self.position = end;
            Ok(Bytes::from(content))
        })
    }

    fn seek(&'_ mut self, position: SeekFrom) -> FsFuture<'_, u64> {
        Box::pin(async move {
            let next = match position {
                SeekFrom::Start(position) => position,
                SeekFrom::Current(offset) => checked_seek(self.position, offset)?,
                SeekFrom::End(offset) => checked_seek(self.len()?, offset)?,
            };
            self.position = next;
            Ok(next)
        })
    }

    fn flush(&'_ mut self) -> FsFuture<'_, ()> {
        Box::pin(async move {
            let FileMode::Write(file) = &mut self.mode else {
                return Ok(());
            };
            let DavWriteFile {
                filesystem,
                credentials,
                bucket_name,
                object_key,
                content_type,
                expected_size,
                content,
                committed,
            } = file.as_mut();
            if committed.is_some() {
                return Ok(());
            }
            if expected_size.is_some_and(|expected| expected != content.len() as u64) {
                return Err(FsError::GeneralFailure);
            }
            let receipt = filesystem
                .put_object(
                    DavUpload {
                        bucket_name: bucket_name.clone(),
                        object_key: object_key.clone(),
                        content_type: content_type.clone(),
                        content: std::mem::take(content),
                    },
                    credentials,
                )
                .await?;
            filesystem
                .record_audit(
                    credentials,
                    "dav.object.uploaded",
                    "object_version",
                    receipt.version.id().to_string(),
                    serde_json::json!({
                        "object_id": receipt.object.id().to_string(),
                        "object_key": object_key,
                        "size": stored_payload(&receipt.version)?.size_bytes(),
                        "version_id": receipt.version.external_version_id().as_str(),
                        "protocol": "webdav",
                    }),
                )
                .await;
            *committed = Some(receipt);
            Ok(())
        })
    }
}

#[derive(Clone, Debug)]
struct DavMetadata {
    is_dir: bool,
    len: u64,
    modified: SystemTime,
    etag: Option<String>,
}

impl DavMetadata {
    fn directory(modified: SystemTime) -> Self {
        Self {
            is_dir: true,
            len: 0,
            modified,
            etag: None,
        }
    }

    fn file(len: u64, modified: SystemTime, etag: Option<String>) -> Self {
        Self {
            is_dir: false,
            len,
            modified,
            etag,
        }
    }

    fn from_object_version(version: &ObjectVersion) -> FsResult<Self> {
        let payload = stored_payload(version)?;
        Ok(Self::file(
            payload.size_bytes(),
            to_system_time(version.created_at()),
            Some(payload.etag().as_str().to_owned()),
        ))
    }
}

impl DavMetaData for DavMetadata {
    fn len(&self) -> u64 {
        self.len
    }

    fn modified(&self) -> FsResult<SystemTime> {
        Ok(self.modified)
    }

    fn is_dir(&self) -> bool {
        self.is_dir
    }

    fn etag(&self) -> Option<String> {
        self.etag.clone()
    }
}

#[derive(Clone, Debug)]
struct DavEntry {
    name: Vec<u8>,
    metadata: DavMetadata,
}

impl DavEntry {
    fn directory(name: Vec<u8>, modified: SystemTime) -> Self {
        Self {
            name,
            metadata: DavMetadata::directory(modified),
        }
    }

    fn file(name: Vec<u8>, version: &ObjectVersion) -> FsResult<Self> {
        Ok(Self {
            name,
            metadata: DavMetadata::from_object_version(version)?,
        })
    }
}

impl DavDirEntry for DavEntry {
    fn name(&self) -> Vec<u8> {
        self.name.clone()
    }

    fn metadata(&'_ self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        let metadata = self.metadata.clone();
        Box::pin(async move { Ok(Box::new(metadata) as Box<dyn DavMetaData>) })
    }
}

fn stored_payload(version: &ObjectVersion) -> FsResult<&StoredObjectVersion> {
    match version.payload() {
        ObjectVersionPayload::Object(payload) => Ok(payload),
        ObjectVersionPayload::DeleteMarker => Err(FsError::NotFound),
    }
}

fn directory_prefix(object_key: &str) -> String {
    if object_key.ends_with('/') {
        object_key.to_owned()
    } else {
        format!("{object_key}/")
    }
}

fn guess_mime(object_key: &str) -> String {
    mime_guess::from_path(object_key)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_owned()
}

fn checked_seek(base: u64, offset: i64) -> FsResult<u64> {
    if offset < 0 {
        base.checked_sub(offset.unsigned_abs())
            .ok_or(FsError::Forbidden)
    } else {
        base.checked_add(offset as u64).ok_or(FsError::TooLarge)
    }
}

fn to_system_time(value: OffsetDateTime) -> SystemTime {
    let seconds = value.unix_timestamp();
    let nanos = value.nanosecond();
    if seconds >= 0 {
        UNIX_EPOCH + Duration::new(seconds as u64, nanos)
    } else {
        UNIX_EPOCH - Duration::new(seconds.unsigned_abs(), nanos)
    }
}

fn map_s3_model_error(_error: S3ModelError) -> FsError {
    FsError::Forbidden
}

fn map_repository_error(error: RepositoryError) -> FsError {
    match error {
        RepositoryError::Conflict => FsError::Exists,
        RepositoryError::QuotaExceeded => FsError::InsufficientStorage,
        RepositoryError::NotFound => FsError::NotFound,
        _ => FsError::GeneralFailure,
    }
}

fn map_s3_object_error(error: S3ObjectServiceError) -> FsError {
    match error {
        S3ObjectServiceError::BucketNotFound
        | S3ObjectServiceError::ObjectNotFound
        | S3ObjectServiceError::VersionNotFound
        | S3ObjectServiceError::DeleteMarker { .. } => FsError::NotFound,
        S3ObjectServiceError::DeleteLocked(_) => FsError::Forbidden,
        S3ObjectServiceError::Repository(RepositoryError::QuotaExceeded) => {
            FsError::InsufficientStorage
        }
        S3ObjectServiceError::Model(_) => FsError::Forbidden,
        _ => FsError::GeneralFailure,
    }
}

fn map_streaming_upload_error(error: StreamingUploadError) -> FsError {
    match error {
        StreamingUploadError::SizeMismatch { .. } => FsError::GeneralFailure,
        StreamingUploadError::Stream(_) | StreamingUploadError::Storage(_) => {
            FsError::GeneralFailure
        }
    }
}
