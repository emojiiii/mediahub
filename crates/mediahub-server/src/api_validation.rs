// Shared validation, cursor, and expiration helpers.

fn validate_application_name(value: String) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(ApiError::bad_request("application name is invalid"));
    }
    Ok(value.to_owned())
}

fn validate_access_key_name(value: String) -> Result<String, ApiError> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(ApiError::bad_request("access key name is invalid"));
    }
    Ok(value.to_owned())
}

fn validate_permissions(values: Vec<String>) -> Result<Vec<String>, ApiError> {
    if values.is_empty() {
        return Err(ApiError::bad_request(
            "at least one access key permission is required",
        ));
    }
    let permissions = values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    if permissions.len() > ACCESS_KEY_PERMISSIONS.len()
        || permissions.is_empty()
        || permissions
            .iter()
            .any(|permission| !ACCESS_KEY_PERMISSIONS.contains(&permission.as_str()))
    {
        return Err(ApiError::bad_request("access key permissions are invalid"));
    }
    Ok(permissions.into_iter().collect())
}

fn validate_webhook_events(values: Vec<String>) -> Result<Vec<String>, ApiError> {
    let events = values
        .into_iter()
        .map(|value| value.trim().to_owned())
        .collect::<std::collections::BTreeSet<_>>();
    if events.is_empty()
        || events.len() > WEBHOOK_EVENT_TYPES.len()
        || events
            .iter()
            .any(|event| !WEBHOOK_EVENT_TYPES.contains(&event.as_str()))
    {
        return Err(ApiError::bad_request(
            "webhook event subscriptions are invalid",
        ));
    }
    Ok(events.into_iter().collect())
}

fn validate_webhook_url(value: String) -> Result<String, ApiError> {
    let url =
        Url::parse(value.trim()).map_err(|_| ApiError::bad_request("webhook URL is invalid"))?;
    validate_webhook_url_parsed(&url)
        .map_err(|_| ApiError::bad_request("webhook URL is not permitted"))?;
    Ok(url.into())
}

fn validate_webhook_url_parsed(url: &Url) -> Result<(), ()> {
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(());
    }
    match url.host().expect("host was checked") {
        Host::Domain(host) => {
            let host = host.to_ascii_lowercase();
            if host == "localhost"
                || host.ends_with(".localhost")
                || host.ends_with(".local")
                || host.ends_with(".internal")
            {
                return Err(());
            }
        }
        Host::Ipv4(ip) if !is_public_webhook_ip(IpAddr::V4(ip)) => return Err(()),
        Host::Ipv6(ip) if !is_public_webhook_ip(IpAddr::V6(ip)) => return Err(()),
        Host::Ipv4(_) | Host::Ipv6(_) => {}
    }
    Ok(())
}

fn is_public_webhook_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [first, second, third, _] = ip.octets();
            !(first == 0
                || first >= 240
                || (first == 100 && (64..=127).contains(&second))
                || (first == 192 && second == 0 && third == 0)
                || (first == 198 && matches!(second, 18 | 19))
                || ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation())
        }
        IpAddr::V6(ip) => {
            if let Some(ip) = ip.to_ipv4_mapped() {
                return is_public_webhook_ip(IpAddr::V4(ip));
            }
            let segments = ip.segments();
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8))
        }
    }
}

fn expiration_from_request(
    expires_at: Option<String>,
    now: OffsetDateTime,
) -> Result<Option<OffsetDateTime>, ApiError> {
    let Some(expires_at) = expires_at else {
        return Ok(None);
    };
    let expires_at = chrono::DateTime::parse_from_rfc3339(&expires_at)
        .map_err(|_| ApiError::bad_request("access key expires_at is invalid"))?
        .timestamp();
    let expires_at = OffsetDateTime::from_unix_timestamp(expires_at)
        .map_err(|_| ApiError::bad_request("access key expires_at is invalid"))?;
    if expires_at <= now {
        return Err(ApiError::bad_request(
            "access key expires_at must be in the future",
        ));
    }
    Ok(Some(expires_at))
}

fn expiration_from_ttl(seconds: u64, now: OffsetDateTime) -> Result<OffsetDateTime, ApiError> {
    let seconds =
        i64::try_from(seconds).map_err(|_| ApiError::bad_request("ttl_seconds is too large"))?;
    Ok(now + time::Duration::seconds(seconds))
}

fn parse_query_time(value: &str) -> Result<OffsetDateTime, ApiError> {
    let timestamp = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| ApiError::bad_request("time filter must be RFC 3339"))?
        .timestamp();
    OffsetDateTime::from_unix_timestamp(timestamp)
        .map_err(|_| ApiError::bad_request("time filter is outside the supported range"))
}

fn parse_media_state(value: &str) -> Result<MediaState, ApiError> {
    match value {
        "uploading" => Ok(MediaState::Uploading),
        "active" => Ok(MediaState::Active),
        "archive_pending" => Ok(MediaState::ArchivePending),
        "archived" => Ok(MediaState::Archived),
        "delete_pending" => Ok(MediaState::DeletePending),
        "deleted" => Ok(MediaState::Deleted),
        "quarantined" => Ok(MediaState::Quarantined),
        _ => Err(ApiError::bad_request("status filter is invalid")),
    }
}

