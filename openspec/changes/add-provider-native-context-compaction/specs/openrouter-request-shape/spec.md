# Delta Spec: openrouter-request-shape

## ADDED Requirements

### Requirement: Primary Agent Uses OpenRouter Anthropic Messages

The OpenRouter primary-agent adapter SHALL send supported Claude Sonnet and Opus turns to `https://openrouter.ai/api/v1/messages` with bearer authorization and the fixed product attribution headers. It SHALL use Anthropic Messages content blocks, tools, streaming, output capacity, reasoning, context management, and usage semantics. It SHALL NOT send these agent turns through `/api/v1/chat/completions`.

#### Scenario: Supported turn uses the Messages endpoint

- **WHEN** a primary agent turn is sent through OpenRouter
- **THEN** the request origin and path SHALL equal the compiled OpenRouter Messages endpoint
- **AND** the selected model slug SHALL be unchanged
- **AND** no caller-supplied base URL SHALL be accepted

#### Scenario: Old Chat Completions wire is unreachable

- **WHEN** all primary-agent call sites are audited after cutover
- **THEN** none SHALL construct or call the OpenRouter Chat Completions dialect
- **AND** tests SHALL fail if that endpoint reappears in the agent path

### Requirement: Messages Wire Preserves Ordered Claude Blocks

The request serializer and SSE parser SHALL preserve complete ordered text, thinking or reasoning, tool-use, tool-result, image or document, and compaction blocks required by the supported protocol. Same-role merging SHALL keep tool results for parallel calls in one valid user turn without reordering blocks.

#### Scenario: Thinking and tool use round-trip exactly

- **GIVEN** a streamed assistant turn with signed thinking blocks followed by parallel tool-use blocks
- **WHEN** the tool results are sent
- **THEN** the assistant content blocks SHALL be returned in their original order and values
- **AND** every tool result SHALL reference the correct tool-use ID

#### Scenario: Compaction is not rendered as text

- **GIVEN** an active opaque compaction block
- **WHEN** a continuation request is serialized
- **THEN** it SHALL appear in the provider-defined content position and shape
- **AND** it SHALL not become a system string, transcript chat row, or summary-only field

### Requirement: Stable Session Identity Starts on the First Request

Every OpenRouter agent request SHALL carry a stable `session_id` derived from the canonical conversation UUID. The same value SHALL be used for retries, tool-loop requests, compaction-only requests, and continuations. It SHALL contain no user, tenant, prompt, or credential content and SHALL remain within 256 characters.

#### Scenario: Conversation stays sticky across compaction

- **GIVEN** one conversation produces an initial request, tool request, compaction request, and continuation
- **WHEN** their bodies or headers are inspected
- **THEN** each SHALL carry the same session ID from the first request onward

#### Scenario: Different conversations do not share identity

- **WHEN** two conversations send requests under the same account and model
- **THEN** their session IDs SHALL differ

### Requirement: Routing Requires Every Semantically Required Parameter

Supported Claude requests SHALL set OpenRouter provider routing to require support for tools, reasoning or thinking, context management, and any other semantically required parameter present in the request. A route that cannot support the request SHALL fail instead of silently dropping a parameter.

#### Scenario: Tool and compaction request cannot degrade to prose-only

- **WHEN** a turn includes tools and native context management
- **THEN** provider routing SHALL require parameter support
- **AND** a route that lacks either capability SHALL not be selected silently

### Requirement: Cache Breakpoints Follow the Stable Prefix

The adapter SHALL apply provider-supported ephemeral cache control to the deterministic stable system prefix and, when allowed by the current protocol, the active compaction prefix. It SHALL keep breakpoint count and placement within provider limits. A fingerprint change SHALL make the changed prefix a new cache identity.

#### Scenario: Stable long system prompt is marked

- **GIVEN** a supported Claude request with an eligible assembled system prefix
- **WHEN** the body is built
- **THEN** the prefix SHALL carry the current Anthropic cache-control shape
- **AND** changing the system, skills, tools, or compaction instructions SHALL change the context fingerprint

#### Scenario: Cache control does not alter opaque content

- **WHEN** a cache breakpoint is placed beside an opaque provider block
- **THEN** the block's own fields and values SHALL remain unchanged

### Requirement: Output Tokens Come from the Catalog

The OpenRouter Messages request SHALL set the required maximum output token field from the selected model's typed DA `max_output_tokens`, after checked integer conversion. It SHALL not use the generic 4,096 required-field fallback for the supported primary agent path.

#### Scenario: Catalog output limit reaches the wire

- **GIVEN** a supported model with output capacity `O`
- **WHEN** the request is serialized
- **THEN** the Messages maximum output field SHALL equal `O`
- **AND** input admission SHALL reserve the same `O`

### Requirement: Provider Errors Name the Compaction Failure Class

The adapter SHALL distinguish authentication, payment, rate limit with retry delay, temporary outage, hard context overflow, native-compaction unsupported, malformed compaction, malformed stream, TLS verification, cancellation, and generic invalid response. It SHALL keep provider content and secrets out of error values.

#### Scenario: Unsupported beta is actionable

- **WHEN** OpenRouter rejects the compaction header or edit
- **THEN** the desktop SHALL surface `native_compaction_unsupported`
- **AND** it SHALL not misclassify the response as a generic parse failure

#### Scenario: Hard context overflow is deterministic

- **WHEN** the provider rejects the request for context length
- **THEN** the error SHALL be non-retryable without context reduction
- **AND** normal same-body transient retry SHALL not repeat it
