//! S3-compatible implementation of MediaHub's immutable object-store port.

use std::{
    collections::BTreeMap,
    fmt,
    ops::Range,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use aws_credential_types::Credentials;
use aws_sigv4::{
    http_request::{
        PercentEncodingMode, SignableBody, SignableRequest, SignatureLocation, SigningSettings,
        UriPathNormalizationMode, sign,
    },
    sign::v4,
};
use futures_util::TryStreamExt;
use http::{Method, Request};
use mediahub_app::{
    ComposedObject, ObjectMetadata, ObjectPage, ObjectStore, ObjectStoreError, PreparedUpload,
    StoredUpload, UploadSessionStorage, UploadTarget,
};
use mediahub_core::{MediaId, OffsetDateTime, UploadSession, UploadSessionId};
use object_store::{
    Attribute, Attributes, Error as BackendError, GetOptions, ObjectStore as BackendObjectStore,
    ObjectStoreExt, PutMode, PutMultipartOptions, PutOptions, WriteMultipart,
    aws::{AmazonS3, AmazonS3Builder},
    path::Path,
    signer::Signer,
};
use sha2::{Digest, Sha256};

const S3_BACKEND: &str = "s3";
const MAX_PRESIGNED_PUT_TTL: Duration = Duration::from_secs(15 * 60);
const COMMIT_COMPARE_CHUNK_SIZE: u64 = 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3Config {
    pub bucket: String,
    pub region: String,
    pub endpoint: Option<String>,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    pub allow_http: bool,
    pub virtual_hosted_style: bool,
    pub prefix: Option<String>,
}

impl S3Config {
    /// Builds an S3-compatible store. Missing credentials may still be
    /// resolved by the backend's standard AWS environment/metadata chain.
    ///
    /// # Errors
    ///
    /// Returns an error when the endpoint, credentials, or prefix are invalid.
    pub fn build(&self) -> Result<S3ObjectStore, ObjectStoreError> {
        if self.bucket.trim().is_empty() || self.region.trim().is_empty() {
            return Err(ObjectStoreError::Unavailable(
                "S3 bucket and region are required".into(),
            ));
        }
        let mut builder = AmazonS3Builder::from_env()
            .with_bucket_name(self.bucket.trim())
            .with_region(self.region.trim())
            .with_allow_http(self.allow_http)
            .with_virtual_hosted_style_request(self.virtual_hosted_style);
        if let Some(endpoint) = self.endpoint.as_deref() {
            builder = builder.with_endpoint(endpoint);
        }
        if let Some(access_key_id) = self.access_key_id.as_deref() {
            builder = builder.with_access_key_id(access_key_id);
        }
        if let Some(secret_access_key) = self.secret_access_key.as_deref() {
            builder = builder.with_secret_access_key(secret_access_key);
        }
        if let Some(session_token) = self.session_token.as_deref() {
            builder = builder.with_token(session_token);
        }
        let backend = builder.build().map_err(map_backend_error)?;
        let signer = AwsPresignedPutSigner::new(backend.clone(), self.region.trim().to_owned());
        S3ObjectStore::from_parts(
            Arc::new(backend),
            self.prefix.as_deref(),
            Some(Arc::new(signer)),
        )
    }
}

#[async_trait]
trait PresignedPutSigner: fmt::Debug + Send + Sync {
    async fn sign(
        &self,
        path: &Path,
        content_length: u64,
        content_type: &str,
        expires_at: OffsetDateTime,
    ) -> Result<String, ObjectStoreError>;
}

#[derive(Clone, Debug)]
struct AwsPresignedPutSigner {
    backend: AmazonS3,
    region: String,
}

impl AwsPresignedPutSigner {
    fn new(backend: AmazonS3, region: String) -> Self {
        Self { backend, region }
    }

    async fn sign_at(
        &self,
        path: &Path,
        content_length: u64,
        content_type: &str,
        expires_at: OffsetDateTime,
        now: SystemTime,
    ) -> Result<String, ObjectStoreError> {
        let expires_at: SystemTime = expires_at.into();
        let expires_in = expires_at.duration_since(now).map_err(|_| {
            ObjectStoreError::Unavailable("presigned PUT expiry must be in the future".into())
        })?;
        let expires_in = Duration::from_secs(expires_in.as_secs());
        if expires_in.is_zero() || expires_in > MAX_PRESIGNED_PUT_TTL {
            return Err(ObjectStoreError::Unavailable(
                "presigned PUT expiry must be between 1 and 900 seconds".into(),
            ));
        }

        // object_store owns endpoint resolution (path style, virtual-hosted
        // style, and compatible endpoints). Its signer cannot bind request
        // headers, so retain only the resolved base URL and sign it below.
        let mut base_url = self
            .backend
            .signed_url(Method::PUT, path, expires_in)
            .await
            .map_err(map_backend_error)?;
        base_url.set_query(None);

        let source = self
            .backend
            .credentials()
            .get_credential()
            .await
            .map_err(map_backend_error)?;
        let identity = Credentials::new(
            source.key_id.clone(),
            source.secret_key.clone(),
            source.token.clone(),
            None,
            "mediahub-object-store-credential-chain",
        )
        .into();
        let mut settings = SigningSettings::default();
        settings.signature_location = SignatureLocation::QueryParams;
        settings.expires_in = Some(expires_in);
        settings.percent_encoding_mode = PercentEncodingMode::Single;
        settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name("s3")
            .time(now)
            .settings(settings)
            .build()
            .map_err(|error| ObjectStoreError::Unavailable(error.to_string()))?
            .into();
        let content_length = content_length.to_string();
        let headers = [
            ("content-length", content_length.as_str()),
            ("content-type", content_type),
        ];
        let signable = SignableRequest::new(
            "PUT",
            base_url.as_str(),
            headers.into_iter(),
            SignableBody::UnsignedPayload,
        )
        .map_err(|error| ObjectStoreError::Unavailable(error.to_string()))?;
        let (instructions, _) = sign(signable, &params)
            .map_err(|error| ObjectStoreError::Unavailable(error.to_string()))?
            .into_parts();
        let mut request = Request::builder()
            .method(Method::PUT)
            .uri(base_url.as_str())
            .body(())
            .map_err(|error| ObjectStoreError::Unavailable(error.to_string()))?;
        instructions.apply_to_request_http1x(&mut request);
        Ok(request.uri().to_string())
    }
}

#[async_trait]
impl PresignedPutSigner for AwsPresignedPutSigner {
    async fn sign(
        &self,
        path: &Path,
        content_length: u64,
        content_type: &str,
        expires_at: OffsetDateTime,
    ) -> Result<String, ObjectStoreError> {
        self.sign_at(
            path,
            content_length,
            content_type,
            expires_at,
            SystemTime::now(),
        )
        .await
    }
}

#[derive(Clone, Debug)]
pub struct S3ObjectStore {
    backend: Arc<dyn BackendObjectStore>,
    prefix: Option<Path>,
    upload_signer: Option<Arc<dyn PresignedPutSigner>>,
}

impl S3ObjectStore {
    /// Wraps an S3-compatible backend with an optional object-key prefix.
    ///
    /// # Errors
    ///
    /// Returns an error when the configured prefix is not a valid object path.
    pub fn from_backend(
        backend: Arc<dyn BackendObjectStore>,
        prefix: Option<&str>,
    ) -> Result<Self, ObjectStoreError> {
        Self::from_parts(backend, prefix, None)
    }

    fn from_parts(
        backend: Arc<dyn BackendObjectStore>,
        prefix: Option<&str>,
        upload_signer: Option<Arc<dyn PresignedPutSigner>>,
    ) -> Result<Self, ObjectStoreError> {
        let prefix = prefix
            .map(|value| parse_path(value, "S3 prefix"))
            .transpose()?;
        Ok(Self {
            backend,
            prefix,
            upload_signer,
        })
    }

    fn path_for(&self, key: &str) -> Result<Path, ObjectStoreError> {
        let key = parse_path(key, "storage key")?;
        match &self.prefix {
            Some(prefix) => Path::parse(format!("{}/{key}", prefix.as_ref()))
                .map_err(|error| ObjectStoreError::Unavailable(error.to_string())),
            None => Ok(key),
        }
    }

    fn logical_key(&self, path: &Path) -> Result<String, ObjectStoreError> {
        let value = path.as_ref();
        match &self.prefix {
            Some(prefix) => value
                .strip_prefix(prefix.as_ref())
                .and_then(|value| value.strip_prefix('/'))
                .map(ToOwned::to_owned)
                .ok_or_else(|| {
                    ObjectStoreError::Unavailable(
                        "S3 listing returned an object outside the configured prefix".into(),
                    )
                }),
            None => Ok(value.to_owned()),
        }
    }

    async fn put_create(
        &self,
        path: &Path,
        content: Vec<u8>,
        attributes: Attributes,
    ) -> Result<(), ObjectStoreError> {
        self.backend
            .put_opts(
                path,
                content.into(),
                PutOptions {
                    mode: PutMode::Create,
                    attributes,
                    ..PutOptions::default()
                },
            )
            .await
            .map(|_| ())
            .map_err(map_backend_error)
    }

    async fn object_contents_match(
        &self,
        left_path: &Path,
        left: &object_store::ObjectMeta,
        right_path: &Path,
        right: &object_store::ObjectMeta,
    ) -> Result<bool, ObjectStoreError> {
        if left.size != right.size {
            return Ok(false);
        }

        let mut start = 0;
        while start < left.size {
            let end = start
                .saturating_add(COMMIT_COMPARE_CHUNK_SIZE)
                .min(left.size);
            let range = start..end;
            let left_chunk = self
                .backend
                .get_opts(
                    left_path,
                    GetOptions::new()
                        .with_range(Some(range.clone()))
                        .with_if_match(left.e_tag.clone())
                        .with_version(left.version.clone()),
                )
                .await
                .map_err(map_commit_comparison_error)?
                .bytes()
                .await
                .map_err(map_backend_error)?;
            let right_chunk = self
                .backend
                .get_opts(
                    right_path,
                    GetOptions::new()
                        .with_range(Some(range))
                        .with_if_match(right.e_tag.clone())
                        .with_version(right.version.clone()),
                )
                .await
                .map_err(map_commit_comparison_error)?
                .bytes()
                .await
                .map_err(map_backend_error)?;
            if left_chunk != right_chunk {
                return Ok(false);
            }
            start = end;
        }
        Ok(true)
    }
}

#[async_trait]
impl UploadSessionStorage for S3ObjectStore {
    async fn prepare_upload(
        &self,
        _upload_session_id: UploadSessionId,
        media_id: MediaId,
        expected_size: u64,
        expected_mime: &str,
        expires_at: OffsetDateTime,
    ) -> Result<PreparedUpload, ObjectStoreError> {
        let storage_key = format!("objects/{media_id}");
        let path = self.path_for(&storage_key)?;
        let signer = self.upload_signer.as_ref().ok_or_else(|| {
            ObjectStoreError::Unavailable(
                "this S3 backend has no presigned PUT signer; construct it from S3Config".into(),
            )
        })?;
        let url = signer
            .sign(&path, expected_size, expected_mime, expires_at)
            .await?;
        Ok(PreparedUpload {
            target: UploadTarget {
                method: "PUT".to_owned(),
                url,
                headers: BTreeMap::from([
                    ("content-length".to_owned(), expected_size.to_string()),
                    ("content-type".to_owned(), expected_mime.to_owned()),
                ]),
                expires_at,
            },
            storage_backend: S3_BACKEND.to_owned(),
            storage_key,
        })
    }

    async fn inspect_upload(
        &self,
        session: &UploadSession,
    ) -> Result<StoredUpload, ObjectStoreError> {
        let path = self.path_for(session.storage_key())?;
        let head = self.backend.head(&path).await.map_err(map_backend_error)?;
        if head.e_tag.is_none() && head.version.is_none() {
            return Err(ObjectStoreError::Unavailable(
                "S3 upload inspection requires an ETag or object version to fence the GET".into(),
            ));
        }
        let result = self
            .backend
            .get_opts(
                &path,
                GetOptions::new()
                    .with_if_match(head.e_tag.clone())
                    .with_version(head.version.clone()),
            )
            .await
            .map_err(|error| match error {
                BackendError::Precondition { .. } => ObjectStoreError::Unavailable(
                    "S3 object changed between upload inspection HEAD and GET".into(),
                ),
                error => map_backend_error(error),
            })?;
        if result.meta.size != head.size
            || result.meta.e_tag != head.e_tag
            || result.meta.version != head.version
        {
            return Err(ObjectStoreError::Unavailable(
                "S3 object changed during upload inspection".into(),
            ));
        }
        let mime = result
            .attributes
            .get(&Attribute::ContentType)
            .ok_or_else(|| {
                ObjectStoreError::Unavailable("S3 object has no Content-Type metadata".into())
            })?
            .to_string();
        let mut stream = result.into_stream();
        let mut size = 0_u64;
        let mut hasher = Sha256::new();
        while let Some(chunk) = stream.try_next().await.map_err(map_backend_error)? {
            size = size
                .checked_add(chunk.len() as u64)
                .ok_or_else(|| ObjectStoreError::Unavailable("S3 object size overflow".into()))?;
            hasher.update(&chunk);
        }
        if size != head.size {
            return Err(ObjectStoreError::Unavailable(
                "S3 object length changed during upload inspection".into(),
            ));
        }
        Ok(StoredUpload {
            size,
            mime,
            sha256: hex::encode(hasher.finalize()),
        })
    }

    async fn abort_upload(&self, session: &UploadSession) -> Result<(), ObjectStoreError> {
        // This adapter currently issues one complete presigned PUT and never
        // creates multipart state. If multipart is introduced, its upload ID
        // must be persisted and explicitly aborted before this delete.
        self.delete(session.storage_key()).await
    }
}

#[async_trait]
impl ObjectStore for S3ObjectStore {
    fn backend_name(&self) -> &str {
        S3_BACKEND
    }

    async fn put_temporary(
        &self,
        temporary_key: &str,
        content: &[u8],
        content_type: &str,
    ) -> Result<(), ObjectStoreError> {
        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentType, content_type.to_owned().into());
        self.put_create(&self.path_for(temporary_key)?, content.to_vec(), attributes)
            .await
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
        let destination = self.path_for(temporary_key)?;
        let mut attributes = Attributes::new();
        attributes.insert(Attribute::ContentType, content_type.to_owned().into());
        let upload = self
            .backend
            .put_multipart_opts(
                &destination,
                PutMultipartOptions {
                    attributes: attributes.clone(),
                    ..PutMultipartOptions::default()
                },
            )
            .await
            .map_err(map_backend_error)?;
        let mut writer = WriteMultipart::new(upload);
        let mut digest = Sha256::new();
        let mut size = 0_u64;
        for source_key in source_keys {
            let result = match self.backend.get(&self.path_for(source_key)?).await {
                Ok(result) => result,
                Err(error) => {
                    let _ = writer.abort().await;
                    return Err(map_backend_error(error));
                }
            };
            let mut stream = result.into_stream();
            loop {
                let chunk = match stream.try_next().await {
                    Ok(Some(chunk)) => chunk,
                    Ok(None) => break,
                    Err(error) => {
                        let _ = writer.abort().await;
                        return Err(map_backend_error(error));
                    }
                };
                size = match size.checked_add(u64::try_from(chunk.len()).map_err(|error| {
                    ObjectStoreError::Unavailable(format!(
                        "part size is not representable: {error}"
                    ))
                })?) {
                    Some(size) => size,
                    None => {
                        let _ = writer.abort().await;
                        return Err(ObjectStoreError::Unavailable(
                            "composed object exceeds u64".into(),
                        ));
                    }
                };
                digest.update(&chunk);
                writer.put(chunk);
                if let Err(error) = writer.wait_for_capacity(4).await {
                    let _ = writer.abort().await;
                    return Err(map_backend_error(error));
                }
            }
        }
        if size == 0 {
            writer.abort().await.map_err(map_backend_error)?;
            self.put_create(&destination, Vec::new(), attributes)
                .await?;
        } else {
            writer.finish().await.map_err(map_backend_error)?;
        }
        Ok(ComposedObject {
            size,
            sha256: hex::encode(digest.finalize()),
        })
    }

    async fn commit_temporary(
        &self,
        temporary_key: &str,
        final_key: &str,
    ) -> Result<(), ObjectStoreError> {
        let temporary_path = self.path_for(temporary_key)?;
        let final_path = self.path_for(final_key)?;
        let temporary = self
            .backend
            .head(&temporary_path)
            .await
            .map_err(map_backend_error)?;

        match self.backend.head(&final_path).await {
            Ok(final_object) => {
                if !self
                    .object_contents_match(&temporary_path, &temporary, &final_path, &final_object)
                    .await?
                {
                    return Err(ObjectStoreError::AlreadyExists);
                }
            }
            Err(BackendError::NotFound { .. }) => self
                .backend
                .copy(&temporary_path, &final_path)
                .await
                .map_err(map_backend_error)?,
            Err(error) => return Err(map_backend_error(error)),
        }
        self.backend
            .delete(&temporary_path)
            .await
            .map_err(map_backend_error)
    }

    async fn read(&self, key: &str) -> Result<Vec<u8>, ObjectStoreError> {
        self.backend
            .get(&self.path_for(key)?)
            .await
            .map_err(map_backend_error)?
            .bytes()
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(map_backend_error)
    }

    async fn read_range(&self, key: &str, range: Range<u64>) -> Result<Vec<u8>, ObjectStoreError> {
        let path = self.path_for(key)?;
        let metadata = self.backend.head(&path).await.map_err(map_backend_error)?;
        if range.start >= range.end || range.end > metadata.size {
            return Err(ObjectStoreError::InvalidRange);
        }
        self.backend
            .get_range(&path, range)
            .await
            .map(|bytes| bytes.to_vec())
            .map_err(map_backend_error)
    }

    async fn head(&self, key: &str) -> Result<ObjectMetadata, ObjectStoreError> {
        let result = self
            .backend
            .get_opts(&self.path_for(key)?, GetOptions::new().with_head(true))
            .await
            .map_err(map_backend_error)?;
        Ok(ObjectMetadata {
            key: key.to_owned(),
            size: result.meta.size,
            content_type: result
                .attributes
                .get(&Attribute::ContentType)
                .map(|value| value.as_ref().to_owned()),
            etag: result.meta.e_tag,
            version: result.meta.version,
            checksum_sha256: None,
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
        let prefix_path = self.path_for(prefix)?;
        let cursor_path = cursor.map(|value| self.path_for(value)).transpose()?;
        if cursor.is_some_and(|value| !value.starts_with(prefix)) {
            return Err(ObjectStoreError::InvalidCursor);
        }
        let mut stream = match cursor_path.as_ref() {
            Some(cursor) => self.backend.list_with_offset(Some(&prefix_path), cursor),
            None => self.backend.list(Some(&prefix_path)),
        };
        let mut objects = Vec::with_capacity(limit.saturating_add(1));
        while objects.len() <= limit {
            let Some(metadata) = stream.try_next().await.map_err(map_backend_error)? else {
                break;
            };
            objects.push(ObjectMetadata {
                key: self.logical_key(&metadata.location)?,
                size: metadata.size,
                content_type: None,
                etag: metadata.e_tag,
                version: metadata.version,
                checksum_sha256: None,
                provider_metadata: BTreeMap::new(),
            });
        }
        objects.sort_by(|left, right| left.key.cmp(&right.key));
        let has_more = objects.len() > limit;
        objects.truncate(limit);
        Ok(ObjectPage {
            next_cursor: has_more.then(|| {
                objects
                    .last()
                    .expect("non-empty paginated object page")
                    .key
                    .clone()
            }),
            objects,
        })
    }

    async fn delete(&self, key: &str) -> Result<(), ObjectStoreError> {
        match self.backend.delete(&self.path_for(key)?).await {
            Ok(()) | Err(BackendError::NotFound { .. }) => Ok(()),
            Err(error) => Err(map_backend_error(error)),
        }
    }

    async fn exists(&self, key: &str) -> Result<bool, ObjectStoreError> {
        match self.backend.head(&self.path_for(key)?).await {
            Ok(_) => Ok(true),
            Err(BackendError::NotFound { .. }) => Ok(false),
            Err(error) => Err(map_backend_error(error)),
        }
    }
}

fn parse_path(value: &str, label: &str) -> Result<Path, ObjectStoreError> {
    if value.is_empty() || value.starts_with('/') || value.ends_with('/') {
        return Err(ObjectStoreError::Unavailable(format!(
            "{label} must be a non-empty relative object path"
        )));
    }
    Path::parse(value).map_err(|error| ObjectStoreError::Unavailable(error.to_string()))
}

fn map_backend_error(error: BackendError) -> ObjectStoreError {
    match error {
        BackendError::NotFound { .. } => ObjectStoreError::NotFound,
        BackendError::AlreadyExists { .. } | BackendError::Precondition { .. } => {
            ObjectStoreError::AlreadyExists
        }
        error => ObjectStoreError::Unavailable(error.to_string()),
    }
}

fn map_commit_comparison_error(error: BackendError) -> ObjectStoreError {
    match error {
        BackendError::Precondition { .. } => ObjectStoreError::Unavailable(
            "object changed while checking an idempotent S3 commit".into(),
        ),
        error => map_backend_error(error),
    }
}
#[cfg(test)]
mod tests {
    include!("tests.rs");
}
