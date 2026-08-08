-- S3 bucket configuration and immutable object-version foundation.
-- Existing Media tables remain temporarily available to the current APIs, but
-- all new S3 repository code writes only the structures introduced here.

ALTER TABLE buckets
    ADD COLUMN region TEXT NOT NULL DEFAULT 'us-east-1',
    ADD COLUMN versioning_status TEXT NOT NULL DEFAULT 'unversioned'
        CHECK (versioning_status IN ('unversioned', 'enabled', 'suspended')),
    ADD COLUMN object_lock_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN default_retention JSONB,
    ADD COLUMN s3_lifecycle_configuration JSONB,
    ADD COLUMN s3_configuration_revision BIGINT NOT NULL DEFAULT 1
        CHECK (s3_configuration_revision > 0),
    ADD CONSTRAINT buckets_s3_region_check CHECK (
        char_length(region) BETWEEN 1 AND 63
        AND region ~ '^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$'
    ),
    ADD CONSTRAINT buckets_s3_object_lock_versioning_check CHECK (
        NOT object_lock_enabled OR versioning_status = 'enabled'
    ),
    ADD CONSTRAINT buckets_s3_default_retention_check CHECK (
        default_retention IS NULL
        OR (object_lock_enabled AND jsonb_typeof(default_retention) = 'object')
    ),
    ADD CONSTRAINT buckets_s3_lifecycle_check CHECK (
        s3_lifecycle_configuration IS NULL
        OR jsonb_typeof(s3_lifecycle_configuration) = 'object'
    );

-- Versioning is irreversible once enabled. Object Lock is also irreversible
-- and forces Enabled on creation or first enablement.
CREATE FUNCTION enforce_s3_bucket_configuration_transition()
RETURNS TRIGGER
LANGUAGE plpgsql
AS $$
BEGIN
    IF TG_OP = 'UPDATE'
       AND OLD.versioning_status <> 'unversioned'
       AND NEW.versioning_status = 'unversioned' THEN
        RAISE EXCEPTION 'bucket versioning cannot return to unversioned'
            USING ERRCODE = '23514';
    END IF;

    IF TG_OP = 'UPDATE'
       AND OLD.versioning_status = 'unversioned'
       AND NEW.versioning_status = 'suspended' THEN
        RAISE EXCEPTION 'an unversioned bucket must be enabled before suspension'
            USING ERRCODE = '23514';
    END IF;

    IF TG_OP = 'UPDATE'
       AND OLD.object_lock_enabled
       AND NOT NEW.object_lock_enabled THEN
        RAISE EXCEPTION 'Object Lock cannot be disabled after enablement'
            USING ERRCODE = '23514';
    END IF;

    IF NEW.object_lock_enabled THEN
        NEW.versioning_status := 'enabled';
    END IF;

    RETURN NEW;
END;
$$;

CREATE TRIGGER buckets_s3_configuration_transition
    BEFORE INSERT OR UPDATE OF versioning_status, object_lock_enabled
    ON buckets
    FOR EACH ROW
    EXECUTE FUNCTION enforce_s3_bucket_configuration_transition();

CREATE TABLE objects (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL,
    bucket_id UUID NOT NULL,
    -- PostgreSQL text values cannot contain NUL; the Core model performs the
    -- same validation before persistence.
    object_key TEXT NOT NULL CHECK (octet_length(object_key) BETWEEN 1 AND 1024),
    current_version_id UUID,
    generation BIGINT NOT NULL DEFAULT 0 CHECK (generation >= 0),
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT objects_bucket_application_fkey
        FOREIGN KEY (bucket_id, application_id)
        REFERENCES buckets (id, application_id),
    CONSTRAINT objects_bucket_key_key UNIQUE (bucket_id, object_key),
    CONSTRAINT objects_identity_tenant_key UNIQUE (id, application_id, bucket_id)
);

