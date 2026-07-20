// In-memory object-store implementation.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuotaSnapshot {
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub reserved_bytes: u64,
}

impl QuotaSnapshot {
    #[must_use]
    pub const fn available_bytes(self) -> u64 {
        self.quota_bytes
            .saturating_sub(self.used_bytes)
            .saturating_sub(self.reserved_bytes)
    }
}

#[derive(Clone, Default)]
pub struct InMemoryObjectStore {
    state: Arc<Mutex<ObjectStoreState>>,
}

#[derive(Default)]
struct ObjectStoreState {
    temporary: HashMap<String, StoredObject>,
    objects: HashMap<String, StoredObject>,
    fail_next_put: Option<ObjectStoreError>,
    fail_next_commit: Option<ObjectStoreError>,
    fail_next_abort: Option<ObjectStoreError>,
}

#[derive(Clone)]
struct StoredObject {
    content: Vec<u8>,
    content_type: String,
}

impl InMemoryObjectStore {
    pub fn fail_next_put(&self, error: ObjectStoreError) {
        self.lock()
            .expect("in-memory object store lock")
            .fail_next_put = Some(error);
    }

    pub fn fail_next_commit(&self, error: ObjectStoreError) {
        self.lock()
            .expect("in-memory object store lock")
            .fail_next_commit = Some(error);
    }

    pub fn fail_next_abort(&self, error: ObjectStoreError) {
        self.lock()
            .expect("in-memory object store lock")
            .fail_next_abort = Some(error);
    }

    #[must_use]
    pub fn temporary_count(&self) -> usize {
        self.lock()
            .expect("in-memory object store lock")
            .temporary
            .len()
    }

    #[must_use]
    pub fn object_count(&self) -> usize {
        self.lock()
            .expect("in-memory object store lock")
            .objects
            .len()
    }

    #[must_use]
    pub fn object_content(&self, key: &str) -> Option<Vec<u8>> {
        self.lock()
            .expect("in-memory object store lock")
            .objects
            .get(key)
            .map(|object| object.content.clone())
    }

    #[must_use]
    pub fn object_content_type(&self, key: &str) -> Option<String> {
        self.lock()
            .expect("in-memory object store lock")
            .objects
            .get(key)
            .map(|object| object.content_type.clone())
    }

    pub fn put_upload(
        &self,
        session: &UploadSession,
        content: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        let mut state = self.lock()?;
        if state.objects.contains_key(session.storage_key()) {
            return Err(ObjectStoreError::AlreadyExists);
        }
        state.objects.insert(
            session.storage_key().to_owned(),
            StoredObject {
                content: content.to_vec(),
                content_type: content_type.to_owned(),
            },
        );
        Ok(())
    }

    fn lock(&self) -> Result<MutexGuard<'_, ObjectStoreState>, ObjectStoreError> {
        self.state.lock().map_err(|_| {
            ObjectStoreError::Unavailable("in-memory object store lock poisoned".into())
        })
    }
}

#[async_trait]
impl ObjectStore for InMemoryObjectStore {
    fn backend_name(&self) -> &str {
        MEMORY_BACKEND
    }

    async fn put_temporary(
        &self,
        temporary_key: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        let mut state = self.lock()?;
        if let Some(error) = state.fail_next_put.take() {
            return Err(error);
        }
        if state.temporary.contains_key(temporary_key) {
            return Err(ObjectStoreError::AlreadyExists);
        }
        state.temporary.insert(
            temporary_key.to_owned(),
            StoredObject {
                content: content.to_vec(),
                content_type: content_type.to_owned(),
            },
        );
        Ok(())
    }

    async fn compose_temporary(
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
        let mut state = self.lock()?;
        if state.temporary.contains_key(temporary_key) {
            return Err(ObjectStoreError::AlreadyExists);
        }
        let mut content = Vec::new();
        for source_key in source_keys {
            let source = state
                .temporary
                .get(source_key)
                .ok_or(ObjectStoreError::NotFound)?;
            content.extend_from_slice(&source.content);
        }
        let size = u64::try_from(content.len()).map_err(|_| {
            ObjectStoreError::Unavailable("composed object size exceeds u64".into())
        })?;
        let sha256 = hex::encode(Sha256::digest(&content));
        state.temporary.insert(
            temporary_key.to_owned(),
            StoredObject {
                content,
                content_type: content_type.to_owned(),
            },
        );
        Ok(ComposedObject { size, sha256 })
    }

