use std::{ops::Range, path::Path};

use async_trait::async_trait;
use mediahub_adapter_local::LocalObjectStore;
use mediahub_adapter_s3::S3ObjectStore;
use mediahub_app::{
    ComposedObject, ObjectMetadata, ObjectPage, ObjectStore, ObjectStoreError, PreparedUpload,
    StoredUpload, UploadSessionStorage,
};
use mediahub_core::{MediaId, OffsetDateTime, UploadSession, UploadSessionId};

#[derive(Clone, Debug)]
pub(crate) enum RuntimeObjectStore {
    Local(LocalObjectStore),
    S3(S3ObjectStore),
}

impl RuntimeObjectStore {
    pub(crate) const fn local(store: LocalObjectStore) -> Self {
        Self::Local(store)
    }

    pub(crate) const fn s3(store: S3ObjectStore) -> Self {
        Self::S3(store)
    }

    pub(crate) fn local_root(&self) -> Option<&Path> {
        match self {
            Self::Local(store) => Some(store.root()),
            Self::S3(_) => None,
        }
    }

    pub(crate) const fn local_store(&self) -> Option<&LocalObjectStore> {
        match self {
            Self::Local(store) => Some(store),
            Self::S3(_) => None,
        }
    }

    #[cfg(test)]
    pub(crate) fn root(&self) -> &Path {
        self.local_root().expect("test storage is local")
    }

    pub(crate) async fn health_check(&self) -> Result<(), ObjectStoreError> {
        match self {
            Self::Local(store) if store.root().is_dir() => Ok(()),
            Self::Local(_) => Err(ObjectStoreError::Unavailable(
                "local storage root is unavailable".to_owned(),
            )),
            Self::S3(store) => store.exists("health/readiness").await.map(|_| ()),
        }
    }
}

#[async_trait]
impl ObjectStore for RuntimeObjectStore {
    fn backend_name(&self) -> &str {
        match self {
            Self::Local(store) => store.backend_name(),
            Self::S3(store) => store.backend_name(),
        }
    }

    async fn put_temporary(
        &self,
        temporary_key: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        match self {
            Self::Local(store) => {
                store
                    .put_temporary(temporary_key, content, content_type)
                    .await
            }
            Self::S3(store) => {
                store
                    .put_temporary(temporary_key, content, content_type)
                    .await
            }
        }
    }

    async fn compose_temporary(
        &self,
        temporary_key: &str,
        source_keys: &[String],
        content_type: &str,
    ) -> Result<ComposedObject, ObjectStoreError> {
        match self {
            Self::Local(store) => {
                store
                    .compose_temporary(temporary_key, source_keys, content_type)
                    .await
            }
            Self::S3(store) => {
                store
                    .compose_temporary(temporary_key, source_keys, content_type)
                    .await
            }
        }
    }

    async fn commit_temporary(
        &self,
        temporary_key: &str,
        final_key: &str,
    ) -> Result<(), ObjectStoreError> {
        match self {
            Self::Local(store) => store.commit_temporary(temporary_key, final_key).await,
            Self::S3(store) => store.commit_temporary(temporary_key, final_key).await,
        }
    }

    async fn read(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        match self {
            Self::Local(store) => ObjectStore::read(store, key).await,
            Self::S3(store) => ObjectStore::read(store, key).await,
        }
    }

