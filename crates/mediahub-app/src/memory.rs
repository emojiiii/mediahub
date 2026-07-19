//! In-memory adapters for tests and local application composition.

//! In-memory fakes for application tests.
//!
//! These types deliberately model transactional upload semantics. They are not
//! production storage or a replacement for the PostgreSQL adapter.

#![allow(clippy::missing_panics_doc)]

use std::{
    collections::{BTreeMap, HashMap},
    ops::Range,
    sync::{Arc, Mutex, MutexGuard},
};

use async_trait::async_trait;
use mediahub_core::{
    ApplicationId, Bucket, BucketId, Media, MediaId, MediaState, OffsetDateTime, UploadSession,
    UploadSessionId, UploadSessionState,
};
use sha2::{Digest, Sha256};

use crate::{
    BucketRepository, Clock, ComposedObject, MediaRepository, ObjectMetadata, ObjectPage,
    ObjectStore, ObjectStoreError, OutboxEvent, OutboxRepository, PreparedUpload, RepositoryError,
    StoredUpload, UploadSessionCancellation, UploadSessionCompletion, UploadSessionExpiration,
    UploadSessionRepository, UploadSessionStorage, UploadTarget,
};

const MEMORY_BACKEND: &str = "memory";

include!("memory_object_store.rs");
include!("memory_bucket.rs");
include!("memory_media.rs");
include!("memory_upload.rs");
include!("memory_outbox.rs");
