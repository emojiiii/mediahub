-- Stable S3 account identities, globally addressed bucket names, and the
-- persistence state required by Bucket Policy authorization.

ALTER TABLE applications
    ADD COLUMN s3_account_id BIGINT GENERATED ALWAYS AS IDENTITY (
        MINVALUE 100000000000
        MAXVALUE 999999999999
        START WITH 100000000000
        NO CYCLE
    ),
    ADD CONSTRAINT applications_s3_account_id_key UNIQUE (s3_account_id),
    ADD CONSTRAINT applications_s3_account_id_check CHECK (
        s3_account_id BETWEEN 100000000000 AND 999999999999
    );

-- Application names and public app IDs may change. The S3 account identity is
-- a durable authorization identity and therefore must never be rewritten.
CREATE FUNCTION enforce_s3_account_id_immutable()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF NEW.s3_account_id IS DISTINCT FROM OLD.s3_account_id THEN
        RAISE EXCEPTION 'S3 account identity is immutable'
            USING ERRCODE = '23514';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER applications_s3_account_id_immutable
    BEFORE UPDATE OF s3_account_id
    ON applications
    FOR EACH ROW
    EXECUTE FUNCTION enforce_s3_account_id_immutable();

-- S3 bucket names occupy one global namespace. There are no production users,
-- so conflicting legacy rows are intentionally rejected by this migration.
ALTER TABLE buckets
    DROP CONSTRAINT buckets_application_id_name_key,
    ADD CONSTRAINT buckets_name_key UNIQUE (name),
    ADD COLUMN s3_bucket_policy JSONB,
    ADD COLUMN s3_bucket_policy_sha256 TEXT,
    ADD COLUMN s3_bucket_policy_revision BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN s3_bucket_policy_updated_at TIMESTAMPTZ,
    ADD CONSTRAINT buckets_s3_bucket_policy_document_check CHECK (
        s3_bucket_policy IS NULL
        OR jsonb_typeof(s3_bucket_policy) = 'object'
    ),
    ADD CONSTRAINT buckets_s3_bucket_policy_sha256_check CHECK (
        s3_bucket_policy_sha256 IS NULL
        OR s3_bucket_policy_sha256 ~ '^[0-9a-f]{64}$'
    ),
    ADD CONSTRAINT buckets_s3_bucket_policy_pair_check CHECK (
        (s3_bucket_policy IS NULL) = (s3_bucket_policy_sha256 IS NULL)
    ),
    ADD CONSTRAINT buckets_s3_bucket_policy_revision_check CHECK (
        s3_bucket_policy_revision >= 0
        AND (
            (
                s3_bucket_policy_revision = 0
                AND s3_bucket_policy IS NULL
                AND s3_bucket_policy_updated_at IS NULL
            )
            OR (s3_bucket_policy_revision > 0 AND s3_bucket_policy_updated_at IS NOT NULL)
        )
    );
