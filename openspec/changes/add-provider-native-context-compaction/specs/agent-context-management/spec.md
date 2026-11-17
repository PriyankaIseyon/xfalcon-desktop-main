# Delta Spec: agent-context-management

## ADDED Requirements

### Requirement: Primary Agent Uses the Supported Million-Token Contract

Primary agent execution SHALL require an OpenRouter credential and an enabled Anthropic Claude Sonnet or Opus catalog model whose `context_length` is at least 1,000,000 and whose `max_output_tokens` is positive and less than its context length. The desktop SHALL reject missing or inconsistent capacity data. It SHALL NOT guess a smaller fallback or route the turn to another provider or model.

#### Scenario: Valid Sonnet or Opus starts

- **GIVEN** an OpenRouter credential and an enabled supported model with valid total and output capacities
- **WHEN** a user submits an agent turn
- **THEN** the run SHALL use that exact OpenRouter model slug
- **AND** context admission SHALL use the catalog capacities

#### Scenario: Invalid catalog capacity fails before provider use

- **GIVEN** a selected model with missing, non-positive, or inconsistent capacity
- **WHEN** a run starts
- **THEN** the desktop SHALL return a typed model-capacity error
- **AND** it SHALL not use the old 200,000-token or fixed-byte fallback

#### Scenario: Unsupported provider is not an implicit fallback

- **GIVEN** the primary agent path has no usable OpenRouter credential
- **WHEN** a turn starts
- **THEN** it SHALL fail with a clear setup error
- **AND** it SHALL not call Anthropic, OpenAI, or Google directly

### Requirement: Inference History Comes from a Complete DA Context Snapshot

Every provider run for a durable conversation SHALL rebuild history from the DA context endpoint. The desktop SHALL follow all cursors, validate frozen snapshot identity and ordering, and include every active post-boundary message. It SHALL NOT use conversation bootstrap, a one-page message list, or the local stale projection as the authoritative inference history.

#### Scenario: More than 200 messages restore completely

- **GIVEN** the active post-boundary transcript contains more than 200 messages
- **WHEN** the desktop loads context
- **THEN** it SHALL follow every DA continuation cursor through `has_more=false`
- **AND** each message through the frozen head SHALL appear exactly once in sequence order

#### Scenario: Stale page restarts the snapshot

- **GIVEN** rewind, truncation, or pointer replacement makes a later cursor stale
- **WHEN** DA returns `context_snapshot_stale`
- **THEN** the desktop SHALL discard every page from that snapshot and restart from page one
- **AND** repeated conflicts SHALL stop after a bounded retry count with a typed conflict

#### Scenario: Incomplete snapshot fails closed

- **WHEN** a page has a duplicate, gap, wrong conversation, wrong active compaction, changed head, invalid payload descriptor, or unmatched tool exchange
- **THEN** the desktop SHALL fail context loading before any provider request
- **AND** it SHALL not continue with the rows that happened to validate

### Requirement: Active Compaction Must Match the Current Context Fingerprint

The desktop SHALL compute the DA-defined versioned context fingerprint from every model-visible configuration input. It SHALL accept an active compaction only when provider protocol, block format, model, fingerprint, lineage, and digest match the current run.

#### Scenario: Skill or tool change invalidates old compaction

- **GIVEN** an active compaction and a changed enabled skill, tool schema, system prompt, agent prompt, reasoning setting, or included document descriptor
- **WHEN** a new run assembles its manifest
- **THEN** the new fingerprint SHALL differ
- **AND** the desktop SHALL not send the stale compaction block to OpenRouter

#### Scenario: Same canonical inputs produce the same fingerprint

- **GIVEN** model-visible inputs with map fields or source enumeration in different incidental order
- **WHEN** the canonical manifest is built
- **THEN** its bytes and SHA-256 SHALL match the pinned test vector
- **AND** meaningful ordered inputs such as tool order SHALL remain order-sensitive

### Requirement: Hard Context Admission Uses Layered Token Safety

The desktop SHALL calculate the hard input limit as `context_length - max_output_tokens - 16,384`. It SHALL use exact provider input usage as the last-request anchor and a worst-case one-token-per-UTF-8-byte estimate plus tested protocol overhead for unmeasured additions. It SHALL remove the fixed 75% byte conversion and fixed 512-message ceiling.

#### Scenario: One-million-token model has explicit reserves

- **GIVEN** a model with 1,000,000 total tokens and catalog output limit `O`
- **WHEN** the run computes its hard input limit
- **THEN** the limit SHALL equal `1,000,000 - O - 16,384`
- **AND** no four-bytes-per-token conversion SHALL replace that calculation

#### Scenario: Message count alone does not evict context

- **GIVEN** more than 512 small messages that remain inside the hard input limit and preserve complete groups
- **WHEN** context assembles
- **THEN** the desktop SHALL retain them
- **AND** it SHALL not stop or evict only because the count reached 512

#### Scenario: Oversized single item fails safely

- **GIVEN** one indivisible content item exceeds the hard admission limit
- **WHEN** no legal provider compaction request can contain it
- **THEN** the desktop SHALL return `context_item_too_large` with safe type and size metadata
- **AND** the error SHALL include no item content

### Requirement: Provider Native Trigger Is the Normal Quality Boundary

The context manager SHALL rely on the provider's 150,000-input-token native compaction trigger for normal long-run quality control. The 1M hard limit SHALL remain a safety limit, not a target. A projected hard-limit breach SHALL cause a compaction-only request over the largest safe closed prefix before the user-answer request continues.

#### Scenario: Long healthy run compacts before hard pressure