CREATE INDEX objects_list_idx
    ON objects(application_id, bucket_id, object_key);

CREATE TABLE object_versions (
    id UUID PRIMARY KEY,
    object_id UUID NOT NULL,
    application_id UUID NOT NULL,
    bucket_id UUID NOT NULL,
    external_version_id TEXT NOT NULL CHECK (
        octet_length(external_version_id) BETWEEN 1 AND 1024
    ),
    generation BIGINT NOT NULL CHECK (generation > 0),
    is_null_version BOOLEAN NOT NULL DEFAULT FALSE,
    is_delete_marker BOOLEAN NOT NULL DEFAULT FALSE,
    state TEXT NOT NULL CHECK (state IN ('committed', 'deleting', 'failed')),

    storage_backend TEXT,
    storage_key TEXT UNIQUE,
    provider_etag TEXT,
    provider_version TEXT,
    etag TEXT,
    size_bytes BIGINT CHECK (size_bytes IS NULL OR size_bytes >= 0),
    content_type TEXT,
    user_metadata JSONB NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(user_metadata) = 'object'),
    checksum_algorithm TEXT CHECK (checksum_algorithm IN ('sha256')),
    checksum_value TEXT,

    retention_mode TEXT CHECK (retention_mode IN ('governance', 'compliance')),
    retain_until TIMESTAMPTZ,
    legal_hold BOOLEAN NOT NULL DEFAULT FALSE,

    created_by TEXT NOT NULL CHECK (created_by <> ''),
    source_protocol TEXT NOT NULL
        CHECK (source_protocol IN ('s3', 'dav', 'json', 'processor')),
    created_at TIMESTAMPTZ NOT NULL,
    became_noncurrent_at TIMESTAMPTZ,
    superseded_at TIMESTAMPTZ,

    CONSTRAINT object_versions_object_tenant_fkey
        FOREIGN KEY (object_id, application_id, bucket_id)
        REFERENCES objects (id, application_id, bucket_id)
        ON DELETE CASCADE,
    CONSTRAINT object_versions_generation_key UNIQUE (object_id, generation),
    CONSTRAINT object_versions_identity_object_key UNIQUE (id, object_id),
    CONSTRAINT object_versions_null_version_check CHECK (
        is_null_version = (external_version_id = 'null')
    ),
    CONSTRAINT object_versions_payload_check CHECK (
        (
            is_delete_marker
            AND storage_backend IS NULL
            AND storage_key IS NULL
            AND etag IS NULL
            AND size_bytes IS NULL
            AND retention_mode IS NULL
            AND retain_until IS NULL
            AND NOT legal_hold
        )
        OR
        (
            NOT is_delete_marker
            AND storage_backend IS NOT NULL
            AND storage_key IS NOT NULL
            AND etag IS NOT NULL
            AND size_bytes IS NOT NULL
        )
    ),
    CONSTRAINT object_versions_etag_check CHECK (
        etag IS NULL OR (
            octet_length(etag) BETWEEN 1 AND 1024
            AND position(chr(34) IN etag) = 0
        )
    ),
    CONSTRAINT object_versions_checksum_pair_check CHECK (
        (checksum_algorithm IS NULL) = (checksum_value IS NULL)
        AND (
            checksum_algorithm IS NULL
            OR (
                checksum_algorithm = 'sha256'
                AND length(checksum_value) = 64
                AND checksum_value ~ '^[0-9A-Fa-f]{64}$'
            )
        )
    ),
    CONSTRAINT object_versions_retention_pair_check CHECK (
        (retention_mode IS NULL) = (retain_until IS NULL)
    ),
    CONSTRAINT object_versions_noncurrent_time_check CHECK (
        became_noncurrent_at IS NULL OR became_noncurrent_at >= created_at
    ),
    CONSTRAINT object_versions_superseded_time_check CHECK (
        superseded_at IS NULL
        OR (is_null_version AND superseded_at >= created_at)
    )
);

