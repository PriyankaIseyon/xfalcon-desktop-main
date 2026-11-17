CREATE TABLE IF NOT EXISTS project_mutation_approvals (
    account_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    plan_id TEXT NOT NULL CHECK (length(plan_id) = 64),
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'claiming', 'denied', 'expired', 'executed', 'executed_durability_unconfirmed', 'failed')),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms >= 0),
    resolved_at_ms INTEGER,
    PRIMARY KEY (account_id, plan_id)
) STRICT;
