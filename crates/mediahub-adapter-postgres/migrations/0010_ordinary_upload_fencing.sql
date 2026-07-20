-- Persist ownership for ordinary (non-multipart) upload promotion. Workers
-- may reconcile only expired leases and every terminal mutation is fenced by
-- the current token.
ALTER TABLE media
    ADD COLUMN upload_temporary_key TEXT,
    ADD COLUMN upload_lease_token TEXT,
    ADD COLUMN upload_leased_until TIMESTAMPTZ;

-- Existing uploading rows may belong to old instances during a rolling
-- deployment. Give them a compatibility lease long enough for those requests
-- to drain before a new worker may claim them.
UPDATE media
SET upload_temporary_key = 'temporary/' || id::text,
    upload_lease_token = md5(random()::text || clock_timestamp()::text || id::text),
    upload_leased_until = clock_timestamp() + INTERVAL '1 hour'
WHERE state = 'uploading';

CREATE FUNCTION mediahub_normalize_upload_lease()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.state = 'uploading' THEN
        NEW.upload_temporary_key := COALESCE(
            NEW.upload_temporary_key,
            'temporary/' || NEW.id::text
        );
        NEW.upload_lease_token := COALESCE(
            NEW.upload_lease_token,
            md5(random()::text || clock_timestamp()::text || NEW.id::text)
        );
        NEW.upload_leased_until := COALESCE(
            NEW.upload_leased_until,
            clock_timestamp() + INTERVAL '1 hour'
        );
    ELSE
        NEW.upload_temporary_key := NULL;
        NEW.upload_lease_token := NULL;
        NEW.upload_leased_until := NULL;
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER mediahub_media_upload_lease_compat
    BEFORE INSERT OR UPDATE OF state, upload_temporary_key, upload_lease_token, upload_leased_until
    ON media
    FOR EACH ROW
    EXECUTE FUNCTION mediahub_normalize_upload_lease();

ALTER TABLE media
    ADD CONSTRAINT media_upload_lease_state_check CHECK (
        (state = 'uploading'
            AND upload_temporary_key IS NOT NULL
            AND length(upload_temporary_key) > 0
            AND upload_lease_token IS NOT NULL
            AND length(upload_lease_token) BETWEEN 1 AND 255
            AND upload_leased_until IS NOT NULL)
        OR
        (state <> 'uploading'
            AND upload_temporary_key IS NULL
            AND upload_lease_token IS NULL
            AND upload_leased_until IS NULL)
    );

CREATE INDEX media_upload_reconciliation_idx
    ON media(upload_leased_until, id)
    WHERE state = 'uploading';