-- Replaced null versions remain immutable audit rows. Only the active row may
-- own the externally visible literal version ID "null".
CREATE UNIQUE INDEX object_versions_active_external_version_key
    ON object_versions(object_id, external_version_id)
    WHERE superseded_at IS NULL;

-- The one nullable current pointer is the only persisted latest marker. The
-- composite FK prevents a pointer from referencing another logical object.
ALTER TABLE objects
    ADD CONSTRAINT objects_current_version_fkey
        FOREIGN KEY (current_version_id, id)
        REFERENCES object_versions (id, object_id)
        DEFERRABLE INITIALLY IMMEDIATE;

CREATE INDEX object_versions_history_idx
    ON object_versions(object_id, generation DESC)
    WHERE superseded_at IS NULL;

CREATE INDEX object_versions_bucket_version_idx
    ON object_versions(application_id, bucket_id, external_version_id)
    WHERE superseded_at IS NULL;

CREATE INDEX object_versions_noncurrent_idx
    ON object_versions(bucket_id, became_noncurrent_at, id)
    WHERE became_noncurrent_at IS NOT NULL
      AND superseded_at IS NULL;
-- Upload intents own staged bytes. They are intentionally separate from the
-- immutable version history and never reference Media.
CREATE TABLE s3_upload_intents (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL,
    bucket_id UUID NOT NULL,
    -- PostgreSQL rejects NUL before a text value reaches this constraint.
    object_key TEXT NOT NULL CHECK (octet_length(object_key) BETWEEN 1 AND 1024),
    proposed_version_id UUID NOT NULL UNIQUE,
    state TEXT NOT NULL CONSTRAINT s3_upload_intents_state_value_check CHECK (
        state IN ('staging', 'ready', 'committing', 'committed', 'aborted', 'expired')
    ),
    storage_backend TEXT NOT NULL CHECK (
        octet_length(storage_backend) BETWEEN 1 AND 255
    ),
    temporary_storage_key TEXT NOT NULL CHECK (
        octet_length(temporary_storage_key) BETWEEN 1 AND 2048
    ),
    final_storage_key TEXT NOT NULL CHECK (
        octet_length(final_storage_key) BETWEEN 1 AND 2048
    ),
    entity_tag TEXT CHECK (
        entity_tag IS NULL OR (
            octet_length(entity_tag) BETWEEN 1 AND 1024
            AND position(chr(34) IN entity_tag) = 0
        )
    ),
    checksum_algorithm TEXT CHECK (checksum_algorithm IN ('sha256')),
    checksum_value TEXT CHECK (
        checksum_value IS NULL OR (
            length(checksum_value) = 64
            AND checksum_value ~ '^[0-9A-Fa-f]{64}$'
        )
    ),
    expected_size_bytes BIGINT NOT NULL CHECK (expected_size_bytes >= 0),
    size_bytes BIGINT CHECK (size_bytes IS NULL OR size_bytes >= 0),
    content_type TEXT,
    user_metadata JSONB NOT NULL DEFAULT '{}'::jsonb
        CHECK (jsonb_typeof(user_metadata) = 'object'),
    lease_token TEXT,
    lease_until TIMESTAMPTZ,
    committed_object_id UUID,
    committed_version_id UUID,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT s3_upload_intents_bucket_application_fkey
        FOREIGN KEY (bucket_id, application_id)
        REFERENCES buckets (id, application_id),
    CONSTRAINT s3_upload_intents_committed_version_fkey
        FOREIGN KEY (committed_version_id, committed_object_id)
        REFERENCES object_versions (id, object_id)
        DEFERRABLE INITIALLY IMMEDIATE,
    CONSTRAINT s3_upload_intents_expiry_check CHECK (expires_at > created_at),
    CONSTRAINT s3_upload_intents_storage_fence_check CHECK (
        temporary_storage_key <> final_storage_key
    ),
    CONSTRAINT s3_upload_intents_facts_check CHECK (
        (entity_tag IS NULL) = (checksum_algorithm IS NULL)
        AND (entity_tag IS NULL) = (checksum_value IS NULL)
        AND (entity_tag IS NULL) = (size_bytes IS NULL)
        AND (size_bytes IS NULL OR size_bytes = expected_size_bytes)
    ),
    CONSTRAINT s3_upload_intents_state_check CHECK (
        (
            state = 'staging'
            AND entity_tag IS NULL
            AND lease_token IS NULL AND lease_until IS NULL
            AND committed_object_id IS NULL AND committed_version_id IS NULL
        ) OR (
            state = 'ready'
            AND entity_tag IS NOT NULL
            AND lease_token IS NULL AND lease_until IS NULL
            AND committed_object_id IS NULL AND committed_version_id IS NULL
        ) OR (
            state = 'committing'
            AND entity_tag IS NOT NULL
            AND lease_token IS NOT NULL AND lease_until IS NOT NULL
            AND committed_object_id IS NULL AND committed_version_id IS NULL
        ) OR (
            state = 'committed'
            AND entity_tag IS NOT NULL
            AND lease_token IS NULL AND lease_until IS NULL
            AND committed_object_id IS NOT NULL AND committed_version_id IS NOT NULL
        ) OR (
            state IN ('aborted', 'expired')
            AND lease_token IS NULL AND lease_until IS NULL
            AND committed_object_id IS NULL AND committed_version_id IS NULL
        )
    )
);
CREATE INDEX s3_upload_intents_target_idx
    ON s3_upload_intents(application_id, bucket_id, object_key, created_at DESC);
