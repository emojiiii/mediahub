// Media responses, upload parsing, validation, and rate limiting.

fn clear_auth_cookies_response(state: &AppState, status: StatusCode) -> Result<Response, ApiError> {
    let mut response = status.into_response();
    let cookie_attributes = cookie_attributes(&state.cookie_config);
    response.headers_mut().insert(
        SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{SESSION_COOKIE}=; Path=/; HttpOnly{cookie_attributes}; Max-Age=0"
        ))
        .map_err(|_| ApiError::unavailable("failed to clear session cookie"))?,
    );
    response.headers_mut().append(
        SET_COOKIE,
        HeaderValue::from_str(&format!(
            "{CSRF_COOKIE}=; Path=/{cookie_attributes}; Max-Age=0"
        ))
        .map_err(|_| ApiError::unavailable("failed to clear CSRF cookie"))?,
    );
    Ok(response)
}

fn cookie_attributes(config: &CookieConfig) -> String {
    let secure = if config.secure { "; Secure" } else { "" };
    format!("{secure}; SameSite={}", config.same_site)
}

fn local_file_body(file: tokio::fs::File) -> Body {
    let stream = stream::try_unfold(file, |mut file| async move {
        let mut buffer = vec![0_u8; 64 * 1024];
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            Ok::<Option<(Bytes, tokio::fs::File)>, std::io::Error>(None)
        } else {
            buffer.truncate(read);
            Ok(Some((Bytes::from(buffer), file)))
        }
    });
    Body::from_stream(stream)
}

async fn configured_download_rate(state: &AppState) -> Result<Option<u64>, ApiError> {
    state
        .repository
        .admin_system_settings()
        .await
        .map(|settings| settings.download_bytes_per_second)
        .map_err(ApiError::from_repository)
}

fn download_limited_body(body: Body, bytes_per_second: Option<u64>) -> Body {
    let bytes_per_second = bytes_per_second.filter(|value| *value > 0);
    let source = Box::pin(body.into_data_stream());
    let stream = stream::try_unfold(
        (source, Bytes::new(), 0_u64, Instant::now()),
        move |(mut source, mut pending, bytes_sent, started_at)| async move {
            while pending.is_empty() {
                match source.next().await {
                    Some(Ok(chunk)) => pending = chunk,
                    Some(Err(error)) => return Err(error),
                    None => return Ok(None),
                }
            }
            let chunk_length = pending.len().min(DOWNLOAD_BODY_CHUNK_BYTES);
            let chunk = pending.split_to(chunk_length);
            let bytes_sent = bytes_sent.saturating_add(chunk_length as u64);
            if let Some(bytes_per_second) = bytes_per_second {
                let target_elapsed = download_target_elapsed(bytes_sent, bytes_per_second);
                if let Some(delay) = target_elapsed.checked_sub(started_at.elapsed()) {
                    tokio::time::sleep(delay).await;
                }
            }
            Ok(Some((chunk, (source, pending, bytes_sent, started_at))))
        },
    );
    Body::from_stream(stream)
}

fn download_target_elapsed(bytes_sent: u64, bytes_per_second: u64) -> StdDuration {
    let whole_seconds = bytes_sent / bytes_per_second;
    let remainder = bytes_sent % bytes_per_second;
    let nanos =
        u64::try_from(u128::from(remainder) * 1_000_000_000_u128 / u128::from(bytes_per_second))
            .expect("subsecond download duration fits u64");
    StdDuration::from_secs(whole_seconds) + StdDuration::from_nanos(nanos)
}

fn media_response_body(
    media: &Media,
    body: Body,
    total: usize,
    range: Option<(usize, usize)>,
    visibility: Visibility,
    head_only: bool,
    download_bytes_per_second: Option<u64>,
) -> Response {
    let (status, expected_length, content_range) = match range {
        Some((start, end)) => (
            StatusCode::PARTIAL_CONTENT,
            end - start + 1,
            Some(format!("bytes {start}-{end}/{total}")),
        ),
        None => (StatusCode::OK, total, None),
    };
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
        HeaderValue::from_str(media.mime())
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response_headers.insert(ETAG, entity_tag_header_value(media.etag()));
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&expected_length.to_string())
            .expect("content length is a valid header"),
    );
    response_headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&content_disposition(media))
            .expect("content disposition is constructed from header-safe characters"),
    );
    response_headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response_headers.insert(
        axum::http::header::CACHE_CONTROL,
        cache_control(media, visibility),
    );
    if media_requires_sandbox(media.mime()) {
        response_headers.insert(CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox"));
    }
    if let Some(content_range) = content_range {
        response_headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&content_range).expect("content range is a valid header"),
        );
    }
    response
}

