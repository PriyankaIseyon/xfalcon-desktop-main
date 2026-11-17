CREATE TABLE IF NOT EXISTS journal_events (
    account_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    event_type TEXT NOT NULL,
    state TEXT NOT NULL,
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    created_at_ms INTEGER NOT NULL,
    PRIMARY KEY (account_id, event_id),
    UNIQUE (account_id, run_id, sequence)
) STRICT;

CREATE INDEX IF NOT EXISTS journal_events_replay
    ON journal_events (account_id, run_id, sequence, event_id);
