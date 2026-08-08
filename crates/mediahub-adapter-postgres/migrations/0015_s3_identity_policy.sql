-- Identity policies are attached to one concrete S3 access key. The access
-- key's existing application foreign key is the tenant ownership boundary;
-- repository mutations additionally fence application_id + access_key_id.

ALTER TABLE access_keys
    ADD COLUMN s3_identity_policy JSONB,
    ADD COLUMN s3_identity_policy_sha256 TEXT,
    ADD COLUMN s3_identity_policy_revision BIGINT NOT NULL DEFAULT 0,
    ADD COLUMN s3_identity_policy_updated_at TIMESTAMPTZ,
    ADD CONSTRAINT access_keys_s3_identity_policy_document_check CHECK (
        s3_identity_policy IS NULL
        OR jsonb_typeof(s3_identity_policy) = 'object'
    ),
    ADD CONSTRAINT access_keys_s3_identity_policy_sha256_check CHECK (
        s3_identity_policy_sha256 IS NULL
        OR s3_identity_policy_sha256 ~ '^[0-9a-f]{64}$'
    ),
    ADD CONSTRAINT access_keys_s3_identity_policy_pair_check CHECK (
        (s3_identity_policy IS NULL) = (s3_identity_policy_sha256 IS NULL)
    ),
    ADD CONSTRAINT access_keys_s3_identity_policy_revision_check CHECK (
        s3_identity_policy_revision >= 0
        AND (
            (
                s3_identity_policy_revision = 0
                AND s3_identity_policy IS NULL
                AND s3_identity_policy_updated_at IS NULL
            )
            OR (
                s3_identity_policy_revision > 0
                AND s3_identity_policy_updated_at IS NOT NULL
            )
        )
    );
