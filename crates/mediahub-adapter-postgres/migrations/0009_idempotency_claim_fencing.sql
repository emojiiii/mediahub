-- Fence stale idempotency owners after an expired row is reclaimed. The
-- token is opaque to the application layer and is rotated on every claim.
ALTER TABLE idempotency_keys
    ADD COLUMN claim_token TEXT;

-- Keep old binaries able to insert during a rolling deployment. New binaries
-- always provide a UUID token explicitly; this default is only a compatibility
-- bridge and can be removed after all instances have been upgraded.
ALTER TABLE idempotency_keys
    ALTER COLUMN claim_token SET DEFAULT
        md5(random()::text || clock_timestamp()::text);
