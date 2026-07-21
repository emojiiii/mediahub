//! Local filesystem implementation of the `MediaHub` object storage port.

use std::{
    collections::{BTreeMap, BinaryHeap},
    fs::{self, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    ops::Range,
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex},
};

#[cfg(unix)]
use std::fs::File;

use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{Stream, StreamExt, pin_mut};
use mediahub_app::{
    ComposedObject, ObjectMetadata, ObjectPage, ObjectStore, ObjectStoreError, PreparedUpload,
    StoredUpload, StreamedObject, StreamingUploadError, UploadSessionStorage, UploadTarget,
};
use mediahub_core::{MediaId, OffsetDateTime, UploadSession, UploadSessionId, UploadSessionState};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const LOCAL_BACKEND: &str = "local";
const UPLOAD_TARGET_PREFIX: &str = "/api/v1/uploads";
const METADATA_DIRECTORY: &str = ".mediahub-metadata";
const STAGING_DIRECTORY: &str = ".mediahub-staging";

pub use mediahub_app::StreamingUploadError as LocalUploadError;

/// Stores all media beneath one configured directory.
///
/// Storage keys are validated as relative component paths. A temporary object
/// is promoted by creating a hard link at the final path, which fails if that
/// path already exists and therefore preserves `MediaHub`'s no-overwrite rule.
#[derive(Clone, Debug)]
pub struct LocalObjectStore {
    root: PathBuf,
    mutation_lock: Arc<Mutex<()>>,
}