fn encode_media_cursor(cursor: MediaListCursor) -> String {
    URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&MediaCursorToken {
            created_at: cursor.created_at.unix_timestamp(),
            id: cursor.id.to_string(),
        })
        .expect("media cursor serializes"),
    )
}

fn encode_media_directory_cursor(cursor: MediaDirectoryListCursor) -> String {
    URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&MediaDirectoryCursorToken {
            entry_key: cursor.entry_key,
            is_prefix: cursor.is_prefix,
        })
        .expect("media directory cursor serializes"),
    )
}

fn decode_media_directory_cursor(value: &str) -> Result<MediaDirectoryListCursor, ApiError> {
    if value.is_empty() || value.len() > 4096 {
        return Err(ApiError::bad_request("cursor is invalid"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ApiError::bad_request("cursor is invalid"))?;
    let cursor = serde_json::from_slice::<MediaDirectoryCursorToken>(&bytes)
        .map_err(|_| ApiError::bad_request("cursor is invalid"))?;
    if cursor.entry_key.is_empty()
        || cursor.entry_key.len() > 1024
        || cursor.entry_key.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ApiError::bad_request("cursor is invalid"));
    }
    Ok(MediaDirectoryListCursor {
        entry_key: cursor.entry_key,
        is_prefix: cursor.is_prefix,
    })
}

fn parse_list_delimiter(value: Option<&str>) -> Result<bool, ApiError> {
    match value {
        None => Ok(false),
        Some("/") => Ok(true),
        Some(_) => Err(ApiError::bad_request("delimiter must be /")),
    }
}

fn decode_media_cursor(value: &str) -> Result<MediaListCursor, ApiError> {
    if value.is_empty() || value.len() > 1024 {
        return Err(ApiError::bad_request("cursor is invalid"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ApiError::bad_request("cursor is invalid"))?;
    let cursor = serde_json::from_slice::<MediaCursorToken>(&bytes)
        .map_err(|_| ApiError::bad_request("cursor is invalid"))?;
    Ok(MediaListCursor {
        created_at: OffsetDateTime::from_unix_timestamp(cursor.created_at)
            .map_err(|_| ApiError::bad_request("cursor is invalid"))?,
        id: MediaId::from_str(&cursor.id)
            .map_err(|_| ApiError::bad_request("cursor is invalid"))?,
    })
}

fn parse_webhook_delivery_status(value: &str) -> Result<WebhookDeliveryHistoryStatus, ApiError> {
    match value {
        "pending" => Ok(WebhookDeliveryHistoryStatus::Pending),
        "delivered" => Ok(WebhookDeliveryHistoryStatus::Delivered),
        "dead_lettered" => Ok(WebhookDeliveryHistoryStatus::DeadLettered),
        _ => Err(ApiError::bad_request(
            "webhook delivery status filter is invalid",
        )),
    }
}

fn webhook_delivery_status_name(value: WebhookDeliveryHistoryStatus) -> &'static str {
    match value {
        WebhookDeliveryHistoryStatus::Pending => "pending",
        WebhookDeliveryHistoryStatus::Delivered => "delivered",
        WebhookDeliveryHistoryStatus::DeadLettered => "dead_lettered",
    }
}

fn encode_webhook_delivery_cursor(cursor: WebhookDeliveryHistoryCursor) -> String {
    URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&WebhookDeliveryCursorToken {
            updated_at: cursor.updated_at.unix_timestamp(),
            row_id: cursor.row_id,
        })
        .expect("webhook delivery cursor serializes"),
    )
}

fn decode_webhook_delivery_cursor(value: &str) -> Result<WebhookDeliveryHistoryCursor, ApiError> {
    if value.is_empty() || value.len() > 1024 {
        return Err(ApiError::bad_request("cursor is invalid"));
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|_| ApiError::bad_request("cursor is invalid"))?;
    let cursor = serde_json::from_slice::<WebhookDeliveryCursorToken>(&bytes)
        .map_err(|_| ApiError::bad_request("cursor is invalid"))?;
    if cursor.row_id <= 0 {
        return Err(ApiError::bad_request("cursor is invalid"));
    }
    Ok(WebhookDeliveryHistoryCursor {
        updated_at: OffsetDateTime::from_unix_timestamp(cursor.updated_at)
            .map_err(|_| ApiError::bad_request("cursor is invalid"))?,
        row_id: cursor.row_id,
    })
}
impl From<Media> for MediaResponse {
    fn from(media: Media) -> Self {
        let dimensions = media.dimensions();
        Self {
            id: media.id().to_string(),
            bucket_id: media.bucket_id().to_string(),
            object_key: media.object_key().to_owned(),
            display_name: media.display_name().to_owned(),
            state: media.state(),
            mime: media.mime().to_owned(),
            size_bytes: media.size(),
            sha256: media.sha256().to_owned(),
            revision: media.revision(),
            width: dimensions.map(|value| value.width()),
            height: dimensions.map(|value| value.height()),
            visibility: media.visibility_override(),
            expires_at: media.expire_at(),
            metadata: serde_json::json!({
                "user": media.metadata().user(),
                "ai": media.metadata().ai(),
            }),
            created_at: media.created_at(),
            updated_at: media.updated_at(),
        }
    }
}