fn variant_response_bytes(
    media: &Media,
    receipt: mediahub_app::VariantReceipt,
    visibility: Visibility,
    head_only: bool,
    download_bytes_per_second: Option<u64>,
) -> Response {
    let body_length = receipt.content.len();
    let body = if head_only {
        Body::empty()
    } else {
        download_limited_body(Body::from(receipt.content), download_bytes_per_second)
    };
    let mut response = (StatusCode::OK, body).into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("none"));
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static(receipt.variant.format.mime()),
    );
    response_headers.insert(
        ETAG,
        entity_tag_header_value(&receipt.variant.transform_key),
    );
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&body_length.to_string()).expect("content length is a valid header"),
    );
    response_headers.insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "inline; filename=\"variant.{}\"",
            receipt.variant.format.extension()
        ))
        .expect("variant filename is header safe"),
    );
    response_headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response_headers.insert(
        axum::http::header::CACHE_CONTROL,
        cache_control(media, visibility),
    );
    response
}

fn variant_not_modified_response(
    media: &Media,
    transform_key: &str,
    visibility: Visibility,
) -> Response {
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    response
        .headers_mut()
        .insert(ETAG, entity_tag_header_value(transform_key));
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        cache_control(media, visibility),
    );
    response
}

fn media_not_modified_response(media: &Media, visibility: Visibility) -> Response {
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    let response_headers = response.headers_mut();
    response_headers.insert(ETAG, entity_tag_header_value(media.etag()));
    response_headers.insert(
        axum::http::header::CACHE_CONTROL,
        cache_control(media, visibility),
    );
    response_headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response_headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

fn entity_tag_header_value(etag: &str) -> HeaderValue {
    HeaderValue::from_str(&format!("\"{etag}\""))
        .unwrap_or_else(|_| HeaderValue::from_static("\"invalid\""))
}

fn if_none_match_matches(headers: &HeaderMap, etag: &str) -> bool {
    let Some(value) = headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    value.trim() == "*"
        || value
            .split(',')
            .map(str::trim)
            .any(|candidate| candidate.trim_start_matches("W/") == entity_tag(etag))
}

fn entity_tag(etag: &str) -> String {
    format!("\"{etag}\"")
}

fn cache_control(media: &Media, visibility: Visibility) -> HeaderValue {
    if visibility == Visibility::Private {
        return HeaderValue::from_static("private, no-store");
    }
    let max_age = media
        .expire_at()
        .map(|expires_at| {
            (expires_at - OffsetDateTime::now_utc())
                .whole_seconds()
                .max(0)
        })
        .unwrap_or(3600)
        .min(3600);
    HeaderValue::from_str(&format!("public, max-age={max_age}"))
        .expect("cache control is constructed from an integer")
}

fn content_disposition(media: &Media) -> String {
    let filename = media
        .original_name()
        .unwrap_or_else(|| media.display_name());
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
    let encoded = filename
        .as_bytes()
        .iter()
        .flat_map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_' | b'~' => {
                format!("{}", char::from(*byte)).chars().collect::<Vec<_>>()
            }
            byte => format!("%{byte:02X}").chars().collect::<Vec<_>>(),
        })
        .collect::<String>();
    let disposition = content_disposition_type(media.mime());
    format!("{disposition}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

fn content_disposition_type(mime: &str) -> &'static str {
    if mime.eq_ignore_ascii_case("application/pdf") || !media_requires_sandbox(mime) {
        "inline"
    } else {
        "attachment"
    }
}

fn media_requires_sandbox(mime: &str) -> bool {
    let mime = mime.to_ascii_lowercase();
    let known_executable = mime == "image/svg+xml"
        || mime.contains("html")
        || mime.contains("xml")
        || mime.contains("javascript")
        || mime.contains("ecmascript");
    !matches!(mime.as_str(), value if value.starts_with("image/") || value.starts_with("audio/") || value.starts_with("video/"))
        || known_executable
}

fn parse_single_range(value: &str, total: usize) -> Result<(usize, usize), ApiError> {
    let value = value
        .strip_prefix("bytes=")
        .filter(|value| !value.contains(','))
        .ok_or_else(ApiError::range_not_satisfiable)?;
    let (start, end) = value
        .split_once('-')
        .ok_or_else(ApiError::range_not_satisfiable)?;
    let start = start
        .parse::<usize>()
        .map_err(|_| ApiError::range_not_satisfiable())?;
    let end = if end.is_empty() {
        total
            .checked_sub(1)
            .ok_or_else(ApiError::range_not_satisfiable)?
    } else {
        end.parse::<usize>()
            .map_err(|_| ApiError::range_not_satisfiable())?
    };
    if start > end || end >= total {
        Err(ApiError::range_not_satisfiable())
    } else {
        Ok((start, end))
    }
}

