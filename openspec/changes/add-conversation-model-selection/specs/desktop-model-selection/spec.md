## ADDED Requirements

### Requirement: The product offers exactly three catalog-backed models

The desktop SHALL offer Sonnet, Grok, and Luna only when their exact product ids are present in the authenticated DA catalog and each row is active, enabled, chat-capable, streaming, tool-capable, OpenRouter-owned, and carries valid context and output limits.

The renderer MUST receive redacted model metadata only. A model id MUST NOT select a network origin or a direct vendor credential.

#### Scenario: All seeded models are available

- **GIVEN** DA returns valid rows for Sonnet, Grok, and Luna
- **WHEN** the desktop loads model choices
- **THEN** it shows exactly those three display names
- **AND** every choice uses the OpenRouter credential path

#### Scenario: A known id has unsafe metadata

- **GIVEN** one of the three ids is disabled, inactive, lacks tools or streaming, belongs to another provider, or has invalid capacity
- **WHEN** the desktop loads model choices
- **THEN** that row is not offered or executed

### Requirement: Model selection is immutable per conversation

The default-model setting SHALL apply only to future conversations. A new conversation MUST persist its validated model before any message or run is created. Every later send, retry, edit, and regeneration MUST resolve the model from that conversation and MUST NOT accept a replacement model from the renderer.

An existing conversation with no stored model MUST be locked to Sonnet before its next run, preserving the model and compaction behavior used before this capability.

#### Scenario: A new chat starts with Luna

- **GIVEN** Luna is the validated default
- **WHEN** the first message creates a conversation
- **THEN** the DA conversation stores Luna before the message and run rows are written

#### Scenario: The default changes after a chat starts

- **GIVEN** a conversation stored Grok
- **WHEN** the default changes to Sonnet and another turn is sent in that conversation
- **THEN** the run still uses Grok

#### Scenario: A legacy conversation has no model

- **GIVEN** a pre-migration conversation whose model is null
- **WHEN** its next run is admitted
- **THEN** the desktop persists Sonnet on that conversation and executes Sonnet

### Requirement: Each model uses its compatible context protocol

Sonnet SHALL use the OpenRouter Anthropic Messages native-compaction path. Grok and Luna SHALL use OpenRouter Chat Completions with normalized text, reasoning, tool calls, usage, cancellation, and failure events.

The non-native path MUST preserve complete admitted history and MUST fail before provider dispatch when that history exceeds catalog-derived input capacity. It MUST NOT invoke native compaction, silently elide history, or switch models.

#### Scenario: Sonnet reaches its compaction trigger

- **WHEN** a Sonnet conversation reaches the native compaction threshold
- **THEN** the existing pause, persist, activate, and resume state machine runs

#### Scenario: Luna or Grok runs

- **WHEN** a Luna or Grok conversation sends a turn within capacity
- **THEN** the fixed OpenRouter Chat Completions endpoint streams the turn with tools and events intact
- **AND** no native-compaction probe or block is used

#### Scenario: A generic conversation exceeds capacity

- **GIVEN** a Luna or Grok conversation whose complete request exceeds its safe input capacity
- **WHEN** a turn is admitted
- **THEN** the run fails clearly before a provider request
- **AND** no history is discarded and no alternate model executes

### Requirement: Default selection is account scoped

The desktop SHALL persist the default model separately for each signed-in account. The local preference MUST store neither a provider secret nor a plain account identity. A saved model that is no longer eligible MUST fall back to the eligible Sonnet default without making the stale id executable.

#### Scenario: The signed-in account changes

- **GIVEN** two accounts selected different default models
- **WHEN** the active account changes
- **THEN** Settings and a new-chat composer show only that account's saved default

#### Scenario: A saved model is disabled

- **GIVEN** the saved default no longer passes catalog validation
- **WHEN** the catalog loads
- **THEN** the desktop selects eligible Sonnet for future chats
