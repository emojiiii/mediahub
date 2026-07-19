//! Local filesystem implementation of the `MediaHub` object storage port.

use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    ops::Range,
    path::{Component, Path, PathBuf},
};

#[cfg(unix)]
use std::fs::File;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{Stream, StreamExt, pin_mut};
use mediahub_app::{
    ComposedObject, ObjectMetadata, ObjectPage, ObjectStore, ObjectStoreError, PreparedUpload,
    StoredUpload, UploadSessionStorage, UploadTarget,
};
use mediahub_core::{MediaId, OffsetDateTime, UploadSession, UploadSessionId};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

const LOCAL_BACKEND: &str = "local";
const UPLOAD_TARGET_PREFIX: &str = "/api/v1/uploads";
const METADATA_DIRECTORY: &str = ".mediahub-metadata";

#[derive(Debug, thiserror::Error)]
pub enum LocalUploadError {
    #[error("uploaded content size {actual} does not match expected size {expected}")]
    SizeMismatch { expected: u64, actual: u64 },

    #[error("upload stream failed: {0}")]
    Stream(String),

    #[error(transparent)]
    Storage(#[from] ObjectStoreError),
}

/// Stores all media beneath one configured directory.
///
/// Storage keys are validated as relative component paths. A temporary object
/// is promoted by creating a hard link at the final path, which fails if that
/// path already exists and therefore preserves `MediaHub`'s no-overwrite rule.
#[derive(Clone, Debug)]
pub struct LocalObjectStore {
    root: PathBuf,
}

impl LocalObjectStore {
    /// Creates the configured storage root if it does not already exist.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage root cannot be created.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, ObjectStoreError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|error| io_error(&error))?;
        Ok(Self { root })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Reads one stored object's complete binary content.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid, the object is missing, or
    /// the underlying filesystem cannot be read.
    pub fn read(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        fs::read(self.path_for(key)?).map_err(|error| map_read_error(&error))
    }

    /// Opens one object for asynchronous streaming reads.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid or the object cannot be opened.
    pub async fn open_file(&self, key: &str) -> Result<tokio::fs::File, ObjectStoreError> {
        tokio::fs::File::open(self.path_for(key)?)
            .await
            .map_err(|error| map_read_error(&error))
    }

    /// Streams one uncommitted upload directly to local storage.
    ///
    /// # Errors
    ///
    /// Returns an error when the key already exists, the body stream fails,
    /// the received byte count differs from `expected_size`, or local storage
    /// cannot durably persist the object and its MIME sidecar.
    pub async fn put_temporary_stream<S, E>(
        &self,
        temporary_key: &str,
        stream: S,
        expected_size: u64,
        content_type: &str,
    ) -> Result<(), LocalUploadError>
    where
        S: Stream<Item = Result<Bytes, E>> + Send,
        E: std::fmt::Display,
    {
        let object_path = self.path_for(temporary_key)?;
        let metadata_path = self.metadata_path_for(temporary_key)?;
        if let Some(parent) = object_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| LocalUploadError::Storage(io_error(&error)))?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&object_path)
            .await
            .map_err(|error| LocalUploadError::Storage(map_create_error(&error)))?;

