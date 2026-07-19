// Local object-store tests.

    use mediahub_app::UploadSessionStorage;
    use mediahub_app::object_store_contract::verify_object_store_contract;
    use mediahub_core::{
        ApplicationId, BucketId, ClientMetadata, NewUploadSession, OffsetDateTime,
    };
    use uuid::Uuid;

    use super::*;

    fn test_root() -> PathBuf {
        std::env::temp_dir().join(format!("mediahub-local-adapter-{}", Uuid::new_v4()))
    }

    fn upload_session(prepared: &PreparedUpload, expires_at: OffsetDateTime) -> UploadSession {
        UploadSession::new(
            NewUploadSession {
                id: UploadSessionId::new(),
                media_id: MediaId::new(),
                application_id: ApplicationId::new(),
                bucket_id: BucketId::new(),
                object_key: "images/example.png".to_owned(),
                original_name: Some("example.png".to_owned()),
                display_name: "example".to_owned(),
                extension: Some("png".to_owned()),
                expected_size: 7,
                expected_mime: "image/png".to_owned(),
                storage_backend: prepared.storage_backend.clone(),
                storage_key: prepared.storage_key.clone(),
                visibility_override: None,
                media_expires_at: None,
                client_metadata: ClientMetadata::default(),
                session_expires_at: expires_at,
            },
            OffsetDateTime::UNIX_EPOCH,
        )
        .expect("valid upload session")
    }

    #[tokio::test]
    async fn satisfies_shared_object_store_contract() {
        let root = test_root();
        let store = LocalObjectStore::new(&root).expect("store initializes");
        let namespace = format!("contract/{}", Uuid::new_v4());

        verify_object_store_contract(&store, &namespace)
            .await
            .expect("local adapter satisfies the shared contract");

        fs::remove_dir_all(root).expect("test root cleanup");
    }

    #[tokio::test]
    async fn temporary_object_is_promoted_without_overwriting_existing_content() {
        let root = test_root();
        let store = LocalObjectStore::new(&root).expect("store initializes");
        store
            .put_temporary("temporary/first", b"first", "text/plain")
            .await
            .expect("temporary upload succeeds");
        store
            .commit_temporary("temporary/first", "objects/media-1")
            .await
            .expect("promotion succeeds");

        store
            .put_temporary("temporary/second", b"second", "text/plain")
            .await
            .expect("second temporary upload succeeds");
        assert_eq!(
            store
                .commit_temporary("temporary/second", "objects/media-1")
                .await,
            Err(ObjectStoreError::AlreadyExists)
        );
        assert_eq!(store.read("objects/media-1"), Ok(b"first".to_vec()));
        fs::remove_dir_all(root).expect("test root cleanup");
    }

    #[tokio::test]
    async fn traversal_keys_are_rejected() {
        let root = test_root();
        let store = LocalObjectStore::new(&root).expect("store initializes");
        assert!(matches!(
            store.put_temporary("../outside", b"no", "text/plain").await,
            Err(ObjectStoreError::Unavailable(_))
        ));
        fs::remove_dir_all(root).expect("test root cleanup");
    }

    #[tokio::test]
    async fn direct_upload_target_and_stored_facts_are_backend_owned() {
        let root = test_root();
        let store = LocalObjectStore::new(&root).expect("store initializes");
        let upload_id = UploadSessionId::new();
        let media_id = MediaId::new();
        let expires_at = OffsetDateTime::from_unix_timestamp(900).expect("valid timestamp");
        let prepared = store
            .prepare_upload(upload_id, media_id, 7, "image/png", expires_at)
            .await
            .expect("upload is prepared");

        assert_eq!(prepared.storage_backend, "local");
        assert_eq!(prepared.storage_key, format!("objects/{media_id}"));
        assert_eq!(prepared.target.method, "PUT");
        assert_eq!(
            prepared.target.url,
            format!("/api/v1/uploads/{upload_id}/content")
        );
        assert_eq!(
            prepared.target.headers,
            BTreeMap::from([
                ("content-length".to_owned(), "7".to_owned()),
                ("content-type".to_owned(), "image/png".to_owned()),
            ])
        );
        assert_eq!(prepared.target.expires_at, expires_at);

        store
            .put_temporary(&prepared.storage_key, b"content", "image/png")
            .await
            .expect("gateway upload succeeds");
        let session = upload_session(&prepared, expires_at);
        assert_eq!(
            store.inspect_upload(&session).await,
            Ok(StoredUpload {
                size: 7,
                mime: "image/png".to_owned(),
                sha256: "ed7002b439e9ac845f22357d822bac1444730fbdb6016d3ec9432297b9ec9f73"
                    .to_owned(),
            })
        );
        fs::remove_dir_all(root).expect("test root cleanup");
    }

    #[tokio::test]
    async fn streamed_upload_is_bounded_and_cleans_partial_files() {
        let root = test_root();
        let store = LocalObjectStore::new(&root).expect("store initializes");
        let key = "objects/streamed";
        store
            .put_temporary_stream(
                key,
                futures_util::stream::iter([
                    Ok::<Bytes, &str>(Bytes::from_static(b"abc")),
                    Ok(Bytes::from_static(b"defg")),
                ]),
                7,
                "application/octet-stream",
            )
            .await
            .expect("streamed upload succeeds");
        assert_eq!(store.read(key), Ok(b"abcdefg".to_vec()));
        assert_eq!(
            store.content_type_for(key),
            Ok(Some("application/octet-stream".to_owned()))
        );

        let short_key = "objects/short";
        assert!(matches!(
            store
                .put_temporary_stream(
                    short_key,
                    futures_util::stream::iter([Ok::<Bytes, &str>(Bytes::from_static(b"abc"))]),
                    4,
                    "text/plain",
                )
                .await,
            Err(LocalUploadError::SizeMismatch {
                expected: 4,
                actual: 3
            })
        ));
        assert!(!store.exists(short_key).await.expect("check partial object"));
        assert!(
            !store
                .metadata_path_for(short_key)
                .expect("metadata path")
                .exists()
        );

        let failed_key = "objects/disconnected";
        assert!(matches!(
            store
                .put_temporary_stream(
                    failed_key,
                    futures_util::stream::iter([
                        Ok(Bytes::from_static(b"ab")),
                        Err::<Bytes, &str>("connection closed"),
                    ]),
                    4,
                    "text/plain",
                )
                .await,
            Err(LocalUploadError::Stream(_))
        ));
        assert!(!store.exists(failed_key).await.expect("check failed object"));
        fs::remove_dir_all(root).expect("test root cleanup");
    }

    #[tokio::test]
    async fn abort_upload_removes_object_and_mime_metadata_idempotently() {
        let root = test_root();
        let store = LocalObjectStore::new(&root).expect("store initializes");
        let expires_at = OffsetDateTime::from_unix_timestamp(900).expect("valid timestamp");
        let prepared = store
            .prepare_upload(
                UploadSessionId::new(),
                MediaId::new(),
                7,
                "image/png",
                expires_at,
            )
            .await
            .expect("upload is prepared");
        let session = upload_session(&prepared, expires_at);
        store
            .put_temporary(session.storage_key(), b"content", "image/png")
            .await
            .expect("gateway upload succeeds");

        store
            .abort_upload(&session)
            .await
            .expect("first abort succeeds");
        store
            .abort_upload(&session)
            .await
            .expect("repeated abort succeeds");
        assert!(
            !store
                .exists(session.storage_key())
                .await
                .expect("existence check succeeds")
        );
        assert!(
            !store
                .metadata_path_for(session.storage_key())
                .expect("valid metadata path")
                .exists()
        );
        assert_eq!(
            store.inspect_upload(&session).await,
            Err(ObjectStoreError::NotFound)
        );
        fs::remove_dir_all(root).expect("test root cleanup");
    }
