
CREATE TABLE IF NOT EXISTS chat_title_sources (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    aad_schema_version INTEGER NOT NULL CHECK (aad_schema_version > 0),
    key_version INTEGER NOT NULL CHECK (key_version > 0),
    nonce BLOB NOT NULL CHECK (length(nonce) = 12),
    ciphertext BLOB NOT NULL,
    PRIMARY KEY (account_id, conversation_id)
) STRICT;
