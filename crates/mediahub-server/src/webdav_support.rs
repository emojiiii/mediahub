// WebDAV metadata, path, and error helpers.

fn direct_entries(media: &[Media], prefix: &str) -> Vec<DavEntry> {
    let mut entries = BTreeMap::<String, DavEntry>::new();
    for item in media {
        let Some(remainder) = item.object_key().strip_prefix(prefix) else {
            continue;
        };
        if remainder.is_empty() {
            continue;
        }
        if let Some((directory, _)) = remainder.split_once('/') {
            entries.insert(
                directory.to_owned(),
                DavEntry::directory(directory.as_bytes().to_vec(), UNIX_EPOCH),
            );
        } else {
            entries
                .entry(remainder.to_owned())
                .or_insert_with(|| DavEntry::file(remainder.as_bytes().to_vec(), item));
        }
    }
    entries.into_values().collect()
}

fn directory_prefix(object_key: &str) -> String {
    if object_key.ends_with('/') {
        object_key.to_owned()
    } else {
        format!("{object_key}/")
    }
}

fn guess_mime(object_key: &str) -> String {
    mime_guess::from_path(object_key)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_owned()
}

fn checked_seek(base: u64, offset: i64) -> FsResult<u64> {
    if offset < 0 {
        base.checked_sub(offset.unsigned_abs())
            .ok_or(FsError::Forbidden)
    } else {
        base.checked_add(offset as u64).ok_or(FsError::TooLarge)
    }
}

fn to_system_time(value: OffsetDateTime) -> SystemTime {
    let seconds = value.unix_timestamp();
    let nanos = value.nanosecond();
    if seconds >= 0 {
        UNIX_EPOCH + Duration::new(seconds as u64, nanos)
    } else {
        UNIX_EPOCH - Duration::new(seconds.unsigned_abs(), nanos)
    }
}

fn map_domain_error(error: DomainError) -> FsError {
    match error {
        DomainError::ObjectTooLarge { .. } => FsError::TooLarge,
        _ => FsError::Forbidden,
    }
}

fn map_repository_error(error: mediahub_app::RepositoryError) -> FsError {
    match error {
        mediahub_app::RepositoryError::Conflict => FsError::Exists,
        mediahub_app::RepositoryError::QuotaExceeded => FsError::InsufficientStorage,
        _ => FsError::GeneralFailure,
    }
}

fn map_application_error(error: ApplicationError) -> FsError {
    match error {
        ApplicationError::BucketNotFound => FsError::NotFound,
        ApplicationError::BucketDoesNotBelongToApplication => FsError::Forbidden,
        ApplicationError::ObjectAlreadyExists => FsError::Exists,
        ApplicationError::QuotaExceeded => FsError::InsufficientStorage,
        ApplicationError::Domain(error) => map_domain_error(error),
        ApplicationError::Repository(error) => map_repository_error(error),
        ApplicationError::ObjectStore(_) => FsError::GeneralFailure,
        _ => FsError::GeneralFailure,
    }
}

struct DavClock;

impl Clock for DavClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}
