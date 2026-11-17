# Delta Spec: tool-result-context

## ADDED Requirements

### Requirement: Successful Tool Results Have Exact Durable Payloads

A successful tool result whose exact serialized bytes exceed the bounded model preview SHALL be prepared, uploaded when required, finalized, and atomically bound through the DA tool-result payload contract before the next provider request. The desktop SHALL preserve exact bytes up to the existing 8 MiB MCP response boundary. It SHALL NOT treat a truncated preview as the durable full result.

#### Scenario: Large MCP result persists before continuation

- **GIVEN** an MCP call returns a valid 2 MiB JSON result
- **WHEN** the tool round prepares its provider continuation
- **THEN** the exact 2 MiB bytes SHALL be ready and bound in DA
- **AND** the model message SHALL contain a bounded preview and payload descriptor

#### Scenario: Durable storage failure stops the round

- **GIVEN** a result needs DA object storage
- **WHEN** prepare, upload, finalize, or bind fails
- **THEN** the desktop SHALL not send the next provider request as if the result were complete
- **AND** the run SHALL report a typed retryable or terminal persistence failure

#### Scenario: Result over 8 MiB fails at the trust boundary

- **WHEN** a tool response exceeds 8 MiB
- **THEN** the invocation SHALL fail with an explicit size error
- **AND** neither a ready payload nor a misleading success preview SHALL be created

### Requirement: Fixed Four-Kilobyte Context Truncation Is Removed

The desktop SHALL remove `MAX_TOOL_RESULT_CONTEXT_BYTES = 4_096` and every equivalent first-prefix truncation in live and restored context. Tool result admission SHALL use the typed preview and model-capacity contracts. No sibling tool-result construction path SHALL retain the old limit.

#### Scenario: Result larger than 4 KiB remains useful

- **GIVEN** a valid 40 KiB result
- **WHEN** it enters model context
- **THEN** its type-aware preview MAY contain the complete result within the 64 KiB preview bound
- **AND** it SHALL not be cut at 4 KiB

#### Scenario: History rebuild matches live construction

- **GIVEN** the same durable tool result is used live and after restart
- **WHEN** both paths build provider context
- **THEN** they SHALL create equivalent preview and payload descriptors
- **AND** neither path SHALL apply a hidden smaller cap

### Requirement: Model Preview Is Typed, Bounded, and Honest

The desktop SHALL build `ToolResultPreviewV1` at no more than 64 KiB. It SHALL include payload ID, exact length, SHA-256, media and format version, preview strategy, and `complete`. Text SHALL use head and tail with omitted-byte count. Tables and CSV SHALL include columns, row count when known, head and tail rows, and omitted-row count. JSON SHALL preserve a bounded structural projection. Binary SHALL include metadata only.

#### Scenario: Table preview shows shape and omission

- **GIVEN** a 100,000-row table payload
- **WHEN** its preview is built
- **THEN** the model SHALL receive column identity, row count when known, selected head and tail rows, payload reference, and omitted-row count
- **AND** `complete` SHALL be false

#### Scenario: Complete small result says complete

- **GIVEN** exact valid text whose complete preview fits the bound
- **WHEN** the preview is built
- **THEN** `complete` SHALL be true
- **AND** the preview bytes SHALL equal the exact text representation

#### Scenario: UTF-8 and JSON boundaries remain valid

- **WHEN** a preview bound falls inside a multibyte character or JSON value
- **THEN** the preview SHALL remain valid for its declared format
- **AND** it SHALL not cut raw bytes into malformed model-visible syntax

### Requirement: Model Can Read Admitted Exact Ranges

The desktop SHALL provide `__internal__read_tool_result` when the active context contains a ready payload descriptor. The tool SHALL accept that admitted payload ID, an offset, and a length of at most 64 KiB. It SHALL return exact range bytes with total length, bounds, media type, digest, and continuation state.

#### Scenario: Model reads a later range

- **GIVEN** a ready payload admitted in the current conversation and run
- **WHEN** the model requests a legal later range
- **THEN** the desktop SHALL retrieve and validate that range through DA
- **AND** the tool result SHALL state the exact returned bounds and whether bytes remain

#### Scenario: Unadmitted payload is denied