    async fn read_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ObjectStoreError> {
        match self {
            Self::Local(store) => store.read_range(key, range).await,
            Self::S3(store) => store.read_range(key, range).await,
        }
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, ObjectStoreError> {
        match self {
            Self::Local(store) => store.head(key).await,
            Self::S3(store) => store.head(key).await,
        }
    }

    async fn list(
        &self,
        prefix: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ObjectPage, ObjectStoreError> {
        match self {
            Self::Local(store) => store.list(prefix, cursor, limit).await,
            Self::S3(store) => store.list(prefix, cursor, limit).await,
        }
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        match self {
            Self::Local(store) => store.delete(key).await,
            Self::S3(store) => store.delete(key).await,
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        match self {
            Self::Local(store) => store.exists(key).await,
            Self::S3(store) => store.exists(key).await,
        }
    }
}

#[async_trait]
impl UploadSessionStorage for RuntimeObjectStore {
    async fn prepare_upload(
        &self,
        upload_session_id: UploadSessionId,
        media_id: MediaId,
        expected_size: u64,
        expected_mime: &str,
        expires_at: OffsetDateTime,
    ) -> Result<PreparedUpload, ObjectStoreError> {
        match self {
            Self::Local(store) => {
                store
                    .prepare_upload(
                        upload_session_id,
                        media_id,
                        expected_size,
                        expected_mime,
                        expires_at,
                    )
                    .await
            }
            Self::S3(store) => {
                store
                    .prepare_upload(
                        upload_session_id,
                        media_id,
                        expected_size,
                        expected_mime,
                        expires_at,
                    )
                    .await
            }
        }
    }

    async fn inspect_upload(
        &self,
        session: &UploadSession,
    ) -> Result<StoredUpload, ObjectStoreError> {
        match self {
            Self::Local(store) => store.inspect_upload(session).await,
            Self::S3(store) => store.inspect_upload(session).await,
        }
    }

    async fn abort_upload(&self, session: &UploadSession) -> Result<(), ObjectStoreError> {
        match self {
            Self::Local(store) => store.abort_upload(session).await,
            Self::S3(store) => store.abort_upload(session).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc};

    use mediahub_adapter_local::LocalObjectStore;
    use mediahub_adapter_s3::{S3Config, S3ObjectStore};
    use mediahub_app::{ObjectStore, ObjectStoreError, UploadSessionStorage};
    use mediahub_core::{
        ApplicationId, BucketId, ClientMetadata, MediaId, NewUploadSession, OffsetDateTime,
        UploadSession, UploadSessionId,
    };
    use object_store::{
        Attribute, Attributes, ObjectStore as BackendObjectStore, PutOptions, memory::InMemory,
        path::Path,
    };

    use super::RuntimeObjectStore;

    #[tokio::test]
    async fn local_dispatches_object_upload_and_health_operations() {
        let root = test_directory("local");
        let runtime = RuntimeObjectStore::local(
            LocalObjectStore::new(&root).expect("create local object store"),
        );
        assert_eq!(runtime.backend_name(), "local");
        assert_eq!(runtime.local_root(), Some(root.as_path()));
        runtime.health_check().await.expect("healthy local store");

        runtime
            .put_temporary("temporary/object", b"hello", "text/plain")
            .await
            .expect("stage local object");
        runtime
            .commit_temporary("temporary/object", "committed/object")
            .await
            .expect("commit local object");
        assert_eq!(
            runtime.read("committed/object").await.expect("read object"),
            b"hello"
        );
        assert_eq!(
            runtime
                .read_range("committed/object", 1..4)
                .await
                .expect("read object range"),
            b"ell"
        );
        let metadata = runtime.head("committed/object").await.expect("head object");
        assert_eq!(metadata.size, 5);
        assert_eq!(metadata.content_type.as_deref(), Some("text/plain"));
        let page = runtime
            .list("committed", None, 10)
            .await
            .expect("list objects");
        assert_eq!(page.objects.len(), 1);
        assert_eq!(page.objects[0].key, metadata.key);
        assert_eq!(page.objects[0].size, metadata.size);
        assert_eq!(page.objects[0].content_type, metadata.content_type);
        assert_eq!(page.next_cursor, None);
        assert!(
            runtime
                .exists("committed/object")
                .await
                .expect("object existence")
        );

        let now = OffsetDateTime::now_utc();
        let expires_at = now + std::time::Duration::from_secs(600);
        let upload_id = UploadSessionId::new();
        let media_id = MediaId::new();
        let prepared = runtime
            .prepare_upload(upload_id, media_id, 5, "text/plain", expires_at)
            .await
            .expect("prepare local upload");
        assert_eq!(prepared.storage_backend, "local");
        assert_eq!(
            prepared.target.url,
            format!("/api/v1/uploads/{upload_id}/content")
        );
        let session = upload_session(
            upload_id,
            media_id,
            prepared.storage_backend,
            prepared.storage_key,
            expires_at,
            now,
        );
        runtime
            .put_temporary(session.storage_key(), b"hello", "text/plain")
            .await
            .expect("write local upload content");
        let inspected = runtime
            .inspect_upload(&session)
            .await
            .expect("inspect local upload");
        assert_eq!(inspected.size, 5);
        assert_eq!(inspected.mime, "text/plain");
        runtime.abort_upload(&session).await.expect("abort upload");
        runtime
            .abort_upload(&session)
            .await
            .expect("repeat upload abort");

        fs::remove_dir_all(&root).expect("remove local storage root");
        assert_eq!(
            runtime.health_check().await,
            Err(ObjectStoreError::Unavailable(
                "local storage root is unavailable".to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn s3_dispatches_health_inspection_and_abort_to_wrapped_backend() {
        let backend = Arc::new(InMemory::new());
        let runtime = RuntimeObjectStore::s3(
            S3ObjectStore::from_backend(backend.clone(), Some("tenant"))
                .expect("wrap in-memory S3 backend"),
        );
        assert_eq!(runtime.backend_name(), "s3");
        assert_eq!(runtime.local_root(), None);
        runtime.health_check().await.expect("healthy S3 backend");

        let now = OffsetDateTime::now_utc();
        let expires_at = now + std::time::Duration::from_secs(600);
        let upload_id = UploadSessionId::new();
        let media_id = MediaId::new();
        let storage_key = format!("objects/{media_id}");
        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentType, "text/plain".into());
        BackendObjectStore::put_opts(
            backend.as_ref(),
            &Path::from(format!("tenant/{storage_key}")),
            b"hello".to_vec().into(),
            PutOptions::from(attributes),
        )
        .await
        .expect("seed S3 upload");
        let session = upload_session(
            upload_id,
            media_id,
            "s3".to_owned(),
            storage_key,
            expires_at,
            now,
        );
        let inspected = runtime
            .inspect_upload(&session)
            .await
            .expect("inspect S3 upload");
        assert_eq!(inspected.size, 5);
        assert_eq!(inspected.mime, "text/plain");
        assert_eq!(
            inspected.sha256,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        assert_eq!(
            runtime
                .read_range(session.storage_key(), 1..4)
                .await
                .expect("read S3 range"),
            b"ell"
        );
        let metadata = runtime
            .head(session.storage_key())
            .await
            .expect("head S3 object");
        assert_eq!(metadata.size, 5);
        assert_eq!(metadata.content_type.as_deref(), Some("text/plain"));
        let page = runtime
            .list("objects", None, 10)
            .await
            .expect("list S3 objects");
        assert_eq!(page.objects.len(), 1);
        assert_eq!(page.objects[0].key, session.storage_key());
        runtime
            .abort_upload(&session)
            .await
            .expect("abort S3 upload");
        runtime
            .abort_upload(&session)
            .await
            .expect("repeat S3 upload abort");
    }

    #[tokio::test]
    async fn s3_prepare_returns_external_header_bound_target_offline() {
        let runtime = RuntimeObjectStore::s3(
            S3Config {
                bucket: "test-bucket".to_owned(),
                region: "us-east-1".to_owned(),
                endpoint: Some("https://storage.example.test".to_owned()),
                access_key_id: Some("AKIDEXAMPLE".to_owned()),
                secret_access_key: Some("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_owned()),
                session_token: Some("temporary-session-token".to_owned()),
                allow_http: false,
                virtual_hosted_style: false,
                prefix: Some("tenant".to_owned()),
            }
            .build()
            .expect("offline S3 configuration"),
        );
        let expires_at = OffsetDateTime::now_utc() + std::time::Duration::from_secs(600);
        let upload_id = UploadSessionId::new();
        let media_id = MediaId::new();
        let prepared = runtime
            .prepare_upload(upload_id, media_id, 5, "image/png", expires_at)
            .await
            .expect("prepare signed S3 upload");

        assert_eq!(prepared.storage_backend, "s3");
        assert_eq!(prepared.storage_key, format!("objects/{media_id}"));
        assert_eq!(prepared.target.method, "PUT");
        assert_eq!(prepared.target.headers["content-length"], "5");
        assert_eq!(prepared.target.headers["content-type"], "image/png");
        assert!(
            prepared
                .target
                .url
                .starts_with("https://storage.example.test/test-bucket/tenant/objects/")
        );
        assert!(
            prepared
                .target
                .url
                .contains("X-Amz-SignedHeaders=content-length%3Bcontent-type%3Bhost")
        );
        assert!(prepared.target.url.contains("X-Amz-Security-Token="));
        assert!(!prepared.target.url.starts_with("/api/v1/uploads/"));

        let recovered = runtime
            .prepare_upload(upload_id, media_id, 5, "image/png", expires_at)
            .await
            .expect("recover signed S3 upload target");
        assert_eq!(recovered.storage_backend, prepared.storage_backend);
        assert_eq!(recovered.storage_key, prepared.storage_key);
        assert_eq!(recovered.target.headers, prepared.target.headers);
    }

    fn upload_session(
        id: UploadSessionId,
        media_id: MediaId,
        storage_backend: String,
        storage_key: String,
        expires_at: OffsetDateTime,
        now: OffsetDateTime,
    ) -> UploadSession {
        UploadSession::new(
            NewUploadSession {
                id,
                media_id,
                application_id: ApplicationId::new(),
                bucket_id: BucketId::new(),
                object_key: "incoming/example.txt".to_owned(),
                original_name: Some("example.txt".to_owned()),
                display_name: "example.txt".to_owned(),
                extension: Some("txt".to_owned()),
                expected_size: 5,
                expected_mime: "text/plain".to_owned(),
                storage_backend,
                storage_key,
                visibility_override: None,
                media_expires_at: None,
                client_metadata: ClientMetadata::default(),
                session_expires_at: expires_at,
            },
            now,
        )
        .expect("upload session")
    }

    fn test_directory(label: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "mediahub-runtime-storage-{label}-{}",
            uuid::Uuid::new_v4()
        ))
    }
}
