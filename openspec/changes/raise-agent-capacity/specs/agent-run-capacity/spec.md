## ADDED Requirements

### Requirement: Tool rounds use the model's capacity

A Path A run SHALL allow up to 64 tool rounds and up to 8 parallel tool calls in one round. The context window and the run wall-clock SHALL remain the binding limits. The desktop MUST NOT stop a healthy run at 8 rounds while context remains.

#### Scenario: A long analysis continues past eight rounds

- **GIVEN** a model that still has context budget
- **WHEN** the run completes its eighth tool round without an answer
- **THEN** the loop continues
- **AND** it stops at 64 rounds or when the context or wall-clock budget is exhausted

### Requirement: Prose budget comes from the model's context length

The run-scoped prose byte budget SHALL be derived from the selected model's `context_length` in tokens, as stored on the tenant catalog row that OpenRouter populated. Conversion SHALL reserve one quarter of the window for completion (3 bytes per token of the remaining three quarters, which is 4 bytes/token on 75% of the window).

When the catalog is empty and the built-in OpenRouter Sonnet slug is used, the budget SHALL assume 200_000 tokens.

A catalog row whose `context_length` is missing or non-positive SHALL use the same 200_000-token fallback rather than a silent 256 KiB ceiling.

#### Scenario: A 200k-token catalog row

- **GIVEN** the selected model reports `context_length` 200000
- **WHEN** a run starts
- **THEN** the prose budget is 200000 × 0.75 × 4 bytes
- **AND** it is not the old 256 KiB constant

#### Scenario: Empty catalog

- **GIVEN** no usable catalog row
- **WHEN** a run starts on the built-in OpenRouter model
- **THEN** the prose budget is derived from 200000 tokens

### Requirement: MCP keeps the bundled server and a higher session cap

The desktop SHALL keep HTTP Streamable MCP only. It MUST NOT add stdio servers or a connector gallery in this change. Concurrent held sessions SHALL be capped at 32, counting the bundled xFalcon server.

Registrations beyond the cap SHALL still list in Settings and SHALL be recorded as a session-limit exclusion, not ignored.

#### Scenario: More than eight HTTP servers

- **GIVEN** the bundled server plus eight registered HTTP servers
- **WHEN** a run starts
- **THEN** those sessions may all be held
- **AND** the 33rd concurrent session is excluded with `SessionLimit`
