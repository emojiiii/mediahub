// WebDAV guarded filesystem operations.

#[derive(Clone)]
struct MediaHubDavFs {
    repository: PostgresRepository,
    object_store: RuntimeObjectStore,
}

impl GuardedFileSystem<DavCredentials> for MediaHubDavFs {
    fn open<'a>(
        &'a self,
        path: &'a DavPath,
        options: OpenOptions,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, Box<dyn DavFile>> {
        Box::pin(async move {
            let resource = DavResource::parse(path, credentials)?;
            let DavResource::Object {
                bucket_name,
                object_key,
                collection: false,
            } = resource
            else {
                return Err(FsError::Forbidden);
            };
            let bucket = self.find_bucket(credentials, &bucket_name).await?;
            if options.write || options.create || options.append || options.truncate {
                credentials.require("media:upload")?;
                if options.append {
                    return Err(FsError::Forbidden);
                }
                if let Some(size) = options.size
                    && size > MAX_REQUEST_BYTES as u64
                {
                    return Err(FsError::TooLarge);
                }
                if self
                    .find_media(credentials.application.id, bucket.id(), &object_key)
                    .await?
                    .is_some()
                {
                    return Err(FsError::Exists);
                }
                let mime = credentials
                    .content_type
                    .clone()
                    .unwrap_or_else(|| guess_mime(&object_key));
                if let Some(size) = options.size {
                    bucket
                        .validate_upload(&mime, size)
                        .map_err(map_domain_error)?;
                }
                let file = MediaHubDavFile::write(
                    self.clone(),
                    credentials.clone(),
                    credentials.application.id,
                    bucket.id(),
                    object_key,
                    mime,
                    options.size,
                );
                Ok(Box::new(file) as Box<dyn DavFile>)
            } else {
                credentials.require("media:read")?;
                let media = self
                    .find_media(credentials.application.id, bucket.id(), &object_key)
                    .await?
                    .ok_or(FsError::NotFound)?;
                Ok(
                    Box::new(MediaHubDavFile::read(self.object_store.clone(), media))
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
            let resource = DavResource::parse(path, credentials)?;
            let entries = match resource {
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
                        .list_buckets(credentials.application.id)
                        .await
                        .map_err(|_| FsError::GeneralFailure)?
                        .into_iter()
                        .map(|bucket| {
                            DavEntry::directory(
                                bucket.name().as_bytes().to_vec(),
                                to_system_time(bucket.updated_at()),
                            )
                        })
                        .collect()
                }
                DavResource::Bucket { bucket_name } => {
                    credentials.require_any(&["media:list", "media:read", "media:upload"])?;
                    let bucket = self.find_bucket(credentials, &bucket_name).await?;
                    let media = self
                        .list_media(credentials.application.id, bucket.id(), None)
                        .await?;
                    if credentials.method == axum::http::Method::DELETE && !media.is_empty() {
                        return Err(FsError::Exists);
                    }
                    direct_entries(&media, "")
                }
                DavResource::Object {
                    bucket_name,
                    object_key,
                    ..
                } => {
                    credentials.require_any(&["media:list", "media:read", "media:upload"])?;
                    let bucket = self.find_bucket(credentials, &bucket_name).await?;
                    let prefix = directory_prefix(&object_key);
                    let media = self
                        .list_media(
                            credentials.application.id,
                            bucket.id(),
                            Some(prefix.clone()),
                        )
                        .await?;
                    if media.is_empty() {
                        return Err(FsError::NotFound);
                    }
                    direct_entries(&media, &prefix)
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
            let resource = DavResource::parse(path, credentials)?;
            let metadata = match resource {
                DavResource::Root | DavResource::Application => DavMetadata::directory(UNIX_EPOCH),
                DavResource::Bucket { bucket_name } => {
                    let bucket = self.find_bucket(credentials, &bucket_name).await?;
                    DavMetadata::directory(to_system_time(bucket.updated_at()))
                }
                DavResource::Object {
                    bucket_name,
                    object_key,
                    collection,
                } => {
                    let bucket = self.find_bucket(credentials, &bucket_name).await?;
                    if !collection
                        && let Some(media) = self
                            .find_media(credentials.application.id, bucket.id(), &object_key)
                            .await?
                    {
                        credentials.require_any(&["media:list", "media:read"])?;
                        DavMetadata::from_media(&media)
                    } else {
                        credentials.require_any(&["media:list", "media:read", "media:upload"])?;
                        let prefix = directory_prefix(&object_key);
                        let media = self
                            .list_media(credentials.application.id, bucket.id(), Some(prefix))
                            .await?;
                        if media.is_empty() {
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
                        .find_bucket_by_name(credentials.application.id, &bucket_name)
                        .await
                        .map_err(|_| FsError::GeneralFailure)?
                        .is_some()
                    {
                        return Err(FsError::Exists);
                    }
                    let bucket = Bucket::new(
                        BucketId::new(),
                        credentials.application.id,
                        bucket_name,
                        BucketPolicy::new(Visibility::Private, None, None, [])
                            .map_err(map_domain_error)?,
                        OffsetDateTime::now_utc(),
                    )
                    .map_err(map_domain_error)?;
                    self.repository
                        .create_bucket(&bucket)
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
                    if !self
                        .list_media(credentials.application.id, bucket.id(), Some(prefix))
                        .await?
                        .is_empty()
                    {
                        return Err(FsError::Exists);
                    }
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
            let (bucket, media) = self.resolve_file(path, credentials).await?;
            if media.bucket_id() != bucket.id() {
                return Err(FsError::NotFound);
            }
            self.schedule_delete(media, credentials, "webdav").await
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
                        .delete_empty_bucket(credentials.application.id, &bucket_name)
                        .await
                        .map_err(map_repository_error)?;
                    if !deleted {
                        return Err(FsError::Exists);
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
        Box::pin(async move { self.copy_file(from, to, credentials).await.map(|_| ()) })
    }

    fn rename<'a>(
        &'a self,
        from: &'a DavPath,
        to: &'a DavPath,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, ()> {
        Box::pin(async move {
            credentials.require("media:delete")?;
            let source_resource = DavResource::parse(from, credentials)?;
            if matches!(source_resource, DavResource::Object { .. }) {
                match self.resolve_file(from, credentials).await {
                    Ok((_, source)) => {
                        self.copy_file(from, to, credentials).await?;
                        self.schedule_delete(source, credentials, "webdav_move")
                            .await
                    }
                    Err(FsError::NotFound | FsError::Forbidden) => {
                        self.move_directory(from, to, credentials).await
                    }
                    Err(error) => Err(error),
                }
            } else {
                Err(FsError::Forbidden)
            }
        })
    }

    fn get_quota<'a>(
        &'a self,
        credentials: &'a DavCredentials,
    ) -> FsFuture<'a, (u64, Option<u64>)> {
        Box::pin(async move {
            let quota = self
                .repository
                .quota(credentials.application.id)
                .await
                .map_err(|_| FsError::GeneralFailure)?;
            Ok((
                quota.used_bytes.saturating_add(quota.reserved_bytes),
                Some(quota.quota_bytes),
            ))
        })
    }
}

impl MediaHubDavFs {
    async fn find_bucket(
        &self,
        credentials: &DavCredentials,
        bucket_name: &str,
    ) -> FsResult<Bucket> {
        self.repository
            .find_bucket_by_name(credentials.application.id, bucket_name)
            .await
            .map_err(|_| FsError::GeneralFailure)?
            .ok_or(FsError::NotFound)
    }

    async fn find_media(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        object_key: &str,
    ) -> FsResult<Option<Media>> {
        let media = self
            .repository
            .find_by_object_key(application_id, bucket_id, object_key)
            .await
            .map_err(|_| FsError::GeneralFailure)?;
        Ok(media.filter(|media| media.state().is_readable()))
    }

    async fn list_media(
        &self,
        application_id: ApplicationId,
        bucket_id: BucketId,
        prefix: Option<String>,
    ) -> FsResult<Vec<Media>> {
        let mut cursor = None;
        let mut items = Vec::new();
        loop {
            let page = self
                .repository
                .list_media_page(
                    application_id,
                    &MediaListQuery {
                        bucket_id: Some(bucket_id),
                        state: Some(MediaState::Active),
                        object_key_prefix: prefix.clone(),
                        cursor,
                        limit: PAGE_SIZE,
                        ..MediaListQuery::default()
                    },
                )
                .await
                .map_err(|_| FsError::GeneralFailure)?;
            let next = page.items.last().map(|media| MediaListCursor {
                created_at: media.created_at(),
                id: media.id(),
            });
            items.extend(page.items);
            if !page.has_more {
                break;
            }
            cursor = Some(next.ok_or(FsError::GeneralFailure)?);
        }
        Ok(items)
    }

    async fn resolve_file(
        &self,
        path: &DavPath,
        credentials: &DavCredentials,
    ) -> FsResult<(Bucket, Media)> {
        let DavResource::Object {
            bucket_name,
            object_key,
            collection: false,
        } = DavResource::parse(path, credentials)?
        else {
            return Err(FsError::Forbidden);
        };
        let bucket = self.find_bucket(credentials, &bucket_name).await?;
        let media = self
            .find_media(credentials.application.id, bucket.id(), &object_key)
            .await?
            .ok_or(FsError::NotFound)?;
        Ok((bucket, media))
    }

    async fn copy_file(
        &self,
        from: &DavPath,
        to: &DavPath,
        credentials: &DavCredentials,
    ) -> FsResult<Media> {
        credentials.require("media:read")?;
        credentials.require("media:upload")?;
        let (_, source) = self.resolve_file(from, credentials).await?;
        let DavResource::Object {
            bucket_name,
            object_key,
            collection: false,
        } = DavResource::parse(to, credentials)?
        else {
            return Err(FsError::Forbidden);
        };
        let destination_bucket = self.find_bucket(credentials, &bucket_name).await?;
        if self
            .find_media(
                credentials.application.id,
                destination_bucket.id(),
                &object_key,
            )
            .await?
            .is_some()
        {
            return Err(FsError::Exists);
        }
        self.copy_media(&source, destination_bucket.id(), object_key, credentials)
            .await
    }

    async fn copy_media(
        &self,
        source: &Media,
        destination_bucket_id: BucketId,
        object_key: String,
        credentials: &DavCredentials,
    ) -> FsResult<Media> {
        let content = self
            .object_store
            .read(source.storage_key())
            .await
            .map_err(|_| FsError::GeneralFailure)?;
        let receipt = self
            .upload(DavUpload {
                application_id: credentials.application.id,
                bucket_id: destination_bucket_id,
                object_key,
                mime: source.mime().to_owned(),
                content,
                visibility_override: source.visibility_override(),
                expire_at: source.expire_at(),
            })
            .await?;
        self.record_audit(
            credentials,
            "media.copied",
            "media",
            receipt.id().to_string(),
            serde_json::json!({
                "source_media_id": source.id().to_string(),
                "object_key": receipt.object_key(),
                "protocol": "webdav",
            }),
        )
        .await;
        Ok(receipt)
    }

    async fn move_directory(
        &self,
        from: &DavPath,
        to: &DavPath,
        credentials: &DavCredentials,
    ) -> FsResult<()> {
        credentials.require("media:read")?;
        credentials.require("media:upload")?;
        credentials.require("media:delete")?;
        let DavResource::Object {
            bucket_name: source_bucket_name,
            object_key: source_key,
            ..
        } = DavResource::parse(from, credentials)?
        else {
            return Err(FsError::Forbidden);
        };
        let DavResource::Object {
            bucket_name: destination_bucket_name,
            object_key: destination_key,
            ..
        } = DavResource::parse(to, credentials)?
        else {
            return Err(FsError::Forbidden);
        };
        let source_bucket = self.find_bucket(credentials, &source_bucket_name).await?;
        let destination_bucket = self
            .find_bucket(credentials, &destination_bucket_name)
            .await?;
        let source_prefix = directory_prefix(&source_key);
        let destination_prefix = directory_prefix(&destination_key);
        if source_bucket.id() == destination_bucket.id()
            && (source_prefix == destination_prefix
                || destination_prefix.starts_with(&source_prefix))
        {
            return Err(FsError::LoopDetected);
        }
        let source_media = self
            .list_media(
                credentials.application.id,
                source_bucket.id(),
                Some(source_prefix.clone()),
            )
            .await?;
        if source_media.is_empty() {
            return Err(FsError::NotFound);
        }
        let mut destinations = Vec::with_capacity(source_media.len());
        for media in &source_media {
            let suffix = media
                .object_key()
                .strip_prefix(&source_prefix)
                .ok_or(FsError::GeneralFailure)?;
            let object_key = format!("{destination_prefix}{suffix}");
            if self
                .find_media(
                    credentials.application.id,
                    destination_bucket.id(),
                    &object_key,
                )
                .await?
                .is_some()
            {
                return Err(FsError::Exists);
            }
            destinations.push(object_key);
        }
        for (media, object_key) in source_media.into_iter().zip(destinations) {
            self.copy_media(&media, destination_bucket.id(), object_key, credentials)
                .await?;
            self.schedule_delete(media, credentials, "webdav_move")
                .await?;
        }
        Ok(())
    }

    async fn upload(&self, upload: DavUpload) -> FsResult<Media> {
        let display_name = upload
            .object_key
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .ok_or(FsError::Forbidden)?
            .to_owned();
        let extension = display_name
            .rsplit_once('.')
            .and_then(|(_, extension)| (!extension.is_empty()).then(|| extension.to_owned()));
        let service = UploadMediaService::new(
            self.object_store.clone(),
            self.repository.clone(),
            self.repository.clone(),
            DavClock,
        );
        service
            .upload(&UploadMediaRequest {
                application_id: upload.application_id,
                bucket_id: upload.bucket_id,
                object_key: upload.object_key,
                original_name: Some(display_name.clone()),
                display_name,
                extension,
                mime: upload.mime,
                content: upload.content,
                visibility_override: upload.visibility_override,
                expire_at: upload.expire_at,
                metadata: ClientMetadata::default(),
            })
            .await
            .map(|receipt| receipt.media)
            .map_err(map_application_error)
    }

    async fn schedule_delete(
        &self,
        media: Media,
        credentials: &DavCredentials,
        reason: &str,
    ) -> FsResult<()> {
        let now = OffsetDateTime::now_utc();
        let event = OutboxEvent::media_delete_scheduled(&media, now, reason);
        self.repository
            .schedule_delete(media.id(), now, event)
            .await
            .map_err(map_repository_error)?;
        self.record_audit(
            credentials,
            "media.delete_scheduled",
            "media",
            media.id().to_string(),
            serde_json::json!({ "reason": reason, "protocol": "webdav" }),
        )
        .await;
        Ok(())
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

