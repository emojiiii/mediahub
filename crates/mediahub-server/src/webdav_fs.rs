// ObjectVersion-backed WebDAV guarded filesystem operations.

#[derive(Clone)]
struct MediaHubDavFs {
    repository: PostgresRepository,
    object_store: RuntimeObjectStore,
    gc_grace: time::Duration,
}

impl GuardedFileSystem<DavCredentials> for MediaHubDavFs {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        Box::pin(async move {
            let DavResource::Object {
                bucket_name,
                object_key,
                collection: false,
            } = DavResource::parse(path, credentials)?
            else {
                return Err(FsError::Forbidden);
            };
            self.find_bucket(credentials, &bucket_name).await?;
            if options.write || options.create || options.append || options.truncate {
                credentials.require("media:upload")?;
                if credentials.method != axum::http::Method::PUT || options.append {
                    return Err(FsError::NotImplemented);
                }
                if let Some(size) = options.size
                    && size > MAX_REQUEST_BYTES as u64
                {
                    return Err(FsError::TooLarge);
                }
                if options.create_new {
                    match self
                        .head_object(credentials, &bucket_name, &object_key)
                        .await
                    {
                        Ok(_) => return Err(FsError::Exists),
                        Err(FsError::NotFound) => {}
                        Err(error) => return Err(error),
                    }
                }
                let content_type = credentials
                    .content_type
                    .clone()
                    .unwrap_or_else(|| guess_mime(&object_key));
                Ok(Box::new(MediaHubDavFile::write(
                    self.clone(),
                    credentials.clone(),
                    bucket_name,
                    object_key,
                    content_type,
                    options.size,
                )) as Box<dyn DavFile>)
            } else {
                credentials.require("media:read")?;
                let version = self
                    .head_object(credentials, &bucket_name, &object_key)
                    .await?;
                Ok(
                    Box::new(MediaHubDavFile::read(self.object_store.clone(), version))
                        as Box<dyn DavFile>,
                )
            }
        })
    }

    fn read_dir<'a>(
        &'a self,
        path: &'a DavPath,
        _meta: ReadDirMeta,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, FsStream<Box<dyn DavDirEntry>>> {
        Box::pin(async move {
            let entries = match DavResource::parse(path, credentials)? {
                DavResource::Root => {
                    credentials.require_any(&[
                        "application:read",
                        "bucket:list",
                        "media:list",
                        "media:read",
                        "media:upload",
                    ])?;
                    vec![DavEntry::directory(
                        credentials.application.app_id.as_bytes().to_vec(),
                        UNIX_EPOCH,
                    )]
                }
                DavResource::Application => {
                    credentials.require_any(&[
                        "bucket:list",
                        "media:list",
                        "media:read",
                        "media:upload",
                    ])?;
                    self.repository
                        .list_s3_buckets(credentials.application.id)
                        .await
                        .map_err(map_repository_error)?
                        .into_iter()
                        .map(|bucket| {
                            DavEntry::directory(
                                bucket.name().as_bytes().to_vec(),
                                to_system_time(bucket.configuration().updated_at()),
                            )
                        })
                        .collect()
                }
                DavResource::Bucket { bucket_name } => {
                    credentials.require_any(&["media:list", "media:read", "media:upload"])?;
                    let bucket = self.find_bucket(credentials, &bucket_name).await?;
                    let entries = self
                        .list_directory_entries(credentials.application.id, bucket.id(), "")
                        .await?;
                    if credentials.method == axum::http::Method::DELETE && !entries.is_empty() {
                        return Err(FsError::Exists);
                    }
                    entries
                }
                DavResource::Object {
                    bucket_name,
                    object_key,
                    ..
                } => {
                    credentials.require_any(&["media:list", "media:read", "media:upload"])?;
                    let bucket = self.find_bucket(credentials, &bucket_name).await?;
                    let prefix = directory_prefix(&object_key);
                    let entries = self
                        .list_directory_entries(credentials.application.id, bucket.id(), &prefix)
                        .await?;
                    if entries.is_empty() {
                        return Err(FsError::NotFound);
                    }
                    entries
                }
            };
            let entries = entries
                .into_iter()
                .map(|entry| Ok(Box::new(entry) as Box<dyn DavDirEntry>));
            Ok(Box::pin(stream::iter(entries)) as FsStream<Box<dyn DavDirEntry>>)
        })
    }

    fn metadata<'a>(
        &'a self,
        path: &'a DavPath,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, Box<dyn DavMetaData>> {
        Box::pin(async move {
            let metadata = match DavResource::parse(path, credentials)? {
                DavResource::Root | DavResource::Application => DavMetadata::directory(UNIX_EPOCH),
                DavResource::Bucket { bucket_name } => {
                    let bucket = self.find_bucket(credentials, &bucket_name).await?;
                    DavMetadata::directory(to_system_time(bucket.configuration().updated_at()))
                }
                DavResource::Object {
                    bucket_name,
                    object_key,
                    collection,
                } => {
                    let bucket = self.find_bucket(credentials, &bucket_name).await?;
                    if !collection {
                        match self
                            .head_object(credentials, &bucket_name, &object_key)
                            .await
                        {
                            Ok(version) => {
                                credentials.require_any(&["media:list", "media:read"])?;
                                DavMetadata::from_object_version(&version)?
                            }
                            Err(FsError::NotFound) => {
                                credentials.require_any(&[
                                    "media:list",
                                    "media:read",
                                    "media:upload",
                                ])?;
                                let prefix = directory_prefix(&object_key);
                                if !self
                                    .directory_exists(
                                        credentials.application.id,
                                        bucket.id(),
                                        &prefix,
                                    )
                                    .await?
                                {
                                    return Err(FsError::NotFound);
                                }
                                DavMetadata::directory(UNIX_EPOCH)
                            }
                            Err(error) => return Err(error),
                        }
                    } else {
                        credentials.require_any(&["media:list", "media:read", "media:upload"])?;
                        let prefix = directory_prefix(&object_key);
                        if !self
                            .directory_exists(credentials.application.id, bucket.id(), &prefix)
                            .await?
                        {
                            return Err(FsError::NotFound);
                        }
                        DavMetadata::directory(UNIX_EPOCH)
                    }
                }
            };
            Ok(Box::new(metadata) as Box<dyn DavMetaData>)
        })
    }

    fn create_dir<'a>(
        &'a self,
        path: &'a DavPath,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, ()> {
        Box::pin(async move {
            match DavResource::parse(path, credentials)? {
                DavResource::Bucket { bucket_name } => {
                    credentials.require("bucket:manage")?;
                    if self
                        .repository
                        .find_s3_bucket(credentials.application.id, &bucket_name)
                        .await
                        .map_err(map_repository_error)?
                        .is_some()
                    {
                        return Err(FsError::Exists);
                    }
                    let bucket = S3Bucket::new(
                        BucketId::new(),
                        credentials.application.id,
                        bucket_name,
                        "us-east-1",
                        false,
                        None,
                        OffsetDateTime::now_utc(),
                    )
                    .map_err(map_s3_model_error)?;
                    self.repository
                        .create_s3_bucket(&bucket)
                        .await
                        .map_err(map_repository_error)?;
                    self.record_audit(
                        credentials,
                        "bucket.created",
                        "bucket",
                        bucket.id().to_string(),
                        serde_json::json!({ "name": bucket.name(), "protocol": "webdav" }),
                    )
                    .await;
                    Ok(())
                }
                DavResource::Object {
                    bucket_name,
                    object_key,
                    ..
                } => {
                    credentials.require("media:upload")?;
                    let bucket = self.find_bucket(credentials, &bucket_name).await?;
                    let prefix = directory_prefix(&object_key);
                    if self
                        .directory_exists(credentials.application.id, bucket.id(), &prefix)
                        .await?
                    {
                        return Err(FsError::Exists);
                    }
                    // PrismArk directories are prefixes; MKCOL does not create a marker object.
                    Ok(())
                }
                DavResource::Root | DavResource::Application => Err(FsError::Forbidden),
            }
        })
    }

    fn remove_file<'a>(
        &'a self,
        path: &'a DavPath,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, ()> {
        Box::pin(async move {
            credentials.require("media:delete")?;
            let DavResource::Object {
                bucket_name,
                object_key,
                collection: false,
            } = DavResource::parse(path, credentials)?
            else {
                return Err(FsError::Forbidden);
            };
            // DAV DELETE of a missing key is 404 even though S3 DeleteObject is idempotent.
            self.head_object(credentials, &bucket_name, &object_key)
                .await?;
            let receipt = self
                .object_service()
                .delete(&DeleteObjectRequest {
                    application_id: credentials.application.id,
                    bucket_name: bucket_name.clone(),
                    object_key: object_key.clone(),
                    version_id: None,
                    bypass_governance: false,
                    deleted_by: format!("webdav:{}", credentials.access_key_id),
                })
                .await
                .map_err(map_s3_object_error)?;
            self.record_audit(
                credentials,
                "dav.object.deleted",
                "object",
                format!("{bucket_name}/{object_key}"),
                serde_json::json!({
                    "bucket": bucket_name,
                    "object_key": object_key,
                    "delete_marker": receipt.delete_marker,
                    "version_id": receipt.version_id.as_ref().map(|version| version.as_str()),
                    "protocol": "webdav",
                }),
            )
            .await;
            Ok(())
        })
    }

    fn remove_dir<'a>(
        &'a self,
        path: &'a DavPath,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, ()> {
        Box::pin(async move {
            match DavResource::parse(path, credentials)? {
                DavResource::Bucket { bucket_name } => {
                    credentials.require("bucket:manage")?;
                    let deleted = self
                        .repository
                        .delete_s3_bucket(credentials.application.id, &bucket_name)
                        .await
                        .map_err(map_repository_error)?;
                    if !deleted {
                        return Err(FsError::NotFound);
                    }
                    self.record_audit(
                        credentials,
                        "bucket.deleted",
                        "bucket",
                        bucket_name.clone(),
                        serde_json::json!({ "name": bucket_name, "protocol": "webdav" }),
                    )
                    .await;
                    Ok(())
                }
                DavResource::Object { bucket_name, .. } => {
                    credentials.require("media:delete")?;
                    self.find_bucket(credentials, &bucket_name).await?;
                    // dav-server removes discovered children before this prefix-only callback.
                    Ok(())
                }
                DavResource::Root | DavResource::Application => Err(FsError::Forbidden),
            }
        })
    }

    fn copy<'a>(
        &'a self,
        from: &'a DavPath,
        to: &'a DavPath,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, ()> {
        Box::pin(async move {
            credentials.require("media:read")?;
            credentials.require("media:upload")?;
            let DavResource::Object {
                bucket_name: source_bucket,
                object_key: source_key,
                collection: false,
            } = DavResource::parse(from, credentials)?
            else {
                return Err(FsError::NotImplemented);
            };
            let DavResource::Object {
                bucket_name: destination_bucket,
                object_key: destination_key,
                collection: false,
            } = DavResource::parse(to, credentials)?
            else {
                return Err(FsError::NotImplemented);
            };
            if source_bucket == destination_bucket && source_key == destination_key {
                return Err(FsError::Exists);
            }
            let receipt = self
                .copy_object(
                    credentials,
                    &source_bucket,
                    &source_key,
                    &destination_bucket,
                    &destination_key,
                )
                .await?;
            self.record_audit(
                credentials,
                "dav.object.copied",
                "object_version",
                receipt.version.id().to_string(),
                serde_json::json!({
                    "source_bucket": source_bucket,
                    "source_key": source_key,
                    "destination_bucket": destination_bucket,
                    "destination_key": destination_key,
                    "object_id": receipt.object.id().to_string(),
                    "version_id": receipt.version.external_version_id().as_str(),
                    "protocol": "webdav",
                }),
            )
            .await;
            Ok(())
        })
    }

    fn rename<'a>(
        &'a self,
        _from: &'a DavPath,
        _to: &'a DavPath,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, ()> {
        Box::pin(async move {
            credentials.require("media:read")?;
            credentials.require("media:upload")?;
            credentials.require("media:delete")?;
            Err(FsError::NotImplemented)
        })
    }

    fn get_quota<'a>(
        &'a self,
        _credentials: &'a DavCredentials,
    ) -> FsFuture<'a, (u64, Option<u64>)> {
        Box::pin(async move { Err(FsError::NotImplemented) })
    }
}