        pin_mut!(stream);
        let mut received = 0_u64;
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    drop(file);
                    let _ = tokio::fs::remove_file(&object_path).await;
                    return Err(LocalUploadError::Stream(error.to_string()));
                }
            };
            received = match received.checked_add(chunk.len() as u64) {
                Some(value) => value,
                None => {
                    drop(file);
                    let _ = tokio::fs::remove_file(&object_path).await;
                    return Err(LocalUploadError::SizeMismatch {
                        expected: expected_size,
                        actual: u64::MAX,
                    });
                }
            };
            if received > expected_size {
                drop(file);
                let _ = tokio::fs::remove_file(&object_path).await;
                return Err(LocalUploadError::SizeMismatch {
                    expected: expected_size,
                    actual: received,
                });
            }
            if let Err(error) = file.write_all(&chunk).await {
                drop(file);
                let _ = tokio::fs::remove_file(&object_path).await;
                return Err(LocalUploadError::Storage(io_error(&error)));
            }
        }
        if received != expected_size {
            drop(file);
            let _ = tokio::fs::remove_file(&object_path).await;
            return Err(LocalUploadError::SizeMismatch {
                expected: expected_size,
                actual: received,
            });
        }
        if let Err(error) = file.sync_all().await {
            drop(file);
            let _ = tokio::fs::remove_file(&object_path).await;
            return Err(LocalUploadError::Storage(io_error(&error)));
        }
        drop(file);

        if let Some(parent) = metadata_path.parent()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            let _ = tokio::fs::remove_file(&object_path).await;
            return Err(LocalUploadError::Storage(io_error(&error)));
        }
        let _ = tokio::fs::remove_file(&metadata_path).await;
        let metadata_result = async {
            let mut metadata = tokio::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&metadata_path)
                .await
                .map_err(|error| LocalUploadError::Storage(map_create_error(&error)))?;
            metadata
                .write_all(content_type.as_bytes())
                .await
                .map_err(|error| LocalUploadError::Storage(io_error(&error)))?;
            metadata
                .sync_all()
                .await
                .map_err(|error| LocalUploadError::Storage(io_error(&error)))?;
            Ok::<(), LocalUploadError>(())
        }
        .await;
        if let Err(error) = metadata_result {
            let _ = tokio::fs::remove_file(&object_path).await;
            let _ = tokio::fs::remove_file(&metadata_path).await;
            return Err(error);
        }
        Ok(())
    }

    fn path_for(&self, key: &str) -> Result<PathBuf, ObjectStoreError> {
        Ok(self.root.join(Self::relative_path_for(key)?))
    }

    fn metadata_path_for(&self, key: &str) -> Result<PathBuf, ObjectStoreError> {
        Ok(self
            .root
            .join(METADATA_DIRECTORY)
            .join(Self::relative_path_for(key)?))
    }

    fn relative_path_for(key: &str) -> Result<PathBuf, ObjectStoreError> {
        if key.is_empty() {
            return Err(ObjectStoreError::Unavailable("storage key is empty".into()));
        }

        let mut safe_path = PathBuf::new();
        for component in Path::new(key).components() {
            match component {
                Component::Normal(value) => safe_path.push(value),
                Component::CurDir => {}
                Component::ParentDir | Component::Prefix(_) | Component::RootDir => {
                    return Err(ObjectStoreError::Unavailable(
                        "storage key is not a relative path".into(),
                    ));
                }
            }
        }

        if safe_path.as_os_str().is_empty() {
            return Err(ObjectStoreError::Unavailable("storage key is empty".into()));
        }
        Ok(safe_path)
    }

    fn write_new(path: &Path, content: &[u8]) -> Result<(), ObjectStoreError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| io_error(&error))?;
        }
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| map_create_error(&error))?;
        file.write_all(content).map_err(|error| io_error(&error))?;
        file.sync_all().map_err(|error| io_error(&error))?;
        Ok(())
    }

    fn compose_new(
        &self,
        temporary_key: &str,
        source_keys: &[String],
        content_type: &str,
    ) -> Result<ComposedObject, ObjectStoreError> {
        if source_keys.is_empty() {
            return Err(ObjectStoreError::Unavailable(
                "multipart composition requires at least one part".into(),
            ));
        }
        let object_path = self.path_for(temporary_key)?;
        let metadata_path = self.metadata_path_for(temporary_key)?;
        if let Some(parent) = object_path.parent() {
            fs::create_dir_all(parent).map_err(|error| io_error(&error))?;
        }
        let result = (|| {
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&object_path)
                .map_err(|error| map_create_error(&error))?;
            let mut digest = Sha256::new();
            let mut size = 0_u64;
            let mut buffer = [0_u8; 64 * 1024];
            for source_key in source_keys {
                let mut source = fs::File::open(self.path_for(source_key)?)
                    .map_err(|error| map_read_error(&error))?;
                loop {
                    let read = source.read(&mut buffer).map_err(|error| io_error(&error))?;
                    if read == 0 {
                        break;
                    }
                    output
                        .write_all(&buffer[..read])
                        .map_err(|error| io_error(&error))?;
                    digest.update(&buffer[..read]);
                    size = size
                        .checked_add(u64::try_from(read).map_err(|error| {
                            ObjectStoreError::Unavailable(format!(
                                "part size is not representable: {error}"
                            ))
                        })?)
                        .ok_or_else(|| {
                            ObjectStoreError::Unavailable("composed object exceeds u64".into())
                        })?;
                }
            }
            output.sync_all().map_err(|error| io_error(&error))?;
            Self::write_new(&metadata_path, content_type.as_bytes())?;
            Ok(ComposedObject {
                size,
                sha256: hex::encode(digest.finalize()),
            })
        })();
        if result.is_err() {
            let _ = Self::remove_if_exists(&object_path);
            let _ = Self::remove_if_exists(&metadata_path);
        }
        result
    }

    fn remove_if_exists(path: &Path) -> Result<(), ObjectStoreError> {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(io_error(&error)),
        }
    }

    fn inspect_path(path: &Path) -> Result<(u64, String), ObjectStoreError> {
        let mut file = fs::File::open(path).map_err(|error| map_read_error(&error))?;
        let mut digest = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).map_err(|error| io_error(&error))?;
            if read == 0 {
                break;
            }
            size = size
                .checked_add(u64::try_from(read).map_err(|error| {
                    ObjectStoreError::Unavailable(format!(
                        "object size is not representable: {error}"
                    ))
                })?)
                .ok_or_else(|| ObjectStoreError::Unavailable("object exceeds u64 size".into()))?;
            digest.update(&buffer[..read]);
        }
        Ok((size, hex::encode(digest.finalize())))
    }

    fn content_type_for(&self, key: &str) -> Result<Option<String>, ObjectStoreError> {
        match fs::read_to_string(self.metadata_path_for(key)?) {
            Ok(content_type) => Ok(Some(content_type)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error(&error)),
        }
    }

    fn collect_object_keys(
        root: &Path,
        directory: &Path,
        keys: &mut Vec<String>,
    ) -> Result<(), ObjectStoreError> {
        for entry in fs::read_dir(directory).map_err(|error| io_error(&error))? {
            let entry = entry.map_err(|error| io_error(&error))?;
            let path = entry.path();
            if path == root.join(METADATA_DIRECTORY) {
                continue;
            }
            let file_type = entry.file_type().map_err(|error| io_error(&error))?;
            if file_type.is_dir() {
                Self::collect_object_keys(root, &path, keys)?;
            } else if file_type.is_file() {
                let relative = path.strip_prefix(root).map_err(|error| {
                    ObjectStoreError::Unavailable(format!(
                        "object path escaped the storage root: {error}"
                    ))
                })?;
                let key = relative
                    .components()
                    .map(|component| component.as_os_str().to_str())
                    .collect::<Option<Vec<_>>>()
                    .ok_or_else(|| {
                        ObjectStoreError::Unavailable("object key is not valid UTF-8".into())
                    })?
                    .join("/");
                keys.push(key);
            }
        }
        Ok(())
    }

    #[cfg(unix)]
    fn sync_parent(path: &Path) -> Result<(), ObjectStoreError> {
        let parent = path
            .parent()
            .ok_or_else(|| ObjectStoreError::Unavailable("storage key has no parent".into()))?;
        File::open(parent)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_error(&error))
    }

    #[cfg(not(unix))]
    fn sync_parent(_path: &Path) {}
}

