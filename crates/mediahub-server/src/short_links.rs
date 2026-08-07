// Public-object short-link creation and redirect resolution.

const SHORT_LINK_CODE_BYTES: usize = 9;
const SHORT_LINK_CREATE_ATTEMPTS: usize = 4;
const MAX_SHORT_LINK_TARGET_URL_BYTES: usize = 8 * 1024;

struct ValidatedShortLinkTarget {
    bucket: String,
    object_key: String,
    canonical_path: String,
}

async fn create_short_link(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    hmac_identity: Option<Extension<HmacIdentity>>,
    Json(request): Json<CreateShortLinkRequest>,
) -> Result<(StatusCode, Json<ShortLinkResponse>), ApiError> {
    let auth =
        authenticated_application(&state, &headers, hmac_identity.map(|value| value.0)).await?;
    auth.verify_mutation_csrf(&state, &headers).await?;
    auth.authorize("media:read")?;

    let target = validate_short_link_target(&request.target_url, &auth.application.app_id)?;
    let bucket = state
        .repository
        .find_bucket_by_name(auth.application.id, &target.bucket)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("public object not found"))?;
    let media = state
        .repository
        .find_by_object_key(auth.application.id, bucket.id(), &target.object_key)
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("public object not found"))?;
    let now = OffsetDateTime::now_utc();
    media
        .ensure_readable()
        .map_err(|_| ApiError::not_found("public object not found"))?;
    if media.expire_at().is_some_and(|expires_at| expires_at <= now) {
        return Err(ApiError::not_found("public object not found"));
    }
    if media.effective_visibility(bucket.policy().visibility()) != Visibility::Public {
        return Err(ApiError::forbidden(
            "short links may only target public objects",
        ));
    }

    for _ in 0..SHORT_LINK_CREATE_ATTEMPTS {
        let code = URL_SAFE_NO_PAD.encode(rand::random::<[u8; SHORT_LINK_CODE_BYTES]>());
        match state
            .repository
            .create_short_link(
                &code,
                auth.application.id,
                media.id(),
                &target.canonical_path,
                None,
                now,
            )
            .await
        {
            Ok(record) => {
                return Ok((
                    StatusCode::CREATED,
                    Json(ShortLinkResponse {
                        url: format!("/s/{}", record.code),
                        target_url: record.target_path,
                        code: record.code,
                        expires_at: record.expires_at,
                        created_at: record.created_at,
                    }),
                ));
            }
            Err(mediahub_app::RepositoryError::Conflict) => continue,
            Err(error) => return Err(ApiError::from_repository(error)),
        }
    }
    Err(ApiError::unavailable(
        "a unique short-link code could not be allocated",
    ))
}

async fn redirect_short_link(
    State(state): State<Arc<AppState>>,
    Path(code): Path<String>,
) -> Result<axum::response::Redirect, ApiError> {
    validate_short_link_code(&code)?;
    let link = state
        .repository
        .find_public_short_link(&code, OffsetDateTime::now_utc())
        .await
        .map_err(ApiError::from_repository)?
        .ok_or_else(|| ApiError::not_found("short link not found"))?;
    Ok(axum::response::Redirect::temporary(&link.target_path))
}

fn validate_short_link_target(
    target_url: &str,
    expected_app_id: &str,
) -> Result<ValidatedShortLinkTarget, ApiError> {
    let target_url = target_url.trim();
    if target_url.is_empty() || target_url.len() > MAX_SHORT_LINK_TARGET_URL_BYTES {
        return Err(ApiError::bad_request("target_url is invalid"));
    }
    let url = Url::parse(target_url)
        .map_err(|_| ApiError::bad_request("target_url must be an absolute HTTP URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ApiError::bad_request(
            "target_url must be a public object URL without credentials, query, or fragment",
        ));
    }
    let segments = decoded_path_segments(&url)?;
    if segments.len() < 3 {
        return Err(ApiError::bad_request(
            "target_url must identify an application, bucket, and object",
        ));
    }
    if segments[0] != expected_app_id || segments[1].is_empty() {
        return Err(ApiError::bad_request(
            "target_url does not belong to the active application",
        ));
    }
    let object_key = segments[2..].join("/");
    if object_key.is_empty() || object_key.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(ApiError::bad_request("target_url object key is invalid"));
    }
    let canonical_path = object_content_path(expected_app_id, &segments[1], &object_key);
    let canonical_url = Url::parse(&format!("http://mediahub.invalid{canonical_path}"))
        .expect("canonical object paths form valid HTTP URLs");
    if decoded_path_segments(&canonical_url)? != segments {
        return Err(ApiError::bad_request(
            "target_url path is not in canonical object URL form",
        ));
    }
    Ok(ValidatedShortLinkTarget {
        bucket: segments[1].clone(),
        object_key,
        canonical_path,
    })
}

fn decoded_path_segments(url: &Url) -> Result<Vec<String>, ApiError> {
    url.path_segments()
        .ok_or_else(|| ApiError::bad_request("target_url path is invalid"))?
        .map(|segment| {
            percent_encoding::percent_decode_str(segment)
                .decode_utf8()
                .map(|value| value.into_owned())
                .map_err(|_| ApiError::bad_request("target_url path encoding is invalid"))
        })
        .collect()
}

fn validate_short_link_code(code: &str) -> Result<(), ApiError> {
    if (8..=32).contains(&code.len())
        && code
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        Ok(())
    } else {
        Err(ApiError::not_found("short link not found"))
    }
}

#[cfg(test)]
mod short_link_tests {
    use super::*;

    #[test]
    fn validates_and_canonicalizes_public_object_targets() {
        let target = validate_short_link_target(
            "https://media.example/app_demo/public/%E8%A7%86%E9%A2%91/demo%20clip.mp4",
            "app_demo",
        )
        .expect("valid object target");
        assert_eq!(target.bucket, "public");
        assert_eq!(target.object_key, "视频/demo clip.mp4");
        assert_eq!(
            target.canonical_path,
            "/app_demo/public/%E8%A7%86%E9%A2%91/demo%20clip.mp4"
        );
    }

    #[test]
    fn rejects_tokens_queries_foreign_apps_and_non_http_targets() {
        for target in [
            "https://media.example/app_demo/public/video.mp4?token=secret",
            "https://media.example/app_other/public/video.mp4",
            "javascript:alert(1)",
            "/app_demo/public/video.mp4",
        ] {
            assert!(
                validate_short_link_target(target, "app_demo").is_err(),
                "target should be rejected: {target}"
            );
        }
    }

    #[test]
    fn accepts_equivalent_browser_path_encoding_but_rejects_encoded_separators() {
        let target = validate_short_link_target(
            "https://media.example/app_demo/public/report%2Bfinal%3Dv1.mp4",
            "app_demo",
        )
        .expect("equivalent browser encoding should be accepted");
        assert_eq!(target.object_key, "report+final=v1.mp4");
        assert_eq!(target.canonical_path, "/app_demo/public/report+final=v1.mp4");

        assert!(
            validate_short_link_target(
                "https://media.example/app_demo/public/folder%2Fclip.mp4",
                "app_demo",
            )
            .is_err()
        );
    }

    #[test]
    fn short_link_codes_have_a_strict_public_route_alphabet() {
        assert!(validate_short_link_code("Abcdef_123-4").is_ok());
        assert!(validate_short_link_code("short").is_err());
        assert!(validate_short_link_code("has/slash").is_err());
    }
}