    async fn commit_temporary(
        &self,
        temporary_key: &str,
        final_key: &str,
    ) -> Result<(), ObjectStoreError> {
        let mut state = self.lock()?;
        if let Some(error) = state.fail_next_commit.take() {
            return Err(error);
        }
        if state.objects.contains_key(final_key) {
            return Err(ObjectStoreError::AlreadyExists);
        }
        let object = state
            .temporary
            .remove(temporary_key)
            .ok_or(ObjectStoreError::NotFound)?;
        state.objects.insert(final_key.to_owned(), object);
        Ok(())
    }

    async fn read(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        let state = self.lock()?;
        state
            .objects
            .get(key)
            .map(|object| object.content.clone())
            .ok_or(ObjectStoreError::NotFound)
    }

    async fn read_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ObjectStoreError> {
        let content = self.read(key).await?;
        let start = usize::try_from(range.start).map_err(|_| ObjectStoreError::InvalidRange)?;
        let end = usize::try_from(range.end).map_err(|_| ObjectStoreError::InvalidRange)?;
        if start >= end || end > content.len() {
            return Err(ObjectStoreError::InvalidRange);
        }
        Ok(content[start..end].to_vec())
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, ObjectStoreError> {
        let state = self.lock()?;
        let object = state.objects.get(key).ok_or(ObjectStoreError::NotFound)?;
        Ok(ObjectMetadata {
            key: key.to_owned(),
            size: object.content.len() as u64,
            content_type: Some(object.content_type.clone()),
            etag: None,
            version: None,
            checksum_sha256: Some(hex::encode(Sha256::digest(&object.content))),
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
        if cursor.is_some_and(|value| !value.starts_with(prefix)) {
            return Err(ObjectStoreError::InvalidCursor);
        }
        let state = self.lock()?;
        let mut keys = state
            .objects
            .keys()
            .filter(|key| key.starts_with(prefix))
            .filter(|key| cursor.is_none_or(|value| key.as_str() > value))
            .cloned()
            .collect::<Vec<_>>();
        keys.sort();
        let has_more = keys.len() > limit;
        keys.truncate(limit);
        let objects = keys
            .iter()
            .filter_map(|key| {
                state.objects.get(key).map(|object| ObjectMetadata {
                    key: key.clone(),
                    size: object.content.len() as u64,
                    content_type: Some(object.content_type.clone()),
                    etag: None,
                    version: None,
                    checksum_sha256: Some(hex::encode(Sha256::digest(&object.content))),
                    provider_metadata: BTreeMap::new(),
                })
            })
            .collect();
        Ok(ObjectPage {
            next_cursor: has_more.then(|| keys.last().expect("non-empty page").clone()),
            objects,
        })
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        let mut state = self.lock()?;
        state.temporary.remove(key);
        state.objects.remove(key);
        Ok(())
    }

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        let state = self.lock()?;
        Ok(state.temporary.contains_key(key) || state.objects.contains_key(key))
    }
}

#[async_trait]
impl UploadSessionStorage for InMemoryObjectStore {
    async fn prepare_upload(
        &self,
        upload_session_id: UploadSessionId,
        media_id: MediaId,
        _expected_size: u64,
        expected_mime: &str,
        expires_at: OffsetDateTime,
    ) -> Result<PreparedUpload, ObjectStoreError> {
        Ok(PreparedUpload {
            target: UploadTarget {
                method: "PUT".to_owned(),
                url: format!("memory://uploads/{upload_session_id}"),
                headers: BTreeMap::from([("content-type".to_owned(), expected_mime.to_owned())]),
                expires_at,
            },
            storage_backend: MEMORY_BACKEND.to_owned(),
            storage_key: format!("objects/{media_id}"),
        })
    }

    async fn inspect_upload(
        &self,
        session: &UploadSession,
    ) -> Result<StoredUpload, ObjectStoreError> {
        let state = self.lock()?;
        let object = state
            .objects
            .get(session.storage_key())
            .ok_or(ObjectStoreError::NotFound)?;
        Ok(StoredUpload {
            size: u64::try_from(object.content.len())
                .map_err(|_| ObjectStoreError::Unavailable("object exceeds u64 size".to_owned()))?,
            mime: object.content_type.clone(),
            sha256: hex::encode(Sha256::digest(&object.content)),
        })
    }

    async fn abort_upload(&self, session: &UploadSession) -> Result<(), ObjectStoreError> {
        if let Some(error) = self
            .lock()?
            .fail_next_abort
            .take()
        {
            return Err(error);
        }
        if session.state() == UploadSessionState::Completed {
            return Ok(());
        }
        self.delete(session.storage_key()).await
    }
}
