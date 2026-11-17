## RENAMED Requirements

- FROM: `### Requirement: The product offers exactly four catalog-backed models`
- TO: `### Requirement: The product offers exactly five catalog-backed models`

## MODIFIED Requirements

### Requirement: The product offers exactly five catalog-backed models

The desktop SHALL offer Haiku (`~anthropic/claude-haiku-latest`), Sonnet
(`~anthropic/claude-sonnet-latest`), Grok (`~x-ai/grok-latest`), Luna
(`~openai/gpt-luna-latest`), and Sol (`~openai/gpt-sol-latest`), in that order.

With no saved preference, the default for a new chat SHALL be the first eligible
model in that order, which is Haiku. A saved preference that is still offered
SHALL win.

It SHALL offer each model only when its exact product id is present in the
authenticated DA catalog and the row passes every check:

- active, enabled, and chat-capable;
- streaming and tool-capable;
- OpenRouter-owned;
- valid context and output limits.

Haiku SHALL run on OpenRouter Chat Completions with bounded context. Only Sonnet
SHALL use native compaction.

The retired Luna id `openai/gpt-5.6-luna` SHALL NOT be offered or chosen as a
default. A conversation already locked to it SHALL keep running on it while the
catalog serves a valid row, and the catalog view for that conversation SHALL
list it.

The renderer MUST receive redacted model metadata only. A model id MUST NOT
select a network origin or a direct vendor credential.

#### Scenario: All seeded models are available

- **GIVEN** DA returns valid rows for Haiku, Sonnet, Grok, Luna, and Sol
- **WHEN** the desktop loads model choices
- **THEN** it shows exactly those five display names, in that order
- **AND** every choice uses the OpenRouter credential path

#### Scenario: Haiku is the default

- **GIVEN** the catalog serves valid rows for Haiku and Sonnet and the person
  has no saved preference
- **WHEN** a new chat starts
- **THEN** it runs on Haiku
- **AND** with no valid Haiku row it falls through to Sonnet

#### Scenario: Haiku runs on bounded context

- **GIVEN** the catalog serves a valid Haiku row
- **WHEN** a conversation is started on Haiku
- **THEN** its execution mode is bounded context
- **AND** it does not use the Messages endpoint

#### Scenario: A pinned Haiku id is not offered

- **GIVEN** the catalog serves `anthropic/claude-haiku-4.5`
- **WHEN** the desktop loads model choices or a saved preference names it
- **THEN** it is not offered or selected
