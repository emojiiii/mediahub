-- Server webhook IDs use the stable `wh_...` public form. The initial
-- PostgreSQL profile used UUIDs here, which could not persist those IDs.
ALTER TABLE webhook_deliveries
    DROP CONSTRAINT webhook_deliveries_endpoint_id_fkey;
ALTER TABLE webhook_deliveries
    ALTER COLUMN endpoint_id TYPE TEXT USING endpoint_id::TEXT;
ALTER TABLE webhook_endpoints
    ALTER COLUMN id TYPE TEXT USING id::TEXT;
ALTER TABLE webhook_deliveries
    ADD CONSTRAINT webhook_deliveries_endpoint_id_fkey
    FOREIGN KEY (endpoint_id) REFERENCES webhook_endpoints(id) ON DELETE CASCADE;

-- SQLite uses rowid as the deterministic cursor tie-breaker. PostgreSQL owns
-- an explicit immutable identity with the same ordering contract.
ALTER TABLE webhook_deliveries
    ADD COLUMN history_id BIGINT GENERATED ALWAYS AS IDENTITY;
CREATE UNIQUE INDEX webhook_deliveries_history_id_idx
    ON webhook_deliveries(history_id);
CREATE INDEX webhook_deliveries_history_cursor_idx
    ON webhook_deliveries(endpoint_id, updated_at DESC, history_id DESC);
