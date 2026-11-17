## RENAMED Requirements

- FROM: `### Requirement: The product offers exactly three catalog-backed models`
- TO: `### Requirement: The product offers exactly four catalog-backed models`

## MODIFIED Requirements

### Requirement: The product offers exactly four catalog-backed models

The desktop SHALL offer Sonnet (`~anthropic/claude-sonnet-latest`), Grok
(`~x-ai/grok-latest`), Luna (`~openai/gpt-luna-latest`), and Sol
(`~openai/gpt-sol-latest`), in that order.

It SHALL offer each model only when its exact product id is present in the
authenticated DA catalog and the row passes every check:

- active, enabled, and chat-capable;
- streaming and tool-capable;
- OpenRouter-owned;
- valid context and output limits.

The retired Luna id `openai/gpt-5.6-luna` SHALL NOT be offered or chosen as a
default. A conversation already locked to it SHALL keep running on it while the
catalog serves a valid row, and the catalog view for that conversation SHALL
list it.

The renderer MUST receive redacted model metadata only. A model id MUST NOT
select a network origin or a direct vendor credential.

#### Scenario: All seeded models are available

- **GIVEN** DA returns valid rows for Sonnet, Grok, Luna, and Sol
- **WHEN** the desktop loads model choices
- **THEN** it shows exactly those four display names
- **AND** every choice uses the OpenRouter credential path

#### Scenario: A known id has unsafe metadata

- **GIVEN** one of the product ids is disabled, inactive, lacks tools or
  streaming, belongs to another provider, or has invalid capacity
- **WHEN** the desktop loads model choices
- **THEN** that row is not offered or executed

#### Scenario: A conversation locked to the retired Luna id

- **GIVEN** a conversation stored `openai/gpt-5.6-luna` and the catalog still
  serves that row
- **WHEN** the conversation opens or sends a turn
- **THEN** it runs on `openai/gpt-5.6-luna`, and the composer names its model
- **AND** new chats are not offered that id

### Requirement: Default selection is account scoped

The desktop SHALL persist the default model separately for each signed-in
account. The local preference MUST store neither a provider secret nor a plain
account identity.

A saved preference for the retired Luna id SHALL resolve to
`~openai/gpt-luna-latest`. Any other saved model that is no longer eligible MUST
fall back to the eligible Sonnet default without making the stale id executable.

#### Scenario: The signed-in account changes

- **GIVEN** two accounts selected different default models
- **WHEN** the active account changes
- **THEN** Settings and a new-chat composer show only that account's saved default

#### Scenario: A saved model is disabled

- **GIVEN** the saved default no longer passes catalog validation
- **WHEN** the catalog loads
- **THEN** the desktop selects eligible Sonnet for future chats

#### Scenario: A saved preference names the retired Luna id

- **GIVEN** the saved default is `openai/gpt-5.6-luna` and the catalog serves
  `~openai/gpt-luna-latest`
- **WHEN** the catalog loads
- **THEN** future chats default to `~openai/gpt-luna-latest`
