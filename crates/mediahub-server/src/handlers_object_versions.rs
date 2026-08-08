// Immutable ObjectVersion preview manifest and content handlers.

const OBJECT_VERSION_PREVIEW_RENDERER_VERSION: &str = "1";
const OBJECT_VERSION_BUFFERED_PREVIEW_MAX_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ObjectVersionPreviewRenderer {
    Archive,
    AudioVideo,
    Image,
    Pdf,
    Spreadsheet,
    Sqlite,
    Text,
    General,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ObjectVersionPreviewMode {
    Stream,
    Buffered,
}

#[derive(Debug, Serialize)]
struct ObjectVersionPreviewManifest {
    version_id: String,
    etag: String,
    content_type: String,
    size: u64,
    renderer: ObjectVersionPreviewRenderer,
    renderer_version: &'static str,
    mode: ObjectVersionPreviewMode,
    max_bytes: Option<u64>,
    content_url: String,
    warnings: Vec<&'static str>,
}

async fn get_object_version_preview_manifest(
    State(state): State<Arc<AppState>>,
    Path(version_id): Path<String>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Json<ObjectVersionPreviewManifest>, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.authorize("media:read")?;
    let version_id = parse_preview_object_version_id(&version_id)?;
    let resolved =
        mediahub_app::S3ObjectRepository::find_committed_s3_object_version_for_application(
            &state.repository,
            auth.application.id,
            version_id,
        )
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(object_version_not_found)?;
    let mediahub_core::ObjectVersionPayload::Object(payload) = resolved.version.payload() else {
        return Err(object_version_not_found());
    };
    let content_type = object_version_content_type(payload.content_type(), &resolved.object_key);
    let renderer = detect_object_version_preview_renderer(&resolved.object_key, &content_type);
    let (mode, max_bytes) = object_version_preview_policy(renderer);
    let warnings = if max_bytes.is_some_and(|limit| payload.size_bytes() > limit) {
        vec!["buffered_preview_limit_exceeded"]
    } else {
        Vec::new()
    };
    Ok(Json(ObjectVersionPreviewManifest {
        version_id: version_id.to_string(),
        etag: payload.etag().as_str().to_owned(),
        content_type,
        size: payload.size_bytes(),
        renderer,
        renderer_version: OBJECT_VERSION_PREVIEW_RENDERER_VERSION,
        mode,
        max_bytes,
        content_url: object_version_content_url(version_id),
        warnings,
    }))
}

async fn read_object_version_content(
    State(state): State<Arc<AppState>>,
    Path(version_id): Path<String>,
    method: Method,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
) -> Result<Response, ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.authorize("media:read")?;
    let version_id = parse_preview_object_version_id(&version_id)?;
    let resolved =
        mediahub_app::S3ObjectRepository::find_committed_s3_object_version_for_application(
            &state.repository,
            auth.application.id,
            version_id,
        )
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(object_version_not_found)?;
    let mediahub_core::ObjectVersionPayload::Object(payload) = resolved.version.payload() else {
        return Err(object_version_not_found());
    };
    if payload.storage_backend() != state.object_store.backend_name() {
        return Err(ApiError::unavailable(
            "object version storage backend is not configured",
        ));
    }

    let content_type = object_version_content_type(payload.content_type(), &resolved.object_key);
    if if_none_match_matches(&headers, payload.etag().as_str()) {
        return Ok(object_version_not_modified_response(
            payload.etag().as_str(),
        ));
    }
    let range = match headers
        .get(RANGE)
        .map(|value| {
            value
                .to_str()
                .map_err(|_| ())
                .and_then(|value| parse_object_version_range(value, payload.size_bytes()))
        })
        .transpose()
    {
        Ok(range) => range,
        Err(()) => {
            return Ok(object_version_range_not_satisfiable_response(
                payload.size_bytes(),
            ));
        }
    };
    let stored = state
        .object_store
        .head(payload.storage_key())
        .await
        .map_err(map_object_version_storage_error)?;
    if stored.size != payload.size_bytes() {
        return Err(ApiError::unavailable(
            "object version storage metadata does not match the committed version",
        ));
    }

    let head_only = method == Method::HEAD;
    let body = if head_only {
        Body::empty()
    } else {
        object_version_body(&state, payload.storage_key(), payload.size_bytes(), range).await?
    };
    let download_bytes_per_second = if head_only {
        None
    } else {
        configured_download_rate(&state).await?
    };
    Ok(object_version_content_response(
        body,
        &resolved.object_key,
        &content_type,
        payload.etag().as_str(),
        payload.size_bytes(),
        range,
        head_only,
        download_bytes_per_second,
    ))
}

fn parse_preview_object_version_id(
    value: &str,
) -> Result<mediahub_core::ObjectVersionId, ApiError> {
    value.parse().map_err(|_| object_version_not_found())
}

