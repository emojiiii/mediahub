// S3 object-store adapter tests.

    use std::{
        fmt,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::UNIX_EPOCH,
    };

    use async_trait::async_trait;
    use futures_util::{StreamExt as _, stream::BoxStream};
    use mediahub_app::{ObjectStoreError, UploadSessionStorage};
    use mediahub_core::{
        ApplicationId, BucketId, ClientMetadata, MediaId, NewUploadSession, OffsetDateTime,
        UploadSession, UploadSessionId,
    };
    use object_store::{
        Attribute, Attributes, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload,
        ObjectMeta, ObjectStore as BackendObjectStore, ObjectStoreExt, PutMultipartOptions,
        PutOptions, PutPayload, PutResult, RenameOptions, Result as BackendResult,
        aws::AmazonS3Builder, memory::InMemory, path::Path,
    };
    use sha2::{Digest, Sha256};

    use super::{
        AwsPresignedPutSigner, COMMIT_COMPARE_CHUNK_SIZE, PresignedPutSigner, S3ObjectStore,
    };

    #[derive(Debug)]
    struct StubSigner;

    #[async_trait]
    impl PresignedPutSigner for StubSigner {
        async fn sign(
            &self,
            path: &Path,
            content_length: u64,
            content_type: &str,
            _expires_at: OffsetDateTime,
        ) -> Result<String, ObjectStoreError> {
            Ok(format!(
                "https://upload.example.test/{path}?length={content_length}&type={content_type}"
            ))
        }
    }

    #[derive(Debug, Default)]
    struct CommitProbeStore {
        inner: InMemory,
        copy_calls: AtomicUsize,
        fail_delete_calls: AtomicUsize,
        full_get_calls: AtomicUsize,
        range_get_calls: AtomicUsize,
    }

    impl CommitProbeStore {
        fn fail_next_delete(&self) {
            self.fail_delete_calls.store(1, Ordering::Relaxed);
        }
    }

    impl fmt::Display for CommitProbeStore {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("CommitProbeStore")
        }
    }

    #[async_trait]
    impl BackendObjectStore for CommitProbeStore {
        async fn put_opts(
            &self,
            location: &Path,
            payload: PutPayload,
            options: PutOptions,
        ) -> BackendResult<PutResult> {
            self.inner.put_opts(location, payload, options).await
        }

        async fn put_multipart_opts(
            &self,
            location: &Path,
            options: PutMultipartOptions,
        ) -> BackendResult<Box<dyn MultipartUpload>> {
            self.inner.put_multipart_opts(location, options).await
        }

        async fn get_opts(&self, location: &Path, options: GetOptions) -> BackendResult<GetResult> {
            if !options.head && options.range.is_none() {
                self.full_get_calls.fetch_add(1, Ordering::Relaxed);
                return Err(object_store::Error::NotSupported {
                    source: "test backend rejects unbounded GET requests".into(),
                });
            }
            if options.range.is_some() {
                self.range_get_calls.fetch_add(1, Ordering::Relaxed);
            }
            self.inner.get_opts(location, options).await
        }

        fn delete_stream(
            &self,
            locations: BoxStream<'static, BackendResult<Path>>,
        ) -> BoxStream<'static, BackendResult<Path>> {
            if self.fail_delete_calls.swap(0, Ordering::Relaxed) != 0 {
                return locations
                    .map(|location| match location {
                        Ok(_) => Err(object_store::Error::Generic {
                            store: "commit probe",
                            source: Box::new(std::io::Error::other("injected delete failure")),
                        }),
                        Err(error) => Err(error),
                    })
                    .boxed();
            }
            self.inner.delete_stream(locations)
        }

        fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, BackendResult<ObjectMeta>> {
            self.inner.list(prefix)
        }

        fn list_with_offset(
            &self,
            prefix: Option<&Path>,
            offset: &Path,
        ) -> BoxStream<'static, BackendResult<ObjectMeta>> {
            self.inner.list_with_offset(prefix, offset)
        }

        async fn list_with_delimiter(&self, prefix: Option<&Path>) -> BackendResult<ListResult> {
            self.inner.list_with_delimiter(prefix).await
        }

        async fn copy_opts(
            &self,
            from: &Path,
            to: &Path,
            options: CopyOptions,
        ) -> BackendResult<()> {
            self.copy_calls.fetch_add(1, Ordering::Relaxed);
            self.inner.copy_opts(from, to, options).await
        }

        async fn rename_opts(
            &self,
            from: &Path,
            to: &Path,
            options: RenameOptions,
        ) -> BackendResult<()> {
            self.inner.rename_opts(from, to, options).await
        }
    }

    #[tokio::test]
    async fn official_presigner_binds_length_type_path_and_expiry() {
        let backend = AmazonS3Builder::new()
            .with_bucket_name("test-bucket")
            .with_region("us-east-1")
            .with_endpoint("https://storage.example.test")
            .with_access_key_id("AKIDEXAMPLE")
            .with_secret_access_key("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY")
            .with_token("temporary-session-token")
            .build()
            .expect("test S3 backend");
        let signer = AwsPresignedPutSigner::new(backend, "us-east-1".to_owned());
        let now = UNIX_EPOCH + std::time::Duration::from_secs(1_750_000_000);
        let expires_at = OffsetDateTime::from_unix_timestamp(1_750_000_600).expect("test expiry");
        let path = Path::from("tenant/objects/media-id");

        let signed = signer
            .sign_at(&path, 5, "image/png", expires_at, now)
            .await
            .expect("signed PUT");
        let changed_length = signer
            .sign_at(&path, 6, "image/png", expires_at, now)
            .await
            .expect("signed PUT with changed length");
        let changed_type = signer
            .sign_at(&path, 5, "image/webp", expires_at, now)
            .await
            .expect("signed PUT with changed type");

        assert!(
            signed.starts_with("https://storage.example.test/test-bucket/tenant/objects/media-id?")
        );
        assert_eq!(query_parameter(&signed, "X-Amz-Expires"), Some("600"));
        assert_eq!(
            query_parameter(&signed, "X-Amz-Security-Token"),
            Some("temporary-session-token")
        );
        assert_eq!(
            query_parameter(&signed, "X-Amz-SignedHeaders"),
            Some("content-length%3Bcontent-type%3Bhost%3Bif-none-match")
        );
        assert_ne!(
            query_parameter(&signed, "X-Amz-Signature"),
            query_parameter(&changed_length, "X-Amz-Signature")
        );
        assert_ne!(
            query_parameter(&signed, "X-Amz-Signature"),
            query_parameter(&changed_type, "X-Amz-Signature")
        );

        let too_long = OffsetDateTime::from_unix_timestamp(1_750_000_901).expect("long expiry");
        assert_eq!(
            signer.sign_at(&path, 5, "image/png", too_long, now).await,
            Err(ObjectStoreError::Unavailable(
                "presigned PUT expiry must be between 1 and 900 seconds".to_owned()
            ))
        );
    }

    #[tokio::test]
    async fn upload_session_inspection_hashes_content_and_abort_is_idempotent() {
        let backend = Arc::new(InMemory::new());
        let store =
            S3ObjectStore::from_parts(backend.clone(), Some("tenant"), Some(Arc::new(StubSigner)))
                .expect("test store");
        let now = OffsetDateTime::now_utc();
        let expires_at = now + std::time::Duration::from_secs(600);
        let upload_session_id = UploadSessionId::new();
        let media_id = MediaId::new();
        let prepared = store
            .prepare_upload(upload_session_id, media_id, 5, "text/plain", expires_at)
            .await
            .expect("prepared upload");
        assert_eq!(prepared.target.headers["content-length"], "5");
        assert_eq!(prepared.target.headers["content-type"], "text/plain");
        assert_eq!(prepared.target.headers["if-none-match"], "*");
        assert!(prepared.target.url.contains("tenant/temporary/uploads/"));

        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentType, "text/plain".into());
        BackendObjectStore::put_opts(
            backend.as_ref(),
            &Path::from(format!("tenant/{}", prepared.storage_key)),
            b"hello".to_vec().into(),
            PutOptions::from(attributes),
        )
        .await
        .expect("simulated direct PUT");
        let session = upload_session(
            upload_session_id,
            media_id,
            prepared.storage_key,
            expires_at,
            now,
        );

        let inspected = store
            .inspect_upload(&session)
            .await
            .expect("inspected upload");
        assert_eq!(inspected.size, 5);
        assert_eq!(inspected.mime, "text/plain");
        assert_eq!(
            inspected.sha256,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        let final_upload_key = format!("objects/{media_id}");
        store
            .finalize_upload(&session, &final_upload_key)
            .await
            .expect("finalize verified upload");
        assert_eq!(
            mediahub_app::ObjectStore::read(&store, &final_upload_key)
                .await
                .expect("read finalized upload"),
            b"hello"
        );
        assert_eq!(
            store.inspect_upload(&session).await.expect("inspect finalized upload").sha256,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );

        store.abort_upload(&session).await.expect("first abort");
        store.abort_upload(&session).await.expect("repeated abort");
        assert!(
            !mediahub_app::ObjectStore::exists(&store, &final_upload_key)
                .await
                .expect("finalized upload cleanup")
        );
        assert_eq!(
            store.inspect_upload(&session).await,
            Err(ObjectStoreError::NotFound)
        );
    }

    #[tokio::test]
    async fn completed_upload_cleanup_removes_temporary_but_preserves_final_object() {
        let backend = Arc::new(InMemory::new());
        let store = S3ObjectStore::from_parts(
            backend.clone(),
            Some("tenant"),
            Some(Arc::new(StubSigner)),
        )
        .expect("test store");
        let now = OffsetDateTime::now_utc();
        let expires_at = now + std::time::Duration::from_secs(600);
        let upload_session_id = UploadSessionId::new();
        let media_id = MediaId::new();
        let prepared = store
            .prepare_upload(upload_session_id, media_id, 5, "text/plain", expires_at)
            .await
            .expect("prepared upload");
        let temporary_key = prepared.storage_key.clone();

        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentType, "text/plain".into());
        BackendObjectStore::put_opts(
            backend.as_ref(),
            &Path::from(format!("tenant/{temporary_key}")),
            b"hello".to_vec().into(),
            PutOptions::from(attributes),
        )
        .await
        .expect("simulated direct PUT");
        let mut session = upload_session(
            upload_session_id,
            media_id,
            prepared.storage_key,
            expires_at,
            now,
        );
        let final_key = format!("objects/{media_id}");
        store
            .finalize_upload(&session, &final_key)
            .await
            .expect("finalize upload");
        session
            .complete(now + std::time::Duration::from_secs(1))
            .expect("complete session");

        store
            .abort_upload(&session)
            .await
            .expect("clean completed upload");
        store
            .abort_upload(&session)
            .await
            .expect("repeat completed cleanup");

        assert!(
            !mediahub_app::ObjectStore::exists(&store, &temporary_key)
                .await
                .expect("temporary existence")
        );
        assert_eq!(
            mediahub_app::ObjectStore::read(&store, &final_key)
                .await
                .expect("final object survives cleanup"),
            b"hello"
        );
    }

    #[tokio::test]
    async fn commit_uses_server_side_copy_and_bounded_ranges_for_idempotent_retry() {
        let backend = Arc::new(CommitProbeStore::default());
        let store = S3ObjectStore::from_backend(backend.clone(), Some("tenant"))
            .expect("test object store");
        let content = vec![b'x'; (COMMIT_COMPARE_CHUNK_SIZE * 2 + 17) as usize];
        let temporary_key = "temporary/first";
        let retry_key = "temporary/retry";
        let conflict_key = "temporary/conflict";
        let final_key = "objects/final";

        mediahub_app::ObjectStore::put_temporary(
            &store,
            temporary_key,
            &content,
            "application/octet-stream",
        )
        .await
        .expect("initial temporary write");
        mediahub_app::ObjectStore::commit_temporary(&store, temporary_key, final_key)
            .await
            .expect("initial commit");
        assert_eq!(backend.copy_calls.load(Ordering::Relaxed), 1);
        assert_eq!(backend.full_get_calls.load(Ordering::Relaxed), 0);
        assert_eq!(backend.range_get_calls.load(Ordering::Relaxed), 0);
        let metadata = mediahub_app::ObjectStore::head(&store, final_key)
            .await
            .expect("committed metadata");
        assert_eq!(
            metadata.content_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(
            metadata.checksum_sha256,
            Some(hex::encode(Sha256::digest(&content)))
        );

        mediahub_app::ObjectStore::put_temporary(
            &store,
            retry_key,
            &content,
            "application/octet-stream",
        )
        .await
        .expect("retry temporary write");
        mediahub_app::ObjectStore::commit_temporary(&store, retry_key, final_key)
            .await
            .expect("idempotent retry");
        assert_eq!(backend.copy_calls.load(Ordering::Relaxed), 1);
        assert_eq!(backend.full_get_calls.load(Ordering::Relaxed), 0);
        assert_eq!(backend.range_get_calls.load(Ordering::Relaxed), 6);
        assert!(
            !mediahub_app::ObjectStore::exists(&store, retry_key)
                .await
                .expect("retry temporary existence")
        );

        mediahub_app::ObjectStore::put_temporary(
            &store,
            conflict_key,
            b"different",
            "application/octet-stream",
        )
        .await
        .expect("conflicting temporary write");
        assert_eq!(
            mediahub_app::ObjectStore::commit_temporary(&store, conflict_key, final_key).await,
            Err(ObjectStoreError::AlreadyExists)
        );
        assert_eq!(backend.copy_calls.load(Ordering::Relaxed), 1);
        assert_eq!(backend.full_get_calls.load(Ordering::Relaxed), 0);
        assert!(
            mediahub_app::ObjectStore::exists(&store, conflict_key)
                .await
                .expect("conflicting temporary existence")
        );
        let stored = backend
            .inner
            .get(&Path::from("tenant/objects/final"))
            .await
            .expect("committed object")
            .bytes()
            .await
            .expect("committed bytes");
        assert_eq!(stored.as_ref(), content);
    }

    #[tokio::test]
    async fn promotion_cleanup_failure_keeps_final_and_retries_temporary_deletion() {
        let backend = Arc::new(CommitProbeStore::default());
        let store = S3ObjectStore::from_backend(backend.clone(), Some("tenant"))
            .expect("test object store");
        let temporary_key = "temporary/cleanup-retry";
        let final_key = "objects/cleanup-retry";
        let content = b"durable promotion";
        mediahub_app::ObjectStore::put_temporary(
            &store,
            temporary_key,
            content,
            "text/plain",
        )
        .await
        .expect("temporary write");

        backend.fail_next_delete();
        assert!(matches!(
            mediahub_app::ObjectStore::commit_temporary(&store, temporary_key, final_key).await,
            Err(ObjectStoreError::Unavailable(_))
        ));
        let promoted = backend
            .inner
            .get(&Path::from("tenant/objects/cleanup-retry"))
            .await
            .expect("promoted final survives cleanup failure")
            .bytes()
            .await
            .expect("promoted final bytes");
        assert_eq!(promoted.as_ref(), content);
        assert!(
            mediahub_app::ObjectStore::exists(&store, temporary_key)
                .await
                .expect("temporary remains retryable")
        );

        mediahub_app::ObjectStore::commit_temporary(&store, temporary_key, final_key)
            .await
            .expect("retry removes temporary");
        assert!(
            !mediahub_app::ObjectStore::exists(&store, temporary_key)
                .await
                .expect("temporary cleaned")
        );
        assert_eq!(
            mediahub_app::ObjectStore::checksum_sha256(&store, final_key)
                .await
                .expect("final checksum"),
            hex::encode(Sha256::digest(content))
        );
        assert_eq!(backend.full_get_calls.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn checksum_sha256_streams_when_metadata_is_missing() {
        let backend = Arc::new(InMemory::new());
        let store = S3ObjectStore::from_backend(backend.clone(), Some("tenant"))
            .expect("test object store");
        let key = "objects/checksum-fallback";
        let content = b"checksum fallback content";
        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentType, "text/plain".into());
        BackendObjectStore::put_opts(
            backend.as_ref(),
            &Path::from(format!("tenant/{key}")),
            content.to_vec().into(),
            PutOptions::from(attributes),
        )
        .await
        .expect("raw object without checksum metadata");

        assert_eq!(
            mediahub_app::ObjectStore::head(&store, key)
                .await
                .expect("head object")
                .checksum_sha256,
            None
        );
        assert_eq!(
            mediahub_app::ObjectStore::checksum_sha256(&store, key)
                .await
                .expect("streaming checksum"),
            hex::encode(Sha256::digest(content))
        );
    }

    fn upload_session(
        id: UploadSessionId,
        media_id: MediaId,
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
                storage_backend: "s3".to_owned(),
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

    fn query_parameter<'a>(url: &'a str, name: &str) -> Option<&'a str> {
        url.split_once('?')?.1.split('&').find_map(|pair| {
            let (key, value) = pair.split_once('=')?;
            (key == name).then_some(value)
        })
    }