CREATE INDEX s3_upload_intents_expiry_idx
    ON s3_upload_intents(expires_at, id)
    WHERE state IN ('staging', 'ready');
CREATE INDEX s3_upload_intents_lease_idx
    ON s3_upload_intents(lease_until, id)
    WHERE state = 'committing';

-- Multipart completion is fenced by a durable UploadIntent and commits directly
-- into immutable object history. The initial 0006 schema never references Media.
ALTER TABLE s3_multipart_uploads
    DROP CONSTRAINT s3_multipart_completion_state_check,
    ADD COLUMN upload_intent_id UUID,
    ADD COLUMN object_id UUID,
    ADD COLUMN object_version_id UUID,
    ADD COLUMN final_checksum_algorithm TEXT
        CHECK (final_checksum_algorithm IN ('sha256')),
    ADD COLUMN final_checksum_value TEXT,
    ADD CONSTRAINT s3_multipart_upload_intent_fkey
        FOREIGN KEY (upload_intent_id)
        REFERENCES s3_upload_intents(id),
    ADD CONSTRAINT s3_multipart_upload_intent_key UNIQUE (upload_intent_id),
    ADD CONSTRAINT s3_multipart_completed_version_fkey
        FOREIGN KEY (object_version_id, object_id)
        REFERENCES object_versions (id, object_id)
        DEFERRABLE INITIALLY IMMEDIATE,
    ADD CONSTRAINT s3_multipart_object_version_key UNIQUE (object_version_id),
    ADD CONSTRAINT s3_multipart_final_checksum_check CHECK (
        (final_checksum_algorithm IS NULL) = (final_checksum_value IS NULL)
        AND (
            final_checksum_algorithm IS NULL
            OR (
                final_checksum_algorithm = 'sha256'
                AND length(final_checksum_value) = 64
                AND final_checksum_value ~ '^[0-9A-Fa-f]{64}$'
            )
        )
    ),
    ADD CONSTRAINT s3_multipart_version_completion_check CHECK (
        (
            state = 'completed'
            AND upload_intent_id IS NOT NULL
            AND object_id IS NOT NULL
            AND object_version_id IS NOT NULL
            AND final_etag IS NOT NULL
            AND final_checksum_algorithm = 'sha256'
            AND final_checksum_value IS NOT NULL
            AND completed_at IS NOT NULL
        ) OR (
            state <> 'completed'
            AND object_id IS NULL
            AND object_version_id IS NULL
            AND final_etag IS NULL
            AND final_checksum_algorithm IS NULL
            AND final_checksum_value IS NULL
            AND completed_at IS NULL
        )
    );

