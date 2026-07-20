-- Keep application ownership atomic for bucket-scoped aggregates.  The
-- previous schema allowed application_id and bucket_id to be supplied from
-- different tenants because each column had an independent foreign key.
ALTER TABLE buckets
    ADD CONSTRAINT buckets_id_application_id_key UNIQUE (id, application_id);

ALTER TABLE media
    DROP CONSTRAINT IF EXISTS media_application_id_fkey,
    DROP CONSTRAINT IF EXISTS media_bucket_id_fkey;
ALTER TABLE media
    ADD CONSTRAINT media_bucket_application_fkey
        FOREIGN KEY (bucket_id, application_id)
        REFERENCES buckets (id, application_id);

ALTER TABLE upload_sessions
    DROP CONSTRAINT IF EXISTS upload_sessions_application_id_fkey,
    DROP CONSTRAINT IF EXISTS upload_sessions_bucket_id_fkey;
ALTER TABLE upload_sessions
    ADD CONSTRAINT upload_sessions_bucket_application_fkey
        FOREIGN KEY (bucket_id, application_id)
        REFERENCES buckets (id, application_id);

ALTER TABLE s3_multipart_uploads
    DROP CONSTRAINT IF EXISTS s3_multipart_uploads_application_id_fkey,
    DROP CONSTRAINT IF EXISTS s3_multipart_uploads_bucket_id_fkey;
ALTER TABLE s3_multipart_uploads
    ADD CONSTRAINT s3_multipart_uploads_bucket_application_fkey
        FOREIGN KEY (bucket_id, application_id)
        REFERENCES buckets (id, application_id);

-- Audit records are historical evidence and must survive application deletion.
-- The application_id remains a stable historical identifier, but is no longer
-- a lifecycle FK that prevents aggregate deletion.
ALTER TABLE audit_logs
    DROP CONSTRAINT IF EXISTS audit_logs_application_id_fkey;

DROP INDEX IF EXISTS webhook_deliveries_history_cursor_idx;
CREATE INDEX webhook_deliveries_history_cursor_idx
    ON webhook_deliveries(endpoint_id, history_id DESC);
