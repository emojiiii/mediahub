use md5::{Digest as _, Md5};
use mediahub_app::S3MultipartPart;
use mediahub_core::EntityTag;

const MULTIPART_STORAGE_ROOT: &str = "temporary/s3-multipart";

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

/// S3 multipart ETag: MD5 over the concatenated binary per-part MD5 digests,
/// followed by the selected part count. Parts must already be manifest ordered.
pub(super) fn multipart_entity_tag(parts: &[S3MultipartPart]) -> Option<EntityTag> {
    if parts.is_empty() {
        return None;
    }
    let mut digest = Md5::new();
    for part in parts {
        let binary = hex::decode(&part.md5).ok()?;
        if binary.len() != 16 {
            return None;
        }
        digest.update(binary);
    }
    EntityTag::new(format!(
        "{}-{}",
        hex::encode(digest.finalize()),
        parts.len()
    ))
    .ok()
}

#[cfg(test)]
mod tests {
    use mediahub_app::S3MultipartPart;
    use time::OffsetDateTime;

    use super::*;

    fn part(number: u16, md5: &str) -> S3MultipartPart {
        S3MultipartPart {
            upload_id: "mh_mpu_example".into(),
            part_number: number,
            size: 1,
            sha256: "0".repeat(64),
            md5: md5.into(),
            etag: md5.into(),
            storage_key: format!("part-{number}"),
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn multipart_storage_keys_are_scoped_and_unique() {
        let upload_id = "mh_mpu_example";
        assert_eq!(
            multipart_upload_prefix(upload_id),
            "temporary/s3-multipart/mh_mpu_example/"
        );
        let first = new_multipart_part_storage_key(upload_id, 7);
        let second = new_multipart_part_storage_key(upload_id, 7);
        assert!(first.starts_with("temporary/s3-multipart/mh_mpu_example/7/"));
        assert_ne!(first, second);
    }

    #[test]
    fn multipart_etag_is_md5_of_binary_part_md5_values() {
        let parts = [
            part(1, "c4ca4238a0b923820dcc509a6f75849b"),
            part(2, "c81e728d9d4c2f636f067f89cc14862c"),
        ];
        assert_eq!(
            multipart_entity_tag(&parts).expect("etag").as_str(),
            "62be021ce84139205cdaf464abcd82ff-2"
        );
    }
}
