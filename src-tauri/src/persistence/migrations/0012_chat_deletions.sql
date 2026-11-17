CREATE TABLE IF NOT EXISTS chat_deleted_conversations (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    PRIMARY KEY (account_id, conversation_id)
) STRICT;
