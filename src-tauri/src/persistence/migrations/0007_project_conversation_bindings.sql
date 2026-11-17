
CREATE TABLE IF NOT EXISTS conversation_project_bindings (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL CHECK (created_at_ms >= 0),
    PRIMARY KEY (account_id, conversation_id),
    FOREIGN KEY (account_id, project_id)
        REFERENCES trusted_projects (account_id, project_id)
        ON DELETE CASCADE
) STRICT;

CREATE INDEX IF NOT EXISTS idx_conversation_project_bindings_project
    ON conversation_project_bindings (account_id, project_id);