#[async_trait]
impl ObjectStore for LocalObjectStore {
    fn backend_name(&self) -> &'static str {
        LOCAL_BACKEND
    }

    async fn put_temporary(
        &self,
        temporary_key: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        let object_path = self.path_for(temporary_key)?;
        Self::write_new(&object_path, content)?;
        if let Err(error) = Self::write_new(
            &self.metadata_path_for(temporary_key)?,
            content_type.as_bytes(),
        ) {
            let _ = Self::remove_if_exists(&object_path);
            return Err(error);
        }
        Ok(())
    }

    async fn compose_temporary(
        &self,
        temporary_key: &str,
        source_keys: &[String],
        content_type: &str,
    ) -> Result<ComposedObject, ObjectStoreError> {
        self.compose_new(temporary_key, source_keys, content_type)
    }

    async fn commit_temporary(
        &self,
        temporary_key: &str,
        final_key: &str,
    ) -> Result<(), ObjectStoreError> {
        let temporary_path = self.path_for(temporary_key)?;
        let final_path = self.path_for(final_key)?;
        let temporary_metadata = self.metadata_path_for(temporary_key)?;
        let final_metadata = self.metadata_path_for(final_key)?;
        if !temporary_path.exists() {
            return Err(ObjectStoreError::NotFound);
        }
        if !temporary_metadata.is_file() {
            return Err(ObjectStoreError::Unavailable(
                "temporary object metadata is missing".into(),
            ));
        }
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).map_err(|error| io_error(&error))?;
        }
        if let Some(parent) = final_metadata.parent() {
            fs::create_dir_all(parent).map_err(|error| io_error(&error))?;
        }

        // `hard_link` atomically creates the final name and refuses to replace
        // an existing object. Both paths reside below the same configured root.
        fs::hard_link(&temporary_path, &final_path).map_err(|error| map_create_error(&error))?;
        if let Err(error) = fs::hard_link(&temporary_metadata, &final_metadata) {
            let _ = Self::remove_if_exists(&final_path);
            return Err(map_create_error(&error));
        }
        fs::remove_file(&temporary_path).map_err(|error| io_error(&error))?;
        fs::remove_file(&temporary_metadata).map_err(|error| io_error(&error))?;
        #[cfg(unix)]
        Self::sync_parent(&final_path)?;
        #[cfg(not(unix))]
        Self::sync_parent(&final_path);
        Ok(())
    }

    async fn read(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        Self::read(self, key)
    }

    async fn read_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ObjectStoreError> {
        let mut file =
            fs::File::open(self.path_for(key)?).map_err(|error| map_read_error(&error))?;
        let size = file.metadata().map_err(|error| io_error(&error))?.len();
        if range.start >= range.end || range.end > size {
            return Err(ObjectStoreError::InvalidRange);
        }
        file.seek(SeekFrom::Start(range.start))
            .map_err(|error| io_error(&error))?;
        let length =
            usize::try_from(range.end - range.start).map_err(|_| ObjectStoreError::InvalidRange)?;
        let mut content = vec![0; length];
        file.read_exact(&mut content)
            .map_err(|error| io_error(&error))?;
        Ok(content)
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, ObjectStoreError> {
        let (size, checksum_sha256) = Self::inspect_path(&self.path_for(key)?)?;
        Ok(ObjectMetadata {
            key: key.to_owned(),
            size,
            content_type: self.content_type_for(key)?,
            etag: None,
            version: None,
            checksum_sha256: Some(checksum_sha256),
            provider_metadata: BTreeMap::new(),
        })
    }

    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ObjectPage, ObjectStoreError> {
        if limit == 0 || limit > 1_000 {
            return Err(ObjectStoreError::InvalidLimit);
        }
        Self::relative_path_for(prefix)?;
        if let Some(cursor) = cursor {
            Self::relative_path_for(cursor)?;
            if !cursor.starts_with(prefix) {
                return Err(ObjectStoreError::InvalidCursor);
            }
        }
        if !self.root.is_dir() {
            return Err(ObjectStoreError::Unavailable(
                "storage root is unavailable".into(),
            ));
        }
        let mut keys = Vec::new();
        Self::collect_object_keys(&self.root, &self.root, &mut keys)?;
        keys.retain(|key| {
            key.starts_with(prefix) && cursor.is_none_or(|value| key.as_str() > value)
        });
        keys.sort();
        let has_more = keys.len() > limit;
        keys.truncate(limit);
        let objects = keys
            .iter()
            .map(|key| {
                let size = fs::metadata(self.path_for(key)?)
                    .map_err(|error| map_read_error(&error))?
                    .len();
                Ok(ObjectMetadata {
                    key: key.clone(),
                    size,
                    content_type: self.content_type_for(key)?,
                    etag: None,
                    version: None,
                    checksum_sha256: None,
                    provider_metadata: BTreeMap::new(),
                })
            })
            .collect::<Result<Vec<_>, ObjectStoreError>>()?;
        Ok(ObjectPage {
            next_cursor: has_more.then(|| keys.last().expect("non-empty page").clone()),
            objects,
        })
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        Self::remove_if_exists(&self.path_for(key)?)?;
        Self::remove_if_exists(&self.metadata_path_for(key)?)
    }

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        Ok(self.path_for(key)?.is_file())
    }
}