impl MediaHubDavFs {
    fn object_service(&self) -> DavObjectService {
        S3ObjectService::new(
            self.repository.clone(),
            self.repository.clone(),
            self.repository.clone(),
            self.object_store.clone(),
            SystemClock,
        )
        .with_gc_grace(self.gc_grace)
        .expect("WebDAV GC grace is validated during server configuration")
    }

    async fn find_bucket(
        &self,
        credentials: &DavCredentials,
        bucket_name: &str,
    ) -> FsResult<S3Bucket> {
        self.repository
            .find_s3_bucket(credentials.application.id, bucket_name)
            .await
            .map_err(map_repository_error)?
            .ok_or(FsError::NotFound)
    }

    async fn head_object(
        &self,
        credentials: &DavCredentials,
        bucket_name: &str,
        object_key: &str,
    ) -> FsResult<ObjectVersion> {
        self.object_service()
            .head(&S3ObjectRequest {
                application_id: credentials.application.id,
                bucket_name: bucket_name.to_owned(),
                object_key: object_key.to_owned(),
                version_id: None,
            })
            .await
            .map(|receipt| receipt.version)
            .map_err(map_s3_object_error)
    }

    async fn list_directory_entries(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
    ) -> FsResult<Vec<DavEntry>> {
        let mut cursor = None;
        let mut entries = BTreeMap::<String, DavEntry>::new();
        loop {
            let page = self
                .repository
                .list_current_s3_objects(
                    application_id,
                    &S3ObjectListQuery {
                        bucket_id,
                        prefix: prefix.to_owned(),
                        start_after: cursor,
                        delimiter: true,
                        limit: PAGE_SIZE,
                    },
                )
                .await
                .map_err(map_repository_error)?;
            for common_prefix in page.common_prefixes {
                let Some(name) = common_prefix
                    .strip_prefix(prefix)
                    .and_then(|value| value.strip_suffix('/'))
                else {
                    return Err(FsError::GeneralFailure);
                };
                if !name.is_empty() {
                    entries.entry(name.to_owned()).or_insert_with(|| {
                        DavEntry::directory(name.as_bytes().to_vec(), UNIX_EPOCH)
                    });
                }
            }
            for item in page.items {
                let Some(name) = item.key.strip_prefix(prefix) else {
                    return Err(FsError::GeneralFailure);
                };
                if name.is_empty() || name.contains('/') {
                    return Err(FsError::GeneralFailure);
                }
                entries.insert(
                    name.to_owned(),
                    DavEntry::file(name.as_bytes().to_vec(), &item.version)?,
                );
            }
            let Some(next_cursor) = page.next_cursor else {
                break;
            };
            cursor = Some(next_cursor);
        }
        Ok(entries.into_values().collect())
    }