fn parse_if_match(headers: &HeaderMap) -> Result<u64, ApiError> {
    let value = headers
        .get("if-match")
        .ok_or_else(|| ApiError::bad_request("If-Match is required"))?
        .to_str()
        .map_err(|_| ApiError::bad_request("If-Match is invalid"))?;
    value
        .trim_matches('"')
        .parse()
        .map_err(|_| ApiError::bad_request("If-Match must be a media revision"))
}

async fn parse_upload(mut multipart: Multipart) -> Result<ParsedUpload, ApiError> {
    let mut bucket = None;
    let mut object_key = None;
    let mut display_name = None;
    let mut visibility_override = None;
    let mut raw_metadata = None;
    let mut ttl_seconds = None;
    let mut file = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|_| ApiError::bad_request("invalid multipart payload"))?
    {
        let name = field.name().unwrap_or_default().to_owned();
        match name.as_str() {
            "bucket" => {
                if bucket.is_some() {
                    return Err(ApiError::bad_request("bucket may only be supplied once"));
                }
                let name = field
                    .text()
                    .await
                    .map_err(|_| ApiError::bad_request("invalid bucket"))?;
                if name.trim().is_empty() {
                    return Err(ApiError::bad_request("bucket is invalid"));
                }
                bucket = Some(name);
            }
            "object_key" => {
                if object_key.is_some() {
                    return Err(ApiError::bad_request(
                        "object_key may only be supplied once",
                    ));
                }
                object_key = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| ApiError::bad_request("invalid object_key"))?,
                )
            }
            "display_name" => {
                if display_name.is_some() {
                    return Err(ApiError::bad_request(
                        "display_name may only be supplied once",
                    ));
                }
                display_name = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| ApiError::bad_request("invalid display_name"))?,
                )
            }
            "visibility" => {
                if visibility_override.is_some() {
                    return Err(ApiError::bad_request(
                        "visibility may only be supplied once",
                    ));
                }
                visibility_override = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| ApiError::bad_request("invalid visibility"))?,
                )
            }
            "metadata" => {
                if raw_metadata.is_some() {
                    return Err(ApiError::bad_request("metadata may only be supplied once"));
                }
                raw_metadata = Some(
                    field
                        .text()
                        .await
                        .map_err(|_| ApiError::bad_request("invalid metadata"))?,
                )
            }
            "ttl_seconds" => {
                if ttl_seconds.is_some() {
                    return Err(ApiError::bad_request(
                        "ttl_seconds may only be supplied once",
                    ));
                }
                let value = field
                    .text()
                    .await
                    .map_err(|_| ApiError::bad_request("invalid ttl_seconds"))?
                    .parse::<u64>()
                    .map_err(|_| ApiError::bad_request("ttl_seconds must be a positive integer"))?;
                if value == 0 {
                    return Err(ApiError::bad_request(
                        "ttl_seconds must be a positive integer",
                    ));
                }
                ttl_seconds = Some(value);
            }
            "file" if file.is_none() => {
                let original_name = field.file_name().map(str::to_owned);
                let content = field
                    .bytes()
                    .await
                    .map_err(|_| ApiError::bad_request("invalid file"))?;
                file = Some((original_name, content));
            }
            _ => return Err(ApiError::bad_request("unknown or repeated multipart field")),
        }
    }
    let bucket = bucket.ok_or_else(|| ApiError::bad_request("bucket is required"))?;
    let (original_name, content) = file.ok_or_else(|| ApiError::bad_request("file is required"))?;
    let object_key = object_key.unwrap_or_else(|| generated_object_key(original_name.as_deref()));
    let metadata = match raw_metadata {
        Some(value) => serde_json::from_str(&value)
            .map_err(|_| ApiError::bad_request("metadata must be JSON"))
            .and_then(|value| {
                ClientMetadata::from_value(value)
                    .map_err(|error| ApiError::bad_request(error.to_string()))
            })?,
        None => ClientMetadata::default(),
    };
    let visibility_override = visibility_override
        .map(|value| match value.as_str() {
            "public" => Ok(Visibility::Public),
            "private" => Ok(Visibility::Private),
            _ => Err(ApiError::bad_request("visibility is invalid")),
        })
        .transpose()?;
    let display_name = display_name
        .or_else(|| original_name.clone())
        .unwrap_or_else(|| object_key.clone());
    let extension = original_name.as_deref().and_then(|name| {
        name.rsplit_once('.')
            .map(|(_, extension)| extension.to_owned())
    });
    Ok(ParsedUpload {
        bucket,
        object_key,
        original_name,
        display_name,
        extension,
        mime: detected_mime(&content).to_owned(),
        content: content.to_vec(),
        visibility_override,
        ttl_seconds,
        metadata,
    })
}