CREATE INDEX s3_multipart_upload_intent_idx
    ON s3_multipart_uploads(upload_intent_id)
    WHERE upload_intent_id IS NOT NULL;
CREATE INDEX s3_multipart_object_version_idx
    ON s3_multipart_uploads(object_version_id)
    WHERE object_version_id IS NOT NULL;

-- Storage deletion is an at-least-once persistent workflow. Retention checks
-- occur before enqueue; workers lease rows and may retry without losing intent.
CREATE TABLE storage_gc_tasks (
    id UUID PRIMARY KEY,
    application_id UUID NOT NULL REFERENCES applications(id),
    bucket_id UUID NOT NULL,
    object_version_id UUID REFERENCES object_versions(id) ON DELETE SET NULL,
    upload_intent_id UUID REFERENCES s3_upload_intents(id) ON DELETE SET NULL,
    multipart_upload_id TEXT
        REFERENCES s3_multipart_uploads(upload_id) ON DELETE SET NULL,
    storage_backend TEXT NOT NULL CHECK (
        octet_length(storage_backend) BETWEEN 1 AND 255
    ),
    storage_key TEXT NOT NULL CHECK (
        octet_length(storage_key) BETWEEN 1 AND 2048
    ),
    reason TEXT NOT NULL CHECK (
        reason IN (
            'aborted_upload_intent', 'multipart_temporary',
            'replaced_null_version', 'lifecycle_expiration', 'explicit_delete'
        )
    ),
    state TEXT NOT NULL DEFAULT 'pending'
        CONSTRAINT storage_gc_tasks_state_value_check CHECK (
        state IN ('pending', 'leased', 'completed', 'dead_letter')
    ),
    not_before TIMESTAMPTZ NOT NULL,
    lease_token TEXT,
    lease_until TIMESTAMPTZ,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0),
    last_error TEXT CHECK (last_error IS NULL OR octet_length(last_error) <= 4096),
    completed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT storage_gc_tasks_backend_key_key UNIQUE (storage_backend, storage_key),
    CONSTRAINT storage_gc_tasks_bucket_application_fkey
        FOREIGN KEY (bucket_id, application_id)
        REFERENCES buckets (id, application_id),
    CONSTRAINT storage_gc_tasks_state_check CHECK (
        (
            state = 'pending'
            AND lease_token IS NULL AND lease_until IS NULL AND completed_at IS NULL
        ) OR (
            state = 'leased'
            AND lease_token IS NOT NULL AND lease_until IS NOT NULL AND completed_at IS NULL
        ) OR (
            state = 'completed'
            AND lease_token IS NULL AND lease_until IS NULL AND completed_at IS NOT NULL
        ) OR (
            state = 'dead_letter'
            AND lease_token IS NULL AND lease_until IS NULL AND completed_at IS NULL
        )
    )
);

CREATE INDEX storage_gc_tasks_claim_idx
    ON storage_gc_tasks(not_before, id)
    WHERE state = 'pending';
CREATE INDEX storage_gc_tasks_lease_idx
    ON storage_gc_tasks(lease_until, id)
    WHERE state = 'leased';
CREATE INDEX storage_gc_tasks_version_idx
    ON storage_gc_tasks(object_version_id)
    WHERE object_version_id IS NOT NULL;
CREATE INDEX storage_gc_tasks_multipart_idx
    ON storage_gc_tasks(multipart_upload_id)
    WHERE multipart_upload_id IS NOT NULL AND state <> 'completed';