#[async_trait]
impl UploadSessionStorage for LocalObjectStore {
    async fn prepare_upload(
        &self,
        upload_session_id: UploadSessionId,
        media_id: MediaId,
        expected_size: u64,
        expected_mime: &str,
        expires_at: OffsetDateTime,
    ) -> Result<PreparedUpload, ObjectStoreError> {
        Ok(PreparedUpload {
            target: UploadTarget {
                method: "PUT".to_owned(),
                url: format!("{UPLOAD_TARGET_PREFIX}/{upload_session_id}/content"),
                headers: BTreeMap::from([
                    ("content-length".to_owned(), expected_size.to_string()),
                    ("content-type".to_owned(), expected_mime.to_owned()),
                ]),
                expires_at,
            },
            storage_backend: LOCAL_BACKEND.to_owned(),
            storage_key: format!("objects/{media_id}"),
        })
    }

    async fn inspect_upload(
        &self,
        session: &UploadSession,
    ) -> Result<StoredUpload, ObjectStoreError> {
        let object_path = self.path_for(session.storage_key())?;
        let metadata_path = self.metadata_path_for(session.storage_key())?;
        tokio::task::spawn_blocking(move || {
            let (size, sha256) = Self::inspect_path(&object_path)?;
            let mime = fs::read_to_string(metadata_path).map_err(|error| map_read_error(&error))?;
            Ok(StoredUpload { size, mime, sha256 })
        })
        .await
        .map_err(|error| {
            ObjectStoreError::Unavailable(format!("upload inspection task failed: {error}"))
        })?
    }

    async fn abort_upload(&self, session: &UploadSession) -> Result<(), ObjectStoreError> {
        self.delete(session.storage_key()).await
    }
}

fn io_error(error: &std::io::Error) -> ObjectStoreError {
    ObjectStoreError::Unavailable(error.to_string())
}

fn map_read_error(error: &std::io::Error) -> ObjectStoreError {
    if error.kind() == std::io::ErrorKind::NotFound {
        ObjectStoreError::NotFound
    } else {
        io_error(error)
    }
}

fn map_create_error(error: &std::io::Error) -> ObjectStoreError {
    if error.kind() == std::io::ErrorKind::AlreadyExists {
        ObjectStoreError::AlreadyExists
    } else {
        io_error(error)
    }
}

#[cfg(test)]
mod tests {
    include!("tests.rs");
}