- **GIVEN** a valid payload ID from another conversation, prior run, or non-active context
- **WHEN** the model requests it
- **THEN** the internal tool SHALL deny the call before content retrieval
- **AND** it SHALL reveal no owner, object key, URL, digest, or size

#### Scenario: Range response fails digest identity

- **GIVEN** DA metadata digest differs from the admitted descriptor or reconstructed full bytes fail digest verification
- **WHEN** a range is read
- **THEN** the invocation SHALL fail with a typed integrity error
- **AND** the suspect bytes SHALL not enter model context

### Requirement: Exact-Result Tool Is Read-Only and Policy-Safe

The internal range-read tool SHALL be fixed, read-only, and approval-free. Its response content SHALL remain untrusted data. It SHALL not grant capabilities, change system instructions, approve a tool, modify an allowlist, or admit another payload ID.

#### Scenario: Payload contains prompt injection

- **GIVEN** an exact result range contains instructions that claim system authority
- **WHEN** the range reaches model context
- **THEN** the system contract SHALL label it as untrusted tool data
- **AND** application policy and approval state SHALL remain unchanged

#### Scenario: Tool exposes no transport secret

- **WHEN** the range tool succeeds
- **THEN** its result SHALL contain no signed URL, storage key, tenant ID, host path, bearer, or provider credential

### Requirement: Tool Payload Persistence Is Idempotent and Reconciled

The desktop SHALL use stable client payload IDs and mutation idempotency keys. After an ambiguous prepare, finalize, bind, or message-write outcome, it SHALL read DA resource state before retry. It SHALL not upload, bind, or append a second logical result under a new identity merely because a response was lost.

#### Scenario: Finalize response is lost

- **GIVEN** DA may have finalized payload `P`
- **WHEN** the network fails before the response arrives
- **THEN** the desktop SHALL retrieve metadata for `P`
- **AND** it SHALL reuse ready `P` or retry the same ID according to its observed state

#### Scenario: Bind conflict restores local state

- **WHEN** payload bind or tool-message append conflicts
- **THEN** the complete optimistic chat-store snapshot SHALL be restored
- **AND** the original invocation and DA rows SHALL be reconciled before another attempt

### Requirement: Frontend Projection Remains Bounded and Inert

The frontend tool detail MAY expose a bounded inert preview, `payload_available`, media type, exact byte length, and an explicit retrieve or export action through a typed Tauri command. Full payload bytes, opaque provider blocks, object keys, and signed URLs SHALL not enter the normal chat projection or generic runtime event stream.

#### Scenario: Large result reload remains responsive

- **GIVEN** a ready 8 MiB payload
- **WHEN** the conversation opens
- **THEN** the chat projection SHALL load only bounded metadata and preview content
- **AND** exact bytes SHALL transfer only after an explicit authorized action or internal model range read

#### Scenario: Payload renders as inert text

- **WHEN** a preview contains Markdown, HTML, a URL, or script text
- **THEN** the tool detail SHALL display it as inert text
- **AND** it SHALL not execute, linkify, or grant authority from that content

### Requirement: Temporary Payload State Obeys Local Data Lifecycle

Any temporary local bytes needed between tool completion and DA finalization SHALL be hard-bounded, account-scoped, protected by the existing local data controls, and deleted after success, explicit failure cleanup, cancellation, account change, or clear-local-data. A cleanup failure SHALL be reported through the existing typed cleanup contract.

#### Scenario: Cancellation cleans temporary bytes

- **GIVEN** a local upload is pending
- **WHEN** the run is cancelled before finalization
- **THEN** the desktop SHALL reconcile the DA payload state and remove local temporary bytes when safe
- **AND** an unsuccessful removal SHALL appear as a retryable cleanup failure

### Requirement: Payload Diagnostics Never Include Content

Diagnostics SHALL contain safe payload and invocation IDs, state, bytes, media type, duration, quota status, retry count, and integrity outcome only. Debug implementations SHALL omit exact bytes, previews, signed URLs, tool arguments, object keys, and credentials.

#### Scenario: Upload error is redacted

- **WHEN** payload upload or finalization fails
- **THEN** logs SHALL identify the safe operation and error class
- **AND** they SHALL contain none of the payload, preview, URL query, authorization header, or tool arguments
