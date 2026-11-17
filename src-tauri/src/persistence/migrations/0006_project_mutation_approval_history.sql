CREATE TABLE IF NOT EXISTS project_mutation_approval_history (
    history_id INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    plan_id TEXT NOT NULL CHECK (length(plan_id) = 64),
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('denied', 'expired', 'executed', 'executed_durability_unconfirmed', 'failed')),
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    expires_at_ms INTEGER NOT NULL CHECK (expires_at_ms >= 0),
    resolved_at_ms INTEGER,
    superseded_at_ms INTEGER NOT NULL CHECK (superseded_at_ms >= 0)
) STRICT;

CREATE INDEX IF NOT EXISTS project_mutation_approval_history_account_plan
    ON project_mutation_approval_history (account_id, plan_id);

CREATE TRIGGER IF NOT EXISTS project_mutation_approvals_archive_on_reset
BEFORE UPDATE OF status ON project_mutation_approvals
WHEN OLD.status IN ('denied', 'expired', 'executed', 'executed_durability_unconfirmed', 'failed')
 AND NEW.status = 'pending'
BEGIN
    INSERT INTO project_mutation_approval_history (
        account_id, project_id, plan_id, aad_schema_version, key_version,
        nonce, ciphertext, status, created_at_ms, expires_at_ms, resolved_at_ms,
        superseded_at_ms
    ) VALUES (
        OLD.account_id, OLD.project_id, OLD.plan_id, OLD.aad_schema_version, OLD.key_version,
        OLD.nonce, OLD.ciphertext, OLD.status, OLD.created_at_ms, OLD.expires_at_ms, OLD.resolved_at_ms,
        NEW.created_at_ms
    );
END;