impl LocalObjectStore {
    /// Creates the configured storage root if it does not already exist.
    ///
    /// # Errors
    ///
    /// Returns an error when the storage root cannot be created.
    pub fn new(root: impl Into<PathBuf>) -> Result<Self, ObjectStoreError> {
        let root = root.into();
        if fs::symlink_metadata(&root)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(ObjectStoreError::Unavailable(
                "local storage root must not be a symlink".into(),
            ));
        }
        fs::create_dir_all(&root).map_err(|error| io_error(&error))?;
        let root = fs::canonicalize(root).map_err(|error| io_error(&error))?;
        let staging = root.join(STAGING_DIRECTORY);
        if fs::symlink_metadata(&staging)
            .map(|metadata| metadata.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(ObjectStoreError::Unavailable(
                "local staging directory must not be a symlink".into(),
            ));
        }
        fs::create_dir_all(&staging).map_err(|error| io_error(&error))?;
        if fs::symlink_metadata(&staging)
            .map_err(|error| io_error(&error))?
            .file_type()
            .is_symlink()
        {
            return Err(ObjectStoreError::Unavailable(
                "local staging directory must not be a symlink".into(),
            ));
        }
        Ok(Self {
            root,
            mutation_lock: Arc::new(Mutex::new(())),
        })
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
        let path = self.path_for(key)?;
        self.ensure_safe_path(&path, false)?;
        fs::read(path).map_err(|error| map_read_error(&error))
    }

    /// Opens one object for asynchronous streaming reads.
    ///
    /// # Errors
    ///
    /// Returns an error when the key is invalid or the object cannot be opened.
    pub async fn open_file(&self, key: &str) -> Result<tokio::fs::File, ObjectStoreError> {
        let path = self.path_for(key)?;
        self.ensure_safe_path(&path, false)?;
        tokio::fs::File::open(path)
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
    ) -> Result<StreamedObject, StreamingUploadError>
    where
        S: Stream<Item = Result<Bytes, E>> + Send,
        E: std::fmt::Display,
    {
        let object_path = self.path_for(temporary_key)?;
        let metadata_path = self.metadata_path_for(temporary_key)?;
        self.ensure_safe_path(&object_path, true)?;
        if fs::symlink_metadata(&object_path)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false)
        {
            return Err(LocalUploadError::Storage(ObjectStoreError::AlreadyExists));
        }
        let (stage_object, stage_metadata) = self.new_staging_paths();
        self.ensure_safe_path(&stage_object, true)?;
        let mut file = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&stage_object)
            .await
            .map_err(|error| StreamingUploadError::Storage(map_create_error(&error)))?;

        pin_mut!(stream);
        let mut received = 0_u64;
        let mut digest = Sha256::new();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => {
                    drop(file);
                    let _ = tokio::fs::remove_file(&stage_object).await;
                    return Err(StreamingUploadError::Stream(error.to_string()));
                }
            };
            received = match received.checked_add(chunk.len() as u64) {
                Some(value) => value,
                None => {
                    drop(file);
                    let _ = tokio::fs::remove_file(&stage_object).await;
                    return Err(StreamingUploadError::SizeMismatch {
                        expected: expected_size,
                        actual: u64::MAX,
                    });
                }
            };
            if received > expected_size {
                drop(file);
                let _ = tokio::fs::remove_file(&stage_object).await;
                return Err(StreamingUploadError::SizeMismatch {
                    expected: expected_size,
                    actual: received,
                });
            }
            if let Err(error) = file.write_all(&chunk).await {
                drop(file);
                let _ = tokio::fs::remove_file(&stage_object).await;
                return Err(StreamingUploadError::Storage(io_error(&error)));
            }
            digest.update(&chunk);
        }
        if received != expected_size {
            drop(file);
            let _ = tokio::fs::remove_file(&stage_object).await;
            return Err(StreamingUploadError::SizeMismatch {
                expected: expected_size,
                actual: received,
            });
        }
        if let Err(error) = file.sync_all().await {
            drop(file);
            let _ = tokio::fs::remove_file(&stage_object).await;
            return Err(StreamingUploadError::Storage(io_error(&error)));
        }
        drop(file);

        let metadata_result = async {
            let mut metadata = tokio::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&stage_metadata)
                .await
                .map_err(|error| StreamingUploadError::Storage(map_create_error(&error)))?;
            metadata
                .write_all(content_type.as_bytes())
                .await
                .map_err(|error| StreamingUploadError::Storage(io_error(&error)))?;
            metadata
                .sync_all()
                .await
                .map_err(|error| StreamingUploadError::Storage(io_error(&error)))?;
            Ok::<(), StreamingUploadError>(())
        }
        .await;
        if let Err(error) = metadata_result {
            let _ = tokio::fs::remove_file(&stage_object).await;
            let _ = tokio::fs::remove_file(&stage_metadata).await;
            return Err(error);
        }

        let store = self.clone();
        let result = tokio::task::spawn_blocking(move || {
            store.install_staged(&stage_object, &stage_metadata, &object_path, &metadata_path)
        })
        .await
        .map_err(|error| {
            StreamingUploadError::Storage(ObjectStoreError::Unavailable(format!(
                "local upload installation task failed: {error}"
            )))
        })?;
        result.map_err(StreamingUploadError::Storage)?;
        Ok(StreamedObject {
            size: received,
            sha256: hex::encode(digest.finalize()),
        })
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

        if key.starts_with('/')
            || key.ends_with('/')
            || key.contains("//")
            || key.contains("/./")
            || key.contains("/../")
            || key == "."
            || key == ".."
            || key.contains('\\')
        {
            return Err(ObjectStoreError::Unavailable(
                "storage key is not a canonical relative path".into(),
            ));
        }

        let mut safe_path = PathBuf::new();
        for (index, segment) in key.split('/').enumerate() {
            if segment.is_empty()
                || segment == "."
                || segment == ".."
                || segment.ends_with(['.', ' '])
                || segment.contains(':')
                || segment
                    .chars()
                    .any(|value| value == '\0' || value.is_control())
                || (index == 0
                    && (segment.eq_ignore_ascii_case(METADATA_DIRECTORY)
                        || segment.eq_ignore_ascii_case(STAGING_DIRECTORY)))
                || Self::is_windows_reserved_segment(segment)
            {
                return Err(ObjectStoreError::Unavailable(
                    "storage key contains an invalid or reserved component".into(),
                ));
            }
            match Path::new(segment).components().next() {
                Some(Component::Normal(_)) => safe_path.push(segment),
                _ => {
                    return Err(ObjectStoreError::Unavailable(
                        "storage key is not a canonical relative path".into(),
                    ));
                }
            }
        }
        Ok(safe_path)
    }

    fn is_windows_reserved_segment(segment: &str) -> bool {
        let basename = segment.split('.').next().unwrap_or(segment);
        let uppercase = basename.to_ascii_uppercase();
        matches!(uppercase.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || uppercase
                .strip_prefix("COM")
                .or_else(|| uppercase.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
                })
    }

    fn ensure_safe_path(
        &self,
        path: &Path,
        allow_missing_final: bool,
    ) -> Result<(), ObjectStoreError> {
        let relative = path.strip_prefix(&self.root).map_err(|error| {
            ObjectStoreError::Unavailable(format!("path escaped the storage root: {error}"))
        })?;
        let mut current = self.root.clone();
        let components = relative.components().collect::<Vec<_>>();
        for (index, component) in components.iter().enumerate() {
            let Component::Normal(value) = component else {
                return Err(ObjectStoreError::Unavailable(
                    "storage path is not a normal relative path".into(),
                ));
            };
            current.push(value);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(ObjectStoreError::Unavailable(
                        "storage path contains a symlink".into(),
                    ));
                }
                Ok(metadata) if index + 1 < components.len() && !metadata.file_type().is_dir() => {
                    return Err(ObjectStoreError::Unavailable(
                        "storage path contains a non-directory component".into(),
                    ));
                }
                Ok(_) => {}
                Err(error)
                    if error.kind() == std::io::ErrorKind::NotFound
                        && allow_missing_final
                        && index + 1 == components.len() => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error) => return Err(io_error(&error)),
            }
        }
        Ok(())
    }

    fn ensure_parent_directory(&self, path: &Path) -> Result<(), ObjectStoreError> {
        let parent = path
            .parent()
            .ok_or_else(|| ObjectStoreError::Unavailable("storage path has no parent".into()))?;
        let relative = parent.strip_prefix(&self.root).map_err(|error| {
            ObjectStoreError::Unavailable(format!("path escaped the storage root: {error}"))
        })?;
        let mut current = self.root.clone();
        for component in relative.components() {
            let Component::Normal(value) = component else {
                return Err(ObjectStoreError::Unavailable(
                    "storage parent is not a normal relative path".into(),
                ));
            };
            current.push(value);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(ObjectStoreError::Unavailable(
                        "storage parent contains a symlink".into(),
                    ));
                }
                Ok(metadata) if metadata.file_type().is_dir() => {}
                Ok(_) => {
                    return Err(ObjectStoreError::Unavailable(
                        "storage parent contains a non-directory component".into(),
                    ));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir(&current).map_err(|create_error| {
                        if create_error.kind() == std::io::ErrorKind::AlreadyExists {
                            ObjectStoreError::Unavailable(
                                "storage parent changed while creating directory".into(),
                            )
                        } else {
                            io_error(&create_error)
                        }
                    })?;
                }
                Err(error) => return Err(io_error(&error)),
            }
        }
        self.ensure_safe_path(parent, false)
    }

    fn mutation_guard(&self) -> Result<std::sync::MutexGuard<'_, ()>, ObjectStoreError> {
        self.mutation_lock
            .lock()
            .map_err(|_| ObjectStoreError::Unavailable("local mutation lock is poisoned".into()))
    }

    fn new_staging_paths(&self) -> (PathBuf, PathBuf) {
        let id = Uuid::new_v4();
        let base = self.root.join(STAGING_DIRECTORY).join(id.to_string());
        (
            base.with_extension("object"),
            base.with_extension("metadata"),
        )
    }

    fn write_staging_file(&self, path: &Path, content: &[u8]) -> Result<(), ObjectStoreError> {
        self.ensure_parent_directory(path)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(path)
            .map_err(|error| map_create_error(&error))?;
        file.write_all(content).map_err(|error| io_error(&error))?;
        file.sync_all().map_err(|error| io_error(&error))?;
        #[cfg(unix)]
        Self::sync_parent(path)?;
        Ok(())
    }

    fn put_temporary_sync(
        &self,
        temporary_key: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        let object_path = self.path_for(temporary_key)?;
        let metadata_path = self.metadata_path_for(temporary_key)?;
        self.ensure_safe_path(&object_path, true)?;
        self.ensure_safe_path(&metadata_path, true)?;
        if fs::symlink_metadata(&object_path)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false)
        {
            return Err(ObjectStoreError::AlreadyExists);
        }
        let (stage_object, stage_metadata) = self.new_staging_paths();
        let result = (|| {
            self.write_staging_file(&stage_object, content)?;
            self.write_staging_file(&stage_metadata, content_type.as_bytes())?;
            self.install_staged(&stage_object, &stage_metadata, &object_path, &metadata_path)
        })();
        if result.is_err() {
            Self::remove_staging_files(&stage_object, &stage_metadata);
        }
        result
    }

    fn remove_staging_files(object: &Path, metadata: &Path) {
        let _ = fs::remove_file(object);
        let _ = fs::remove_file(metadata);
    }

    fn install_staged(
        &self,
        stage_object: &Path,
        stage_metadata: &Path,
        object_path: &Path,
        metadata_path: &Path,
    ) -> Result<(), ObjectStoreError> {
        self.ensure_safe_path(stage_object, false)?;
        self.ensure_safe_path(stage_metadata, false)?;
        self.ensure_parent_directory(object_path)?;
        self.ensure_parent_directory(metadata_path)?;
        self.ensure_safe_path(object_path, true)?;
        self.ensure_safe_path(metadata_path, true)?;

        let _guard = self.mutation_guard()?;
        let object_exists =
            fs::symlink_metadata(object_path).is_ok_and(|metadata| metadata.file_type().is_file());
        let metadata_exists = fs::symlink_metadata(metadata_path)
            .is_ok_and(|metadata| metadata.file_type().is_file());
        if object_exists {
            Self::remove_staging_files(stage_object, stage_metadata);
            return Err(ObjectStoreError::AlreadyExists);
        }
        if metadata_exists {
            // A crash can leave only the sidecar link. It is safe to repair it
            // while the object name is still absent.
            Self::remove_if_exists(metadata_path)?;
        }

        fs::hard_link(stage_metadata, metadata_path).map_err(|error| map_create_error(&error))?;
        if let Err(error) = fs::hard_link(stage_object, object_path) {
            let _ = Self::remove_if_exists(metadata_path);
            Self::remove_staging_files(stage_object, stage_metadata);
            return Err(map_create_error(&error));
        }
        #[cfg(unix)]
        {
            Self::sync_parent(metadata_path)?;
            Self::sync_parent(object_path)?;
        }
        Self::remove_staging_files(stage_object, stage_metadata);
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
        self.ensure_safe_path(&object_path, true)?;
        self.ensure_safe_path(&metadata_path, true)?;
        if fs::symlink_metadata(&object_path)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false)
        {
            return Err(ObjectStoreError::AlreadyExists);
        }
        let (stage_object, stage_metadata) = self.new_staging_paths();
        let result = (|| {
            let mut output = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&stage_object)
                .map_err(|error| map_create_error(&error))?;
            let mut digest = Sha256::new();
            let mut size = 0_u64;
            let mut buffer = [0_u8; 64 * 1024];
            for source_key in source_keys {
                let source_path = self.path_for(source_key)?;
                self.ensure_safe_path(&source_path, false)?;
                let mut source =
                    fs::File::open(source_path).map_err(|error| map_read_error(&error))?;
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
            #[cfg(unix)]
            Self::sync_parent(&stage_object)?;
            self.write_staging_file(&stage_metadata, content_type.as_bytes())?;
            self.install_staged(&stage_object, &stage_metadata, &object_path, &metadata_path)?;
            Ok(ComposedObject {
                size,
                sha256: hex::encode(digest.finalize()),
            })
        })();
        if result.is_err() {
            Self::remove_staging_files(&stage_object, &stage_metadata);
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

    fn files_equal(left: &Path, right: &Path) -> Result<bool, ObjectStoreError> {
        let left_metadata = fs::metadata(left).map_err(|error| map_read_error(&error))?;
        let right_metadata = fs::metadata(right).map_err(|error| map_read_error(&error))?;
        if left_metadata.len() != right_metadata.len() {
            return Ok(false);
        }
        let mut left_file = fs::File::open(left).map_err(|error| map_read_error(&error))?;
        let mut right_file = fs::File::open(right).map_err(|error| map_read_error(&error))?;
        let mut left_buffer = [0_u8; 64 * 1024];
        let mut right_buffer = [0_u8; 64 * 1024];
        loop {
            let left_read = left_file
                .read(&mut left_buffer)
                .map_err(|error| io_error(&error))?;
            let right_read = right_file
                .read(&mut right_buffer)
                .map_err(|error| io_error(&error))?;
            if left_read != right_read {
                return Ok(false);
            }
            if left_read == 0 {
                return Ok(true);
            }
            if left_buffer[..left_read] != right_buffer[..right_read] {
                return Ok(false);
            }
        }
    }

    fn commit_sync(&self, temporary_key: &str, final_key: &str) -> Result<(), ObjectStoreError> {
        if temporary_key == final_key {
            return Err(ObjectStoreError::Unavailable(
                "temporary and final storage keys must differ".into(),
            ));
        }
        let temporary_path = self.path_for(temporary_key)?;
        let final_path = self.path_for(final_key)?;
        let temporary_metadata = self.metadata_path_for(temporary_key)?;
        let final_metadata = self.metadata_path_for(final_key)?;
        self.ensure_safe_path(&temporary_path, false)?;
        self.ensure_safe_path(&temporary_metadata, false)?;
        self.ensure_safe_path(&final_path, true)?;
        self.ensure_safe_path(&final_metadata, true)?;
        if !fs::metadata(&temporary_path)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false)
        {
            return Err(ObjectStoreError::NotFound);
        }
        if !fs::metadata(&temporary_metadata)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false)
        {
            return Err(ObjectStoreError::Unavailable(
                "temporary object metadata is missing".into(),
            ));
        }
        self.ensure_parent_directory(&final_path)?;
        self.ensure_parent_directory(&final_metadata)?;

        let _guard = self.mutation_guard()?;
        let final_exists = fs::metadata(&final_path)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false);
        let final_metadata_exists = fs::metadata(&final_metadata)
            .map(|metadata| metadata.file_type().is_file())
            .unwrap_or(false);
        if final_exists {
            if final_metadata_exists
                && Self::files_equal(&temporary_path, &final_path)?
                && fs::read(temporary_metadata.as_path()).map_err(|error| map_read_error(&error))?
                    == fs::read(final_metadata.as_path()).map_err(|error| map_read_error(&error))?
            {
                Self::remove_if_exists(&temporary_path)?;
                Self::remove_if_exists(&temporary_metadata)?;
                return Ok(());
            }
            return Err(ObjectStoreError::AlreadyExists);
        }
        if final_metadata_exists {
            // Repair a sidecar left by an interrupted promotion before making
            // the final object name visible.
            Self::remove_if_exists(&final_metadata)?;
        }
        fs::hard_link(&temporary_metadata, &final_metadata)
            .map_err(|error| map_create_error(&error))?;
        if let Err(error) = fs::hard_link(&temporary_path, &final_path) {
            let _ = Self::remove_if_exists(&final_metadata);
            return Err(map_create_error(&error));
        }
        #[cfg(unix)]
        {
            Self::sync_parent(&final_metadata)?;
            Self::sync_parent(&final_path)?;
        }
        // Promotion is the commit point. Propagating cleanup failure keeps the
        // Media row recoverable so the reconciliation worker can retry it.
        Self::remove_if_exists(&temporary_path)?;
        Self::remove_if_exists(&temporary_metadata)?;
        Ok(())
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
        let path = self.metadata_path_for(key)?;
        self.ensure_safe_path(&path, false)?;
        match fs::read_to_string(path) {
            Ok(content_type) => Ok(Some(content_type)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error(&error)),
        }
    }

    fn collect_object_keys(
        root: &Path,
        directory: &Path,
        prefix: &str,
        cursor: Option<&str>,
        capacity: usize,
        keys: &mut BinaryHeap<String>,
    ) -> Result<(), ObjectStoreError> {
        for entry in fs::read_dir(directory).map_err(|error| io_error(&error))? {
            let entry = entry.map_err(|error| io_error(&error))?;
            let path = entry.path();
            if path == root.join(METADATA_DIRECTORY) || path == root.join(STAGING_DIRECTORY) {
                continue;
            }
            let file_type = entry.file_type().map_err(|error| io_error(&error))?;
            if file_type.is_dir() {
                Self::collect_object_keys(root, &path, prefix, cursor, capacity, keys)?;
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
                if key.starts_with(prefix) && cursor.is_none_or(|value| key.as_str() > value) {
                    if keys.len() < capacity {
                        keys.push(key);
                    } else if keys.peek().is_some_and(|largest| key < *largest) {
                        keys.pop();
                        keys.push(key);
                    }
                }
            }
        }
        Ok(())
    }

    fn list_sync(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ObjectPage, ObjectStoreError> {
        if limit == 0 || limit > 1_000 {
            return Err(ObjectStoreError::InvalidLimit);
        }
        if !prefix.is_empty() {
            // Listing prefixes commonly end in '/', while persisted object
            // keys must not. Validate the path portion without weakening key
            // canonicalization for read/write operations.
            let path_prefix = prefix.strip_suffix('/').unwrap_or(prefix);
            if path_prefix.is_empty() {
                return Err(ObjectStoreError::InvalidCursor);
            }
            Self::relative_path_for(path_prefix)?;
        }
        if let Some(cursor) = cursor {
            Self::relative_path_for(cursor)?;
            if !cursor.starts_with(prefix) {
                return Err(ObjectStoreError::InvalidCursor);
            }
        }
        self.ensure_safe_path(&self.root, false)?;
        if !self.root.is_dir() {
            return Err(ObjectStoreError::Unavailable(
                "storage root is unavailable".into(),
            ));
        }
        let mut keys = BinaryHeap::new();
        Self::collect_object_keys(
            &self.root,
            &self.root,
            prefix,
            cursor,
            limit.saturating_add(1),
            &mut keys,
        )?;
        let keys = keys.into_sorted_vec();
        let has_more = keys.len() > limit;
        let objects = keys
            .iter()
            .take(limit)
            .map(|key| {
                let path = self.path_for(key)?;
                self.ensure_safe_path(&path, false)?;
                let size = fs::metadata(path)
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
            next_cursor: has_more.then(|| objects.last().expect("non-empty page").key.clone()),
            objects,
        })
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
        let store = self.clone();
        let temporary_key = temporary_key.to_owned();
        let content = content.to_vec();
        let content_type = content_type.to_owned();
        tokio::task::spawn_blocking(move || {
            store.put_temporary_sync(&temporary_key, &content, &content_type)
        })
        .await
        .map_err(|error| {
            ObjectStoreError::Unavailable(format!("local temporary write task failed: {error}"))
        })?
    }

    async fn compose_temporary(
        &self,
        temporary_key: &str,
        source_keys: &[String],
        content_type: &str,
    ) -> Result<ComposedObject, ObjectStoreError> {
        let store = self.clone();
        let temporary_key = temporary_key.to_owned();
        let source_keys = source_keys.to_vec();
        let content_type = content_type.to_owned();
        tokio::task::spawn_blocking(move || {
            store.compose_new(&temporary_key, &source_keys, &content_type)
        })
        .await
        .map_err(|error| {
            ObjectStoreError::Unavailable(format!("local composition task failed: {error}"))
        })?
    }

    async fn commit_temporary(
        &self,
        temporary_key: &str,
        final_key: &str,
    ) -> Result<(), ObjectStoreError> {
        let store = self.clone();
        let temporary_key = temporary_key.to_owned();
        let final_key = final_key.to_owned();
        tokio::task::spawn_blocking(move || store.commit_sync(&temporary_key, &final_key))
            .await
            .map_err(|error| {
                ObjectStoreError::Unavailable(format!("local commit task failed: {error}"))
            })?
    }

    async fn read(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        let store = self.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || store.read(&key))
            .await
            .map_err(|error| {
                ObjectStoreError::Unavailable(format!("local read task failed: {error}"))
            })?
    }

    async fn read_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ObjectStoreError> {
        let store = self.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let path = store.path_for(&key)?;
            store.ensure_safe_path(&path, false)?;
            let mut file = fs::File::open(path).map_err(|error| map_read_error(&error))?;
            let size = file.metadata().map_err(|error| io_error(&error))?.len();
            if range.start >= range.end || range.end > size {
                return Err(ObjectStoreError::InvalidRange);
            }
            file.seek(SeekFrom::Start(range.start))
                .map_err(|error| io_error(&error))?;
            let length = usize::try_from(range.end - range.start)
                .map_err(|_| ObjectStoreError::InvalidRange)?;
            let mut content = vec![0; length];
            file.read_exact(&mut content)
                .map_err(|error| io_error(&error))?;
            Ok(content)
        })
        .await
        .map_err(|error| {
            ObjectStoreError::Unavailable(format!("local range read task failed: {error}"))
        })?
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, ObjectStoreError> {
        let store = self.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let path = store.path_for(&key)?;
            store.ensure_safe_path(&path, false)?;
            let (size, checksum_sha256) = Self::inspect_path(&path)?;
            let content_type = store.content_type_for(&key)?;
            Ok(ObjectMetadata {
                key,
                size,
                content_type,
                etag: None,
                version: None,
                checksum_sha256: Some(checksum_sha256),
                provider_metadata: BTreeMap::new(),
            })
        })
        .await
        .map_err(|error| {
            ObjectStoreError::Unavailable(format!("local head task failed: {error}"))
        })?
    }

    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ObjectPage, ObjectStoreError> {
        let store = self.clone();
        let prefix = prefix.to_owned();
        let cursor = cursor.map(ToOwned::to_owned);
        tokio::task::spawn_blocking(move || store.list_sync(&prefix, cursor.as_deref(), limit))
            .await
            .map_err(|error| {
                ObjectStoreError::Unavailable(format!("local list task failed: {error}"))
            })?
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        let store = self.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let _guard = store.mutation_guard()?;
            let object_path = store.path_for(&key)?;
            let metadata_path = store.metadata_path_for(&key)?;
            store.ensure_safe_path(&object_path, true)?;
            store.ensure_safe_path(&metadata_path, true)?;
            Self::remove_if_exists(&object_path)?;
            Self::remove_if_exists(&metadata_path)
        })
        .await
        .map_err(|error| {
            ObjectStoreError::Unavailable(format!("local delete task failed: {error}"))
        })?
    }

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        let store = self.clone();
        let key = key.to_owned();
        tokio::task::spawn_blocking(move || {
            let path = store.path_for(&key)?;
            store.ensure_safe_path(&path, true)?;
            Ok(fs::metadata(path)
                .map(|metadata| metadata.file_type().is_file())
                .unwrap_or(false))
        })
        .await
        .map_err(|error| {
            ObjectStoreError::Unavailable(format!("local existence task failed: {error}"))
        })?
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
        self.ensure_safe_path(&object_path, false)?;
        self.ensure_safe_path(&metadata_path, false)?;
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
        if session.state() == UploadSessionState::Completed {
            return Ok(());
        }
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