- **GIVEN** provider-counted input crosses the native trigger
- **WHEN** OpenRouter processes the request
- **THEN** the compaction state machine SHALL handle the provider block
- **AND** the run SHALL not wait for the 1M hard limit before reducing context

#### Scenario: Large new result forces preflight compaction

- **GIVEN** a new preview or history segment would push the next request above the hard limit
- **WHEN** an older closed prefix fits a legal compaction request
- **THEN** the desktop SHALL compact and activate that prefix first
- **AND** it SHALL then rebuild the continuation from DA

### Requirement: Oversized Legacy History Compacts Progressively

A conversation with no usable active compaction whose complete history exceeds one safe request SHALL use progressive bootstrap compaction over the oldest complete prefixes. Each step SHALL be durable before the next begins. The operation SHALL not execute historical tool calls or answer the current user turn until all required history can be represented safely.

#### Scenario: Legacy history folds in several resumable steps

- **GIVEN** three safe closed prefixes are required to cover retained history
- **WHEN** bootstrap compaction runs
- **THEN** it SHALL request, activate, reload, and advance in that order for each prefix
- **AND** a crash after step two SHALL resume from the second committed compaction

#### Scenario: User cancels bootstrap

- **WHEN** the user cancels between compaction requests
- **THEN** no new provider request SHALL start
- **AND** the latest committed DA compaction SHALL remain valid for a later retry
- **AND** no historical tool call SHALL have run

#### Scenario: Saved tool rounds include signed replay prefixes

- **GIVEN** replay-only reminders or recursive assistant messages precede saved tool turns
- **WHEN** the hard guard selects an oldest complete prefix for native compaction
- **THEN** it SHALL preserve the exact signed wire prefix through a complete durable boundary
- **AND** the source count and digest SHALL include only the durable message identities
- **AND** it SHALL exclude replay owned by a later saved turn and the true unsaved tail

### Requirement: Context Groups Remain Provider-Valid

Context selection, bootstrap compaction, native activation, emergency elision, retry, and restore SHALL preserve complete assistant tool-use and tool-result groups and ordered opaque reasoning blocks whose provider prefix remains valid. No operation SHALL leave an unanswered historical call unless the existing typed failure backfill represents the answer.

#### Scenario: Parallel tool calls stay together

- **GIVEN** one assistant message requests several tool calls
- **WHEN** any context boundary is selected
- **THEN** the assistant message and every matching tool result SHALL remain on the same retained or covered side
- **AND** result order SHALL remain provider-valid

#### Scenario: Opaque reasoning remains ordered in a live tool loop

- **GIVEN** OpenRouter returns thinking blocks with signatures before tool-use blocks
- **WHEN** the desktop sends the tool results without replacing that thinking's prompt prefix
- **THEN** it SHALL return those complete provider blocks in their original order
- **AND** it SHALL not rebuild or omit a signature

#### Scenario: A native compaction invalidates older prefix-bound thinking

- **GIVEN** the selected Sonnet model requires an unchanged prompt prefix for signed thinking
- **WHEN** a new native compaction replaces that prefix
- **THEN** the Messages request SHALL omit thinking produced before that compaction
- **AND** tool-use, tool-result, text, and the exact opaque compaction block SHALL remain intact
- **AND** fresh thinking produced after the compaction SHALL remain byte-exact
- **AND** the provider-neutral context and durable history SHALL remain unmodified

#### Scenario: A tool-round reminder remains in its signed prompt prefix

- **GIVEN** a temporary run-status reminder preceded prefix-bound signed thinking
- **WHEN** the desktop reloads the durable history for another tool round
- **THEN** it SHALL restore that exact reminder before its associated assistant turn
- **AND** the current run-status reminder SHALL remain last
- **AND** a compaction that invalidates the older thinking SHALL also retire its reminder

#### Scenario: Hard-guard compaction occurs during an unfinished answer

- **GIVEN** the live context ends with unsaved assistant prose from a continued answer
- **WHEN** hard-guard bootstrap compacts only durable history and reloads the snapshot
- **THEN** the next request SHALL retain that unsummarized prose once, in its original order
- **AND** it SHALL omit that prose's obsolete thinking and run-status reminders
- **AND** it SHALL not restore earlier prose already covered by a provider-threshold compaction
- **AND** fresh thinking SHALL retain the complete request prefix that produced it

### Requirement: Authoritative Context and Local Projection Have Separate Roles

The encrypted local projection MAY render stale history offline. It SHALL NOT authorize a provider run, compaction activation, or payload retrieval without an authoritative DA context snapshot. Local state needed for an in-flight durable mutation SHALL be bounded, account-scoped, encrypted or protected, and reconciled before reuse.

#### Scenario: Offline chat remains readable but cannot run

- **GIVEN** a sealed local conversation and no authoritative DA connection
- **WHEN** the user opens it
- **THEN** the UI MAY show its stale projection
- **AND** a new provider turn SHALL remain unavailable with a clear sync reason

#### Scenario: Account change purges pending context state

- **WHEN** the active account changes or local data is cleared
- **THEN** pending compaction and payload temporary data for the prior account SHALL be removed or reported as an explicit cleanup failure
- **AND** no prior-account payload ID SHALL remain admitted

### Requirement: Context Failures Restore the Full Optimistic State

Any context load, compaction, payload persistence, provider continuation, or retry failure after an optimistic store update SHALL restore the complete pre-operation snapshot, including messages, activity, reasoning, round prose, active run ID, run state, and admitted invocations.

#### Scenario: DA conflict restores exact state

- **GIVEN** the UI adopted a run and displayed in-progress context work
- **WHEN** compaction activation returns a conflict that cannot be retried safely
- **THEN** every captured store field SHALL equal its pre-operation value
- **AND** the durable original run SHALL remain retryable or terminal according to DA truth