    async fn directory_exists(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: &str,
    ) -> FsResult<bool> {
        self.repository
            .list_current_s3_objects(
                application_id,
                &S3ObjectListQuery {
                    bucket_id,
                    prefix: prefix.to_owned(),
                    start_after: None,
                    delimiter: false,
                    limit: 1,
                },
            )
            .await
            .map(|page| !page.items.is_empty())
            .map_err(map_repository_error)
    }

    async fn put_object(
        &self,
        upload: DavUpload,
        credentials: &DavCredentials,
    ) -> FsResult<CompletePutObjectReceipt> {
        let service = self.object_service();
        let expected_size = upload.content.len() as u64;
        let begun = service
            .begin_put(&BeginPutObjectRequest {
                application_id: credentials.application.id,
                bucket_name: upload.bucket_name,
                object_key: upload.object_key,
                expected_size_bytes: expected_size,
                content_type: Some(upload.content_type.clone()),
                user_metadata: serde_json::json!({}),
                object_tags: mediahub_core::S3ObjectTagSet::empty(),
                expires_at: None,
            })
            .await
            .map_err(map_s3_object_error)?;
        let intent_id = begun.intent.id();
        let stream = once(async move { Ok::<Bytes, Infallible>(Bytes::from(upload.content)) });
        let streamed = match self
            .object_store
            .put_temporary_stream(
                begun.intent.temporary_storage_key(),
                stream,
                expected_size,
                &upload.content_type,
            )
            .await
        {
            Ok(streamed) => streamed,
            Err(error) => {
                self.abort_staged_put(&service, credentials.application.id, intent_id)
                    .await;
                return Err(map_streaming_upload_error(error));
            }
        };
        match service
            .complete_put(&CompletePutObjectRequest {
                application_id: credentials.application.id,
                intent_id,
                streamed,
                created_by: format!("webdav:{}", credentials.access_key_id),
                source_protocol: SourceProtocol::Dav,
            })
            .await
        {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                self.abort_staged_put(&service, credentials.application.id, intent_id)
                    .await;
                Err(map_s3_object_error(error))
            }
        }
    }

    async fn copy_object(
        &self,
        credentials: &DavCredentials,
        source_bucket: &str,
        source_key: &str,
        destination_bucket: &str,
        destination_key: &str,
    ) -> FsResult<CompletePutObjectReceipt> {
        self.find_bucket(credentials, destination_bucket).await?;
        let source = self
            .head_object(credentials, source_bucket, source_key)
            .await?;
        let payload = stored_payload(&source)?;
        let service = self.object_service();
        let begun = service
            .begin_put(&BeginPutObjectRequest {
                application_id: credentials.application.id,
                bucket_name: destination_bucket.to_owned(),
                object_key: destination_key.to_owned(),
                expected_size_bytes: payload.size_bytes(),
                content_type: Some(
                    payload
                        .content_type()
                        .unwrap_or("application/octet-stream")
                        .to_owned(),
                ),
                user_metadata: payload.user_metadata().clone(),
                object_tags: mediahub_core::S3ObjectTagSet::empty(),
                expires_at: None,
            })
            .await
            .map_err(map_s3_object_error)?;
        let intent_id = begun.intent.id();
        let streamed = match self
            .object_store
            .copy_committed_to_temporary(
                payload.storage_key(),
                payload.size_bytes(),
                None,
                begun.intent.temporary_storage_key(),
                payload.content_type().unwrap_or("application/octet-stream"),
            )
            .await
        {
            Ok(streamed) => streamed,
            Err(error) => {
                self.abort_staged_put(&service, credentials.application.id, intent_id)
                    .await;
                return Err(map_streaming_upload_error(error));
            }
        };
        match service
            .complete_put(&CompletePutObjectRequest {
                application_id: credentials.application.id,
                intent_id,
                streamed,
                created_by: format!("webdav:{}", credentials.access_key_id),
                source_protocol: SourceProtocol::Dav,
            })
            .await
        {
            Ok(receipt) => Ok(receipt),
            Err(error) => {
                self.abort_staged_put(&service, credentials.application.id, intent_id)
                    .await;
                Err(map_s3_object_error(error))
            }
        }
    }

    async fn abort_staged_put(
        &self,
        service: &DavObjectService,
        application_id: ApplicationId,
        intent_id: mediahub_core::UploadIntentId,
    ) {
        if let Err(error) = service
            .abort_staged_put(&AbortStagedPutRequest {
                application_id,
                intent_id,
            })
            .await
        {
            warn!(%intent_id, error = %error, "failed to abort rejected WebDAV staged PUT");
        }
    }

    async fn content_type_for_uri(
        &self,
        uri: &axum::http::Uri,
        credentials: &DavCredentials,
    ) -> FsResult<Option<String>> {
        let mut path = DavPath::new(uri.path()).map_err(|_| FsError::Forbidden)?;
        path.set_prefix("/dav").map_err(|_| FsError::Forbidden)?;
        let DavResource::Object {
            bucket_name,
            object_key,
            collection: false,
        } = DavResource::parse(&path, credentials)?
        else {
            return Ok(None);
        };
        let version = self
            .head_object(credentials, &bucket_name, &object_key)
            .await?;
        Ok(Some(
            stored_payload(&version)?
                .content_type()
                .unwrap_or("application/octet-stream")
                .to_owned(),
        ))
    }

    async fn record_audit(
        &self,
        credentials: &DavCredentials,
        action: &str,
        target_type: &str,
        target_id: String,
        summary: serde_json::Value,
    ) {
        let event = AuditEvent {
            id: uuid::Uuid::now_v7().to_string(),
            application_id: credentials.application.id,
            actor_type: "access_key".to_owned(),
            actor_id: credentials.access_key_id.clone(),
            action: action.to_owned(),
            target_type: target_type.to_owned(),
            target_id,
            request_id: credentials.request_id.clone(),
            summary,
            created_at: OffsetDateTime::now_utc(),
        };
        if let Err(error) = self.repository.record_audit(&event).await {
            warn!(error = %error, action, "failed to record WebDAV audit event");
        }
    }
}
