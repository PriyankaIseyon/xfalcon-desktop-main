
CREATE TABLE IF NOT EXISTS project_file_backups (
    account_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    path_key BLOB NOT NULL CHECK (length(path_key) = 32),
    backup_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    compressed_bytes INTEGER NOT NULL CHECK (compressed_bytes >= 0),
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    PRIMARY KEY (account_id, backup_id)
) STRICT;

CREATE INDEX IF NOT EXISTS project_file_backups_path_history
    ON project_file_backups (account_id, project_id, path_key, created_at_ms DESC, backup_id DESC);

CREATE INDEX IF NOT EXISTS project_file_backups_retention
    ON project_file_backups (account_id, created_at_ms ASC, backup_id ASC);
