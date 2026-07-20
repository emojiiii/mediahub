-- Expired/cancelled sessions remain until their physical object has been
-- removed.  This allows lifecycle workers to retry storage cleanup safely.
ALTER TABLE upload_sessions
    ADD COLUMN storage_cleanup_completed_at TIMESTAMPTZ;

CREATE INDEX upload_sessions_cleanup_idx
    ON upload_sessions(updated_at, id)
    WHERE state IN ('completed', 'expired', 'cancelled')
      AND storage_cleanup_completed_at IS NULL;
