CREATE TABLE IF NOT EXISTS project_index_files (
    account_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    path_key BLOB NOT NULL CHECK (length(path_key) = 32),
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    modified_at_ms INTEGER NOT NULL CHECK (modified_at_ms >= 0),
    indexed_at_ms INTEGER NOT NULL CHECK (indexed_at_ms >= 0),
    PRIMARY KEY (account_id, project_id, path_key)
) STRICT;
