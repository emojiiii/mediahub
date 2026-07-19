// WebDAV resource and file-handle implementations.

struct DavUpload {
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: String,
    mime: String,
    content: Vec<u8>,
    visibility_override: Option<Visibility>,
    expire_at: Option<OffsetDateTime>,
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
    media: Media,
}

struct DavWriteFile {
    filesystem: MediaHubDavFs,
    credentials: DavCredentials,
    application_id: ApplicationId,
    bucket_id: BucketId,
    object_key: String,
    mime: String,
    expected_size: Option<u64>,
    content: Vec<u8>,
    committed: Option<Media>,
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
    fn read(object_store: RuntimeObjectStore, media: Media) -> Self {
        Self {
            mode: FileMode::Read(Box::new(DavReadFile {
                object_store,
                media,
            })),
            position: 0,
        }
    }

    fn write(
        filesystem: MediaHubDavFs,
        credentials: DavCredentials,
        application_id: ApplicationId,
        bucket_id: BucketId,
        object_key: String,
        mime: String,
        expected_size: Option<u64>,
    ) -> Self {
        Self {
            mode: FileMode::Write(Box::new(DavWriteFile {
                filesystem,
                credentials,
                application_id,
                bucket_id,
                object_key,
                mime,
                expected_size,
                content: Vec::new(),
                committed: None,
            })),
            position: 0,
        }
    }

    fn len(&self) -> u64 {
        match &self.mode {
            FileMode::Read(file) => file.media.size(),
            FileMode::Write(file) => file
                .committed
                .as_ref()
                .map_or(file.content.len() as u64, Media::size),
        }
    }
}

impl DavFile for MediaHubDavFile {
    fn metadata(&'_ mut self) -> FsFuture<'_, Box<dyn DavMetaData>> {
        Box::pin(async move {
            let metadata = match &self.mode {
                FileMode::Read(file) => DavMetadata::from_media(&file.media),
                FileMode::Write(file) => file.committed.as_ref().map_or_else(
                    || DavMetadata::file(file.content.len() as u64, SystemTime::now(), None),
                    DavMetadata::from_media,
                ),
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
            let start = self.position.min(file.media.size());
            let end = start.saturating_add(count as u64).min(file.media.size());
            if start == end {
                return Ok(Bytes::new());
            }
            let content = file
                .object_store
                .read_range(file.media.storage_key(), start..end)
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
                SeekFrom::End(offset) => checked_seek(self.len(), offset)?,
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
                application_id,
                bucket_id,
                object_key,
                mime,
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
            let media = filesystem
                .upload(DavUpload {
                    application_id: *application_id,
                    bucket_id: *bucket_id,
                    object_key: object_key.clone(),
                    mime: mime.clone(),
                    content: std::mem::take(content),
                    visibility_override: None,
                    expire_at: None,
                })
                .await?;
            *committed = Some(media);
            let committed_media = committed.as_ref().expect("WebDAV upload just committed");
            filesystem
                .record_audit(
                    credentials,
                    "media.uploaded",
                    "media",
                    committed_media.id().to_string(),
                    serde_json::json!({
                        "object_key": committed_media.object_key(),
                        "size": committed_media.size(),
                        "protocol": "webdav",
                    }),
                )
                .await;
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

    fn from_media(media: &Media) -> Self {
        Self::file(
            media.size(),
            to_system_time(media.updated_at()),
            Some(media.etag().to_owned()),
        )
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

    fn file(name: Vec<u8>, media: &Media) -> Self {
        Self {
            name,
            metadata: DavMetadata::from_media(media),
        }
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

