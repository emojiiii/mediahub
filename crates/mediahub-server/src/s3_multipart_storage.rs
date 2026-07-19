use mediahub_app::ObjectStore;
use tracing::warn;

const MULTIPART_STORAGE_ROOT: &str = "temporary/s3-multipart";
const CLEANUP_PAGE_SIZE: usize = 1_000;

pub(super) fn multipart_upload_prefix(upload_id: &str) -> String {
    format!("{MULTIPART_STORAGE_ROOT}/{upload_id}/")
}

pub(super) fn new_multipart_part_storage_key(upload_id: &str, part_number: u16) -> String {
    format!(
        "{}{part_number}/{}",
        multipart_upload_prefix(upload_id),
        uuid::Uuid::now_v7().simple()
    )
}

pub(super) fn new_multipart_completion_storage_key(upload_id: &str) -> String {
    format!(
        "{}complete/{}",
        multipart_upload_prefix(upload_id),
        uuid::Uuid::now_v7().simple()
    )
}

pub(super) async fn cleanup_multipart_storage<S>(object_store: &S, upload_id: &str) -> bool
where
    S: ObjectStore + ?Sized,
{
    let prefix = multipart_upload_prefix(upload_id);
    let mut cursor = None;
    let mut cleaned = true;
    loop {
        let page = match object_store
            .list(&prefix, cursor.as_deref(), CLEANUP_PAGE_SIZE)
            .await
        {
            Ok(page) => page,
            Err(error) => {
                warn!(upload_id, error = %error, "S3 multipart temporary object listing failed");
                return false;
            }
        };
        let next_cursor = page.next_cursor;
        for object in page.objects {
            if let Err(error) = object_store.delete(&object.key).await {
                warn!(upload_id, storage_key = %object.key, error = %error, "S3 multipart temporary object cleanup failed");
                cleaned = false;
            }
        }
        let Some(next_cursor) = next_cursor else {
            break;
        };
        if cursor.as_deref() == Some(next_cursor.as_str()) {
            warn!(
                upload_id,
                "S3 multipart temporary object listing cursor did not advance"
            );
            return false;
        }
        cursor = Some(next_cursor);
    }
    cleaned
}

#[cfg(test)]
mod tests {
    use mediahub_adapter_local::LocalObjectStore;
    use mediahub_app::ObjectStore;

    use super::*;

    #[test]
    fn multipart_storage_keys_are_scoped_and_unique() {
        let upload_id = "mh_mpu_example";
        assert_eq!(
            multipart_upload_prefix(upload_id),
            "temporary/s3-multipart/mh_mpu_example/"
        );

        let first_part = new_multipart_part_storage_key(upload_id, 7);
        let second_part = new_multipart_part_storage_key(upload_id, 7);
        assert!(first_part.starts_with("temporary/s3-multipart/mh_mpu_example/7/"));
        assert_ne!(first_part, second_part);

        let completion = new_multipart_completion_storage_key(upload_id);
        assert!(completion.starts_with("temporary/s3-multipart/mh_mpu_example/complete/"));
    }

    #[tokio::test]
    async fn multipart_cleanup_is_paginated_scoped_and_idempotent() {
        let root = std::env::temp_dir().join(format!(
            "mediahub-multipart-cleanup-test-{}",
            uuid::Uuid::now_v7().simple()
        ));
        let store = LocalObjectStore::new(&root).expect("local object store");
        let target_prefix = multipart_upload_prefix("target");
        for index in 0..=CLEANUP_PAGE_SIZE {
            store
                .put_temporary(
                    &format!("{target_prefix}{index:04}"),
                    b"part",
                    "application/octet-stream",
                )
                .await
                .expect("temporary target part");
        }
        let other_key = new_multipart_part_storage_key("other", 1);
        store
            .put_temporary(&other_key, b"other", "application/octet-stream")
            .await
            .expect("temporary other part");

        assert!(cleanup_multipart_storage(&store, "target").await);
        assert!(
            store
                .list(&target_prefix, None, CLEANUP_PAGE_SIZE)
                .await
                .expect("target listing")
                .objects
                .is_empty()
        );
        assert_eq!(
            store
                .list(&multipart_upload_prefix("other"), None, CLEANUP_PAGE_SIZE,)
                .await
                .expect("other listing")
                .objects
                .len(),
            1
        );
        assert!(cleanup_multipart_storage(&store, "target").await);

        drop(store);
        std::fs::remove_dir_all(root).expect("remove multipart cleanup test storage");
    }
}
