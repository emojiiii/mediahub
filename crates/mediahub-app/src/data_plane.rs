use mediahub_core::{ApplicationId, BucketId, Media, MediaId, MediaState, OffsetDateTime};

#[derive(Clone, Debug, Default)]
pub struct MediaListQuery {
    pub bucket_id: Option<BucketId>,
    pub state: Option<MediaState>,
    pub mime: Option<String>,
    pub created_from: Option<OffsetDateTime>,
    pub created_before: Option<OffsetDateTime>,
    pub object_key_prefix: Option<String>,
    pub cursor: Option<MediaListCursor>,
    pub limit: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaListCursor {
    pub created_at: OffsetDateTime,
    pub id: MediaId,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaPage {
    pub items: Vec<Media>,
    pub has_more: bool,
}

#[derive(Clone, Debug)]
pub struct MediaDirectoryListQuery {
    pub bucket_id: BucketId,
    pub state: Option<MediaState>,
    pub mime: Option<String>,
    pub created_from: Option<OffsetDateTime>,
    pub created_before: Option<OffsetDateTime>,
    pub object_key_prefix: String,
    pub cursor: Option<MediaDirectoryListCursor>,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MediaDirectoryListCursor {
    pub entry_key: String,
    pub is_prefix: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaDirectoryPage {
    pub items: Vec<Media>,
    pub common_prefixes: Vec<String>,
    pub next_cursor: Option<MediaDirectoryListCursor>,
}

#[derive(Clone, Debug)]
pub struct S3MediaListQuery {
    pub bucket_id: BucketId,
    pub object_key_prefix: String,
    pub start_after: Option<String>,
    pub delimiter: bool,
    pub limit: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct S3MediaPage {
    pub items: Vec<Media>,
    pub common_prefixes: Vec<String>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdempotencyClaim {
    Claimed,
    InProgress,
    Completed(CompletedIdempotencyResponse),
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletedIdempotencyResponse {
    pub status: u16,
    pub payload: String,
    pub resource_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdempotencyContext {
    pub application_id: ApplicationId,
    pub operation_scope: String,
    pub key: String,
    pub request_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingMediaDeletion {
    pub media_id: MediaId,
    pub storage_key: String,
}