fn generated_object_key(original_name: Option<&str>) -> String {
    let extension = original_name
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .filter(|extension| {
            !extension.is_empty()
                && extension.len() <= 32
                && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
        .map(str::to_ascii_lowercase);
    let id = uuid::Uuid::now_v7().simple();
    match extension {
        Some(extension) => format!("uploads/{id}.{extension}"),
        None => format!("uploads/{id}"),
    }
}

fn normalized_mime(value: &str) -> Result<String, ApiError> {
    let value = value.trim();
    let valid = value
        .split_once('/')
        .is_some_and(|(type_part, subtype_part)| {
            !type_part.is_empty()
                && !subtype_part.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii() && !byte.is_ascii_control() && byte != b' ')
        });
    if !valid {
        return Err(ApiError::bad_request("content_type is invalid"));
    }
    Ok(value.to_ascii_lowercase())
}

fn detected_mime(content: &[u8]) -> &'static str {
    if content.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png";
    }
    if content.starts_with(&[0xff, 0xd8, 0xff]) {
        return "image/jpeg";
    }
    if content.starts_with(b"GIF87a") || content.starts_with(b"GIF89a") {
        return "image/gif";
    }
    if content.len() >= 12 && &content[..4] == b"RIFF" && &content[8..12] == b"WEBP" {
        return "image/webp";
    }
    if content.starts_with(b"%PDF-") {
        return "application/pdf";
    }
    let prefix = trim_ascii_start(content);
    if starts_with_ascii_case_insensitive(prefix, b"<!doctype html")
        || starts_with_ascii_case_insensitive(prefix, b"<html")
    {
        return "text/html";
    }
    if starts_with_ascii_case_insensitive(prefix, b"<svg")
        || (starts_with_ascii_case_insensitive(prefix, b"<?xml")
            && prefix
                .windows(4)
                .any(|window| window.eq_ignore_ascii_case(b"<svg")))
    {
        return "image/svg+xml";
    }
    if std::str::from_utf8(content).is_ok() {
        "text/plain"
    } else {
        "application/octet-stream"
    }
}

fn trim_ascii_start(value: &[u8]) -> &[u8] {
    let index = value
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(value.len());
    &value[index..]
}

fn starts_with_ascii_case_insensitive(value: &[u8], expected: &[u8]) -> bool {
    value
        .get(..expected.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(expected))
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    cookie_value(headers, SESSION_COOKIE)
}

fn cookie_value<'a>(headers: &'a HeaderMap, expected_name: &str) -> Option<&'a str> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|part| {
            let (name, value) = part.trim().split_once('=')?;
            (name == expected_name).then_some(value)
        })
}

fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn generate_auth_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::random::<[u8; 32]>())
}

fn validate_one_time_token(token: &str) -> Result<(), ApiError> {
    if (20..=512).contains(&token.len())
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(ApiError::invalid_one_time_token())
    }
}

#[allow(clippy::too_many_arguments)]
fn enforce_auth_rate_limit(
    state: &AppState,
    operation: &str,
    connect_info: &ConnectInfo<SocketAddr>,
    subject: Option<&str>,
    ip_limit: u32,
    subject_limit: u32,
    window: StdDuration,
) -> Result<(), ApiError> {
    let ip = connect_info.0.ip().to_string();
    state.auth_rate_limiter.check(
        format!("{operation}:ip:{}", token_hash(&ip)),
        ip_limit,
        window,
    )?;
    if let Some(subject) = subject {
        let subject = subject.trim().to_ascii_lowercase();
        state.auth_rate_limiter.check(
            format!("{operation}:subject:{}", token_hash(&subject)),
            subject_limit,
            window,
        )?;
    }
    Ok(())
}

impl AuthRateLimiter {
    fn check(&self, key: String, limit: u32, window: StdDuration) -> Result<(), ApiError> {
        let mut buckets = self
            .buckets
            .lock()
            .map_err(|_| ApiError::unavailable("authentication rate limiter is unavailable"))?;
        if buckets.len() > 4096 {
            buckets
                .retain(|_, bucket| bucket.window_started.elapsed() < StdDuration::from_secs(3600));
        }
        let bucket = buckets.entry(key).or_insert_with(|| AuthRateBucket {
            window_started: Instant::now(),
            attempts: 0,
        });
        if bucket.window_started.elapsed() >= window {
            bucket.window_started = Instant::now();
            bucket.attempts = 0;
        }
        if bucket.attempts >= limit {
            return Err(ApiError::rate_limited());
        }
        bucket.attempts += 1;
        Ok(())
    }
}

fn summarized_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.chars().take(256).collect())
}

