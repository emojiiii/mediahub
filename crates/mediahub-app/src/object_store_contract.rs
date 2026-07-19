//! Shared behavioral checks for `ObjectStore` adapters.

use crate::{ObjectStore, ObjectStoreError};
use sha2::{Digest, Sha256};

/// Verifies the backend-neutral immutable object-store contract.
///
/// Callers must provide a unique, backend-valid namespace because the suite
/// creates and deletes objects below it.
///
/// # Errors
///
/// Returns a diagnostic when the adapter violates the shared contract.
pub async fn verify_object_store_contract<S>(store: &S, namespace: &str) -> Result<(), String>
where
    S: ObjectStore + ?Sized,
{
    if store.backend_name().trim().is_empty() {
        return Err("backend_name must not be empty".into());
    }

    let first_temporary = format!("{namespace}/temporary/first");
    let second_temporary = format!("{namespace}/temporary/second");
    let other_temporary = format!("{namespace}/temporary/other");
    let missing_temporary = format!("{namespace}/temporary/missing");
    let compose_first = format!("{namespace}/temporary/compose-first");
    let compose_second = format!("{namespace}/temporary/compose-second");
    let composed_temporary = format!("{namespace}/temporary/composed");
    let final_key = format!("{namespace}/objects/final");
    let other_key = format!("{namespace}/objects/other");
    let composed_key = format!("{namespace}/objects/composed");
    let object_prefix = format!("{namespace}/objects");

    store
        .put_temporary(&first_temporary, b"first", "text/plain")
        .await
        .map_err(|error| format!("first temporary write failed: {error}"))?;
    if store
        .exists(&final_key)
        .await
        .map_err(|error| format!("pre-commit visibility check failed: {error}"))?
        || store.read(&final_key).await != Err(ObjectStoreError::NotFound)
    {
        return Err("staged content must not be visible at the final key".into());
    }
    if store
        .put_temporary(&first_temporary, b"replacement", "text/plain")
        .await
        != Err(ObjectStoreError::AlreadyExists)
    {
        return Err("temporary writes must not overwrite existing content".into());
    }
    store
        .commit_temporary(&first_temporary, &final_key)
        .await
        .map_err(|error| format!("first commit failed: {error}"))?;
    if store
        .exists(&first_temporary)
        .await
        .map_err(|error| format!("post-commit temporary check failed: {error}"))?
    {
        return Err("commit must remove the temporary object".into());
    }

    if !store
        .exists(&final_key)
        .await
        .map_err(|error| format!("exists failed: {error}"))?
    {
        return Err("committed object must exist".into());
    }
    if store
        .read(&final_key)
        .await
        .map_err(|error| format!("committed read failed: {error}"))?
        != b"first"
    {
        return Err("committed bytes changed".into());
    }
    let metadata = store
        .head(&final_key)
        .await
        .map_err(|error| format!("head failed: {error}"))?;
    if metadata.key != final_key
        || metadata.size != 5
        || metadata.content_type.as_deref() != Some("text/plain")
    {
        return Err("head returned incorrect key, size, or Content-Type".into());
    }
    let expected_sha256 = hex::encode(Sha256::digest(b"first"));
    if metadata
        .checksum_sha256
        .as_deref()
        .is_some_and(|checksum| checksum != expected_sha256)
    {
        return Err("head returned an incorrect SHA-256 checksum".into());
    }
    if store
        .read_range(&final_key, 1..4)
        .await
        .map_err(|error| format!("range read failed: {error}"))?
        != b"irs"
    {
        return Err("range read returned incorrect bytes".into());
    }
    if store.read_range(&final_key, 4..6).await != Err(ObjectStoreError::InvalidRange)
        || store.read_range(&final_key, 2..2).await != Err(ObjectStoreError::InvalidRange)
    {
        return Err("invalid byte ranges must report InvalidRange".into());
    }

    store
        .put_temporary(&second_temporary, b"second", "text/plain")
        .await
        .map_err(|error| format!("second temporary write failed: {error}"))?;
    if store.commit_temporary(&second_temporary, &final_key).await
        != Err(ObjectStoreError::AlreadyExists)
    {
        return Err("committing over an existing object must report AlreadyExists".into());
    }
    if store
        .read(&final_key)
        .await
        .map_err(|error| format!("read after conflict failed: {error}"))?
        != b"first"
    {
        return Err("commit conflict must preserve the original bytes".into());
    }

    if store
        .commit_temporary(&missing_temporary, &format!("{namespace}/objects/missing"))
        .await
        != Err(ObjectStoreError::NotFound)
    {
        return Err("committing a missing temporary object must report NotFound".into());
    }

    store
        .put_temporary(&compose_first, b"multipart-", "application/octet-stream")
        .await
        .map_err(|error| format!("first composition part failed: {error}"))?;
    store
        .put_temporary(&compose_second, b"content", "application/octet-stream")
        .await
        .map_err(|error| format!("second composition part failed: {error}"))?;
    let composed = store
        .compose_temporary(
            &composed_temporary,
            &[compose_first.clone(), compose_second.clone()],
            "application/octet-stream",
        )
        .await
        .map_err(|error| format!("temporary composition failed: {error}"))?;
    let expected_composed = b"multipart-content";
    if composed.size != expected_composed.len() as u64
        || composed.sha256 != hex::encode(Sha256::digest(expected_composed))
    {
        return Err("composition returned incorrect size or SHA-256".into());
    }
    store
        .commit_temporary(&composed_temporary, &composed_key)
        .await
        .map_err(|error| format!("composed commit failed: {error}"))?;
    if store
        .read(&composed_key)
        .await
        .map_err(|error| format!("composed read failed: {error}"))?
        != expected_composed
    {
        return Err("composition changed part order or bytes".into());
    }
    for key in [&compose_first, &compose_second, &composed_key] {
        store
            .delete(key)
            .await
            .map_err(|error| format!("composition cleanup failed: {error}"))?;
    }

    store
        .put_temporary(&other_temporary, b"other", "text/plain")
        .await
        .map_err(|error| format!("other temporary write failed: {error}"))?;
    store
        .commit_temporary(&other_temporary, &other_key)
        .await
        .map_err(|error| format!("other commit failed: {error}"))?;
    if store.list(&object_prefix, None, 0).await != Err(ObjectStoreError::InvalidLimit) {
        return Err("a zero list limit must report InvalidLimit".into());
    }
    if store.list(&object_prefix, Some("outside/cursor"), 1).await
        != Err(ObjectStoreError::InvalidCursor)
    {
        return Err("a cursor outside the prefix must report InvalidCursor".into());
    }
    let first_page = store
        .list(&object_prefix, None, 1)
        .await
        .map_err(|error| format!("first list page failed: {error}"))?;
    if first_page.objects.len() != 1 || first_page.next_cursor.is_none() {
        return Err("the first list page must contain one object and a cursor".into());
    }
    let cursor = first_page.next_cursor.as_deref().expect("checked cursor");
    if first_page.objects[0].key != cursor {
        return Err("the list cursor must identify the last returned object".into());
    }
    let second_page = store
        .list(&object_prefix, Some(cursor), 1)
        .await
        .map_err(|error| format!("second list page failed: {error}"))?;
    if second_page.objects.len() != 1
        || second_page.next_cursor.is_some()
        || second_page.objects[0].key <= first_page.objects[0].key
    {
        return Err("cursor pagination must return the remaining object in key order".into());
    }

    store
        .delete(&final_key)
        .await
        .map_err(|error| format!("first delete failed: {error}"))?;
    store
        .delete(&final_key)
        .await
        .map_err(|error| format!("repeated delete failed: {error}"))?;
    if store
        .exists(&final_key)
        .await
        .map_err(|error| format!("exists after delete failed: {error}"))?
    {
        return Err("deleted object must not exist".into());
    }
    if store.read(&final_key).await != Err(ObjectStoreError::NotFound) {
        return Err("reading a deleted object must report NotFound".into());
    }
    if store.head(&final_key).await != Err(ObjectStoreError::NotFound) {
        return Err("heading a deleted object must report NotFound".into());
    }

    store
        .delete(&second_temporary)
        .await
        .map_err(|error| format!("temporary cleanup failed: {error}"))?;
    store
        .delete(&other_key)
        .await
        .map_err(|error| format!("other object cleanup failed: {error}"))?;
    Ok(())
}