fn object_version_not_found() -> ApiError {
    ApiError::not_found("object version not found")
}

fn object_version_content_url(version_id: mediahub_core::ObjectVersionId) -> String {
    format!("/api/v1/object-versions/{version_id}/content")
}

fn object_version_content_type(stored: Option<&str>, object_key: &str) -> String {
    stored
        .map(str::trim)
        .filter(|value| !value.is_empty() && HeaderValue::from_str(value).is_ok())
        .map(str::to_owned)
        .or_else(|| {
            mime_guess::from_path(object_key)
                .first_raw()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "application/octet-stream".to_owned())
}

fn object_version_preview_policy(
    renderer: ObjectVersionPreviewRenderer,
) -> (ObjectVersionPreviewMode, Option<u64>) {
    if renderer == ObjectVersionPreviewRenderer::AudioVideo {
        (ObjectVersionPreviewMode::Stream, None)
    } else {
        (
            ObjectVersionPreviewMode::Buffered,
            Some(OBJECT_VERSION_BUFFERED_PREVIEW_MAX_BYTES),
        )
    }
}

fn detect_object_version_preview_renderer(
    object_key: &str,
    content_type: &str,
) -> ObjectVersionPreviewRenderer {
    let extension = object_key
        .rsplit('/')
        .next()
        .unwrap_or(object_key)
        .rsplit_once('.')
        .map_or_else(
            || object_key.rsplit('/').next().unwrap_or(object_key),
            |(_, extension)| extension,
        )
        .to_ascii_lowercase();
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    if matches!(
        extension.as_str(),
        "7z" | "bz2"
            | "bzip2"
            | "gz"
            | "gzip"
            | "lzma"
            | "rar"
            | "tar"
            | "tbz"
            | "tbz2"
            | "tgz"
            | "txz"
            | "xz"
            | "zip"
    ) || matches!(
        mime.as_str(),
        "application/gzip"
            | "application/vnd.rar"
            | "application/x-7z-compressed"
            | "application/x-bzip2"
            | "application/x-compressed-tar"
            | "application/x-gzip"
            | "application/x-gtar"
            | "application/x-lzma"
            | "application/x-rar-compressed"
            | "application/x-tar"
            | "application/x-xz"
            | "application/x-zip-compressed"
            | "application/zip"
    ) {
        return ObjectVersionPreviewRenderer::Archive;
    }
    if matches!(
        extension.as_str(),
        "db" | "db3" | "sdb" | "sqlite" | "sqlite3"
    ) || matches!(
        mime.as_str(),
        "application/sqlite3"
            | "application/vnd.sqlite3"
            | "application/x-sqlite"
            | "application/x-sqlite3"
    ) {
        return ObjectVersionPreviewRenderer::Sqlite;
    }
    if matches!(
        extension.as_str(),
        "csv" | "ods" | "tsv" | "xls" | "xlsb" | "xlsm" | "xlsx"
    ) || matches!(
        mime.as_str(),
        "application/csv"
            | "application/vnd.ms-excel"
            | "application/vnd.ms-excel.sheet.binary.macroenabled.12"
            | "application/vnd.ms-excel.sheet.macroenabled.12"
            | "application/vnd.oasis.opendocument.spreadsheet"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "text/csv"
            | "text/tab-separated-values"
    ) {
        return ObjectVersionPreviewRenderer::Spreadsheet;
    }
    if mime.starts_with("audio/")
        || mime.starts_with("video/")
        || matches!(
            extension.as_str(),
            "aac"
                | "avi"
                | "flac"
                | "flv"
                | "m2ts"
                | "m3u8"
                | "m4a"
                | "m4v"
                | "mid"
                | "midi"
                | "mkv"
                | "mov"
                | "mp3"
                | "mp4"
                | "mpeg"
                | "mpg"
                | "oga"
                | "ogg"
                | "ogv"
                | "opus"
                | "wav"
                | "webm"
                | "wma"
                | "wmv"
        )
    {
        return ObjectVersionPreviewRenderer::AudioVideo;
    }
    if mime == "application/pdf" || extension == "pdf" {
        return ObjectVersionPreviewRenderer::Pdf;
    }
    if mime.starts_with("image/")
        || matches!(
            extension.as_str(),
            "apng"
                | "avif"
                | "bmp"
                | "cur"
                | "gif"
                | "heic"
                | "heif"
                | "ico"
                | "jfif"
                | "jpeg"
                | "jpg"
                | "jxl"
                | "pjpe"
                | "pjpeg"
                | "png"
                | "svg"
                | "tif"
                | "tiff"
                | "webp"
        )
    {
        return if extension == "svg" || mime == "image/svg+xml" {
            ObjectVersionPreviewRenderer::Text
        } else {
            ObjectVersionPreviewRenderer::Image
        };
    }
    if mime.starts_with("text/")
        || mime.contains("json")
        || mime.contains("javascript")
        || mime.contains("typescript")
        || mime == "application/xml"
        || mime.ends_with("+xml")
        || is_preview_text_extension(&extension)
    {
        return ObjectVersionPreviewRenderer::Text;
    }
    ObjectVersionPreviewRenderer::General
}

fn is_preview_text_extension(extension: &str) -> bool {
    matches!(
        extension,
        "astro"
            | "bash"
            | "bat"
            | "bib"
            | "c"
            | "cjs"
            | "clj"
            | "cljs"
            | "cmd"
            | "conf"
            | "config"
            | "cpp"
            | "cs"
            | "css"
            | "cts"
            | "dart"
            | "diff"
            | "dockerfile"
            | "editorconfig"
            | "elm"
            | "env"
            | "erl"
            | "ex"
            | "exs"
            | "fish"
            | "fs"
            | "fsx"
            | "gitignore"
            | "go"
            | "gql"
            | "graphql"
            | "gradle"
            | "h"
            | "hcl"
            | "hpp"
            | "hrl"
            | "hs"
            | "htm"
            | "html"
            | "http"
            | "ini"
            | "ipynb"
            | "java"
            | "js"
            | "json"
            | "json5"
            | "jsonc"
            | "jsonl"
            | "jsx"
            | "kt"
            | "kts"
            | "latex"
            | "less"
            | "lhs"
            | "lock"
            | "log"
            | "lua"
            | "md"
            | "mjs"
            | "mts"
            | "ndjson"
            | "nginxconf"
            | "npmrc"
            | "patch"
            | "php"
            | "proto"
            | "properties"
            | "ps1"
            | "py"
            | "r"
            | "rb"
            | "rs"
            | "scss"
            | "sh"
            | "sql"
            | "svelte"
            | "svg"
            | "swift"
            | "tex"
            | "tf"
            | "tfvars"
            | "toml"
            | "ts"
            | "tsv"
            | "tsx"
            | "txt"
            | "vue"
            | "xml"
            | "yaml"
            | "yml"
            | "zsh"
    )
}

fn parse_object_version_range(value: &str, total: u64) -> Result<(u64, u64), ()> {
    let value = value
        .strip_prefix("bytes=")
        .filter(|value| !value.contains(','))
        .ok_or(())?;
    let (start, end) = value.split_once('-').ok_or(())?;
    if total == 0 {
        return Err(());
    }
    if start.is_empty() {
        let suffix = end.parse::<u64>().map_err(|_| ())?;
        if suffix == 0 {
            return Err(());
        }
        return Ok((total.saturating_sub(suffix), total - 1));
    }
    let start = start.parse::<u64>().map_err(|_| ())?;
    if start >= total {
        return Err(());
    }
    let end = if end.is_empty() {
        total - 1
    } else {
        end.parse::<u64>().map_err(|_| ())?.min(total - 1)
    };
    if start > end {
        Err(())
    } else {
        Ok((start, end))
    }
}

async fn object_version_body(
    state: &AppState,
    storage_key: &str,
    total: u64,
    range: Option<(u64, u64)>,
) -> Result<Body, ApiError> {
    if let Some(local_store) = state.object_store.local_store() {
        let mut file = local_store
            .open_file(storage_key)
            .await
            .map_err(map_object_version_storage_error)?;
        let (start, length) = range.map_or((0, total), |(start, end)| {
            (start, end.saturating_sub(start).saturating_add(1))
        });
        if start != 0 {
            use tokio::io::AsyncSeekExt as _;
            file.seek(std::io::SeekFrom::Start(start))
                .await
                .map_err(|error| {
                    ApiError::unavailable(format!("failed to seek object version: {error}"))
                })?;
        }
        let stream = stream::try_unfold((file, length), |(mut file, remaining)| async move {
            if remaining == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            let mut buffer =
                vec![0_u8; usize::try_from(remaining.min(64 * 1024)).unwrap_or(64 * 1024)];
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                return Ok(None);
            }
            buffer.truncate(read);
            Ok(Some((
                Bytes::from(buffer),
                (file, remaining.saturating_sub(read as u64)),
            )))
        });
        return Ok(Body::from_stream(stream));
    }

    let bytes = match range {
        Some((start, end)) => {
            state
                .object_store
                .read_range(storage_key, start..end.saturating_add(1))
                .await
        }
        None => state.object_store.read(storage_key).await,
    }
    .map_err(map_object_version_storage_error)?;
    let expected = range.map_or(total, |(start, end)| end.saturating_sub(start) + 1);
    if bytes.len() as u64 != expected {
        return Err(ApiError::unavailable(
            "object version storage returned an unexpected content length",
        ));
    }
    Ok(Body::from(bytes))
}

