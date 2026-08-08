-- Metadata-only candidate scans for the standard S3 Lifecycle executor.
CREATE INDEX buckets_s3_lifecycle_scan_idx
    ON buckets(id)
    WHERE s3_lifecycle_configuration IS NOT NULL;

CREATE INDEX objects_s3_lifecycle_prefix_idx
    ON objects(application_id, bucket_id, object_key COLLATE "C", id)
    INCLUDE (current_version_id)
    WHERE current_version_id IS NOT NULL;

CREATE INDEX object_versions_lifecycle_current_idx
    ON object_versions(application_id, bucket_id, created_at, id)
    WHERE state = 'committed'
      AND superseded_at IS NULL
      AND NOT is_delete_marker;

CREATE INDEX object_versions_lifecycle_noncurrent_idx
    ON object_versions(application_id, bucket_id, became_noncurrent_at, id)
    WHERE state = 'committed'
      AND superseded_at IS NULL
      AND became_noncurrent_at IS NOT NULL
      AND NOT is_delete_marker;

CREATE INDEX object_versions_lifecycle_marker_idx
    ON object_versions(application_id, bucket_id, created_at, id)
    INCLUDE (object_id)
    WHERE state = 'committed'
      AND superseded_at IS NULL
      AND is_delete_marker;

CREATE INDEX s3_multipart_lifecycle_idx
    ON s3_multipart_uploads(
        application_id,
        bucket_id,
        created_at,
        upload_id COLLATE "C"
    )
    INCLUDE (object_key)
    WHERE state IN ('pending', 'completing');
