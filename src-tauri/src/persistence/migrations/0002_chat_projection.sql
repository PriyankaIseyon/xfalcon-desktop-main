CREATE TABLE IF NOT EXISTS chat_conversations (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    head_sequence INTEGER NOT NULL CHECK (head_sequence >= 0),
    content_hash TEXT NOT NULL,
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    PRIMARY KEY (account_id, conversation_id)
) STRICT;

CREATE TABLE IF NOT EXISTS chat_messages (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    content_hash TEXT NOT NULL,
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    PRIMARY KEY (account_id, conversation_id, message_id),
    UNIQUE (account_id, conversation_id, sequence)
) STRICT;

CREATE INDEX IF NOT EXISTS chat_messages_order
    ON chat_messages (account_id, conversation_id, sequence, message_id);

CREATE TABLE IF NOT EXISTS chat_drafts (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    content_hash TEXT NOT NULL,
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    PRIMARY KEY (account_id, conversation_id)
) STRICT;

CREATE TABLE IF NOT EXISTS chat_runs (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    run_id TEXT NOT NULL,
    sequence INTEGER NOT NULL CHECK (sequence > 0),
    state TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    PRIMARY KEY (account_id, conversation_id, run_id)
) STRICT;

CREATE TABLE IF NOT EXISTS chat_reconcile_temp (
    account_id TEXT NOT NULL,
    temp_id TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL,
    content_hash TEXT NOT NULL,
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    PRIMARY KEY (account_id, temp_id)
) STRICT;