fn map_object_version_storage_error(error: ObjectStoreError) -> ApiError {
    match error {
        ObjectStoreError::NotFound => object_version_not_found(),
        ObjectStoreError::InvalidRange => ApiError::range_not_satisfiable(),
        ObjectStoreError::AlreadyExists
        | ObjectStoreError::InvalidCursor
        | ObjectStoreError::InvalidLimit
        | ObjectStoreError::Unavailable(_) => {
            warn!(error = %error, "object version content read failed");
            ApiError::unavailable("object version storage is unavailable")
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn object_version_content_response(
    body: Body,
    object_key: &str,
    content_type: &str,
    etag: &str,
    total: u64,
    range: Option<(u64, u64)>,
    head_only: bool,
    download_bytes_per_second: Option<u64>,
) -> Response {
    let (status, content_length, content_range) =
        range.map_or((StatusCode::OK, total, None), |(start, end)| {
            (
                StatusCode::PARTIAL_CONTENT,
                end.saturating_sub(start).saturating_add(1),
                Some(format!("bytes {start}-{end}/{total}")),
            )
        });
    let body = if head_only {
        Body::empty()
    } else {
        download_limited_body(body, download_bytes_per_second)
    };
    let mut response = (status, body).into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(ETAG, entity_tag_header_value(etag));
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&content_length.to_string()).expect("content length is valid"),
    );
    response_headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&object_version_content_disposition(
            object_key,
            content_type,
        ))
        .expect("object version filename is sanitized"),
    );
    response_headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response_headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    if media_requires_sandbox(content_type) {
        response_headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox"));
    }
    if let Some(content_range) = content_range {
        response_headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&content_range).expect("content range is valid"),
        );
    }
    response
}

