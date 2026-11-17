CREATE TABLE IF NOT EXISTS trusted_projects (
    account_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    PRIMARY KEY (account_id, project_id)
) STRICT;
