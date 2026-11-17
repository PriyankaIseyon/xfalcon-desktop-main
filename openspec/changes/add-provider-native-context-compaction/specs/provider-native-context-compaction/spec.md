# Delta Spec: provider-native-context-compaction

## ADDED Requirements

### Requirement: Native Compaction Capability Is Probed Per Route Identity

The desktop SHALL probe the exact OpenRouter credential, concrete model, provider protocol version, and compaction beta version before it relies on native compaction. The probe SHALL include the current compaction edit, beta header, and required-parameter routing. A result SHALL expire after at most 24 hours and SHALL invalidate on credential, model, protocol, beta, route, or relevant provider-error change.

#### Scenario: Compatible route is cached without secrets

- **WHEN** the probe accepts the required request contract
- **THEN** the desktop MAY cache a compatible result under the non-secret route identity
- **AND** the cache SHALL contain no credential, prompt, provider block, or response content

#### Scenario: Incompatible route fails closed

- **WHEN** the route rejects, ignores, or returns an incompatible native-compaction contract
- **THEN** the run SHALL fail with `native_compaction_unsupported`
- **AND** it SHALL not use Chat Completions, direct Anthropic, generic middle-out, or a local summary

### Requirement: Every Eligible Request Enables Paused Native Compaction

Every supported agent request SHALL include one `compact_20260112` context-management edit at the 150,000-token trigger and SHALL request pause after compaction. The request SHALL carry the verified beta version. The desktop SHALL not expose these protocol values as casual user settings.

#### Scenario: Request carries the native edit

- **WHEN** a supported Sonnet or Opus turn is serialized
- **THEN** the Anthropic Messages body SHALL contain the native compaction edit, trigger, and pause request
- **AND** the request SHALL require routed parameter support

#### Scenario: Generic transform is absent

- **WHEN** the native request is built
- **THEN** no OpenRouter middle-out or generic compression transform SHALL be enabled as the durable context path

### Requirement: Compaction Block Is an Opaque Ordered Provider Event

The stream parser SHALL emit a compaction event only after it has received one complete provider compaction block. The event SHALL preserve every nested field and its position relative to other provider content blocks. Application code SHALL not extract summary text as a substitute for the block.

#### Scenario: Fragmented stream becomes one complete event

- **GIVEN** a compaction block arrives through several SSE fragments
- **WHEN** the provider signals that the block is complete
- **THEN** the adapter SHALL emit exactly one `CompactionReady` event
- **AND** no partial event SHALL reach the turn driver

#### Scenario: Unknown nested fields survive

- **GIVEN** a supported block contains a new nested field
- **WHEN** the desktop persists and later serializes it
- **THEN** the field and value SHALL remain present
- **AND** no closed summary DTO SHALL discard it

### Requirement: Durable Activation Is a Barrier Before Continuation

After a complete compaction event, the turn driver SHALL stop provider continuation and atomically activate the block through DA with the request's expected parent, history revision, snapshot head, source boundary, source digest, model, protocol, fingerprint, and usage. It SHALL resume only after DA confirms the active record.

#### Scenario: Successful activation resumes once

- **GIVEN** DA commits the compaction
- **WHEN** the driver receives the confirmed active descriptor
- **THEN** the next OpenRouter request SHALL contain that confirmed block and the remaining tail
- **AND** exactly one continuation request SHALL start under the same session ID

#### Scenario: Explicit activation failure does not continue

- **WHEN** DA rejects activation
- **THEN** no continuation request SHALL use the uncommitted block
- **AND** the prior active context SHALL remain authoritative

#### Scenario: Ambiguous activation reconciles before retry

- **GIVEN** the activation request may have committed but its response was lost
- **WHEN** retry logic runs
- **THEN** the desktop SHALL read the client-supplied compaction ID from DA first
- **AND** it SHALL retry activation only when reconciliation proves it absent

### Requirement: Stale Compaction Is Discarded and Reloaded Safely

A DA `context_conflict` SHALL cause the desktop to discard the provider result and reload authoritative context. It MAY restart the logical turn only when the existing idempotency and side-effect state proves that no duplicate user message, provider output, or tool mutation can occur.

#### Scenario: Rewind wins over stale provider work

- **GIVEN** a user rewind commits while a compaction request is in flight
- **WHEN** activation sees the changed revision
- **THEN** the provider block SHALL be discarded
- **AND** no summarized excluded message SHALL enter a later request

#### Scenario: Unsafe restart stops for reconciliation

- **GIVEN** tool side effects or a message mutation have ambiguous durable status
- **WHEN** a stale compaction would otherwise restart the turn
- **THEN** the run SHALL enter the existing reconciliation or interrupted path
- **AND** it SHALL not repeat the side effect

### Requirement: Multiple Compactions Form Ordered Lineage

One logical user turn MAY produce several compaction blocks. The desktop SHALL activate them in provider order. Each new activation SHALL name the last committed block as its expected parent. A later block SHALL never overtake an uncommitted earlier block.

#### Scenario: Two blocks commit in order

- **GIVEN** OpenRouter compacts, continues, and compacts again
- **WHEN** both activation barriers succeed
- **THEN** the second record's parent SHALL be the first record
- **AND** restore SHALL return the second record as active

#### Scenario: First activation failure prevents second request

- **WHEN** the first barrier fails
- **THEN** the driver SHALL not issue the continuation that could produce a second block

### Requirement: Compaction Usage Is Complete and Counted Once

The provider event SHALL carry input, output, cache-read, cache-write, reasoning, provider request ID, and bounded iteration usage needed to explain compaction. The desktop SHALL persist each provider request's usage exactly once and SHALL not add both an aggregate and its component iterations.

#### Scenario: Automatic compaction appears in totals

- **WHEN** a run performs a compaction-only request and a continuation request
- **THEN** the run's durable usage SHALL include both provider requests once
- **AND** user-visible totals SHALL not hide automatic compaction cost

#### Scenario: Retry cannot double-charge locally

- **GIVEN** DA already accepted usage for a provider request ID
- **WHEN** the desktop reconciles or retries
- **THEN** it SHALL reuse the prior result
- **AND** local totals SHALL not append a duplicate usage event

### Requirement: Cancellation Respects the Durable Barrier

Cancellation SHALL stop future provider requests promptly. It SHALL not abandon an ambiguous compaction activation. The run SHALL reconcile that mutation and publish one durable terminal outcome.

#### Scenario: Cancel before provider block

- **WHEN** cancellation arrives before a complete compaction block
- **THEN** the active HTTP stream SHALL be cancelled
- **AND** no DA compaction activation SHALL start

#### Scenario: Cancel during ambiguous activation

- **WHEN** cancellation arrives after the activation request was sent and before its outcome is known
- **THEN** the desktop SHALL reconcile the compaction ID
- **AND** it SHALL not report terminal cancellation until the active pointer outcome is known

### Requirement: Compaction Diagnostics Exclude Content

Diagnostics SHALL record safe block IDs, parent IDs, model, protocol, beta version, request IDs, trigger reason, before and after tokens, cache usage, latency, conflict class, retry count, and outcome. They SHALL NOT record provider block JSON, transcript text, tool bytes, previews, reasoning signatures, signed URLs, or credentials.

#### Scenario: Invalid block can be diagnosed without content

- **WHEN** provider block validation fails
- **THEN** diagnostics SHALL identify the validation class and safe request identity
- **AND** no block field value SHALL be logged