fn object_version_not_modified_response(etag: &str) -> Response {
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    response
        .headers_mut()
        .insert(ETAG, entity_tag_header_value(etag));
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=31536000, immutable"),
    );
    response
        .headers_mut()
        .insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response
}

fn object_version_range_not_satisfiable_response(total: u64) -> Response {
    let mut response = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
    response.headers_mut().insert(
        CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes */{total}")).expect("content range is valid"),
    );
    response
        .headers_mut()
        .insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response
}

fn object_version_content_disposition(object_key: &str, content_type: &str) -> String {
    let filename = object_key.rsplit('/').next().unwrap_or("download");
    let fallback = filename
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    let fallback = fallback.trim_matches('_');
    let fallback = if fallback.is_empty() {
        "download"
    } else {
        fallback
    };
    let encoded =
        percent_encoding::utf8_percent_encode(filename, percent_encoding::NON_ALPHANUMERIC);
    let disposition = if content_type.eq_ignore_ascii_case("application/pdf")
        || !media_requires_sandbox(content_type)
    {
        "inline"
    } else {
        "attachment"
    };
    format!("{disposition}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod object_version_preview_unit_tests {
    use super::*;

    #[test]
    fn preview_classification_matches_the_current_web_viewer_admission_policy() {
        assert_eq!(
            detect_object_version_preview_renderer("reports/book.xlsx", "application/octet-stream"),
            ObjectVersionPreviewRenderer::Spreadsheet
        );
        assert_eq!(
            detect_object_version_preview_renderer("db/catalog.sqlite", "application/octet-stream"),
            ObjectVersionPreviewRenderer::Sqlite
        );
        assert_eq!(
            detect_object_version_preview_renderer("source/icon.svg", "image/svg+xml"),
            ObjectVersionPreviewRenderer::Text
        );
        assert_eq!(
            detect_object_version_preview_renderer("media/clip.mp4", "application/octet-stream"),
            ObjectVersionPreviewRenderer::AudioVideo
        );
        assert_eq!(
            object_version_preview_policy(ObjectVersionPreviewRenderer::AudioVideo),
            (ObjectVersionPreviewMode::Stream, None)
        );
        assert_eq!(
            object_version_preview_policy(ObjectVersionPreviewRenderer::Pdf),
            (
                ObjectVersionPreviewMode::Buffered,
                Some(OBJECT_VERSION_BUFFERED_PREVIEW_MAX_BYTES)
            )
        );
    }

    #[test]
    fn object_version_ranges_support_bounded_open_and_suffix_forms() {
        assert_eq!(parse_object_version_range("bytes=2-5", 10), Ok((2, 5)));
        assert_eq!(parse_object_version_range("bytes=7-", 10), Ok((7, 9)));
        assert_eq!(parse_object_version_range("bytes=-3", 10), Ok((7, 9)));
        assert_eq!(parse_object_version_range("bytes=7-99", 10), Ok((7, 9)));
        assert_eq!(parse_object_version_range("bytes=10-", 10), Err(()));
        assert_eq!(parse_object_version_range("bytes=0-1,4-5", 10), Err(()));
        assert_eq!(parse_object_version_range("items=0-1", 10), Err(()));
    }
}
