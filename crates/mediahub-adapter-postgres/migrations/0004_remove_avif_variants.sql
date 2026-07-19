ALTER TABLE variants
    DROP CONSTRAINT variants_format_check;

ALTER TABLE variants
    ADD CONSTRAINT variants_format_check
    CHECK (format IN ('jpeg', 'png', 'webp')) NOT VALID;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM variants WHERE format = 'avif') THEN
        ALTER TABLE variants VALIDATE CONSTRAINT variants_format_check;
    END IF;
END
$$;
