CREATE TABLE system_settings (
    singleton BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    download_bytes_per_second BIGINT CHECK (
        download_bytes_per_second IS NULL
        OR download_bytes_per_second BETWEEN 1048576 AND 1073741824
    ),
    updated_by UUID REFERENCES users(id) ON DELETE SET NULL,
    updated_request_id TEXT,
    updated_at TIMESTAMPTZ NOT NULL
);

INSERT INTO system_settings (
    singleton,
    download_bytes_per_second,
    updated_by,
    updated_request_id,
    updated_at
) VALUES (
    TRUE,
    33554432,
    NULL,
    NULL,
    NOW()
);
