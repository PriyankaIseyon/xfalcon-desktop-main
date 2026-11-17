## ADDED Requirements

### Requirement: A signed-in user can manage private models

Desktop SHALL create, edit, and remove the user's private models through DA's
interactive-session custom-model routes. It SHALL show the model configuration
but never show a stored key. A URL change SHALL require a new key. Each
successful mutation SHALL refresh the caller's model catalog.

#### Scenario: Create a private model

- **WHEN** the signed-in user submits a valid endpoint, key, and reviewed model settings
- **THEN** Desktop verifies a completed greeting before creating it through DA
  with the selected model name and limits
  and lists it with a custom marker
- **AND** the response and subsequent model views contain no key
- **AND** the form starts with URL and key before model discovery or manual configuration

#### Scenario: Change the endpoint

- **WHEN** the user edits a private model's URL
- **THEN** Desktop requires a new key in the same update
- **AND** verifies the new connection before changing the saved model or key
- **AND** refreshes the model catalog after DA accepts the change

#### Scenario: Remove a private model

- **WHEN** the user removes a private model
- **THEN** Desktop calls DA's remove route and refreshes the picker
- **AND** a conversation locked to it reports that the model was removed

### Requirement: Only eligible caller-owned private models can run

Desktop SHALL offer the caller's active, enabled, chat, streaming, and
tool-capable `custom_openai` rows with valid capacity and complete binding
metadata. It SHALL keep the four product models and Sonnet default. It SHALL
send `upstream_model_id` to the endpoint while recording the stable
`custom/<uuid>` ID and `custom_openai` provider in DA. Private models SHALL
use bounded context and their catalog context and output limits.

#### Scenario: A private model lacks tools

- **WHEN** DA returns a custom model with `supports_tools: false`
- **THEN** Desktop does not offer or execute it

#### Scenario: A valid private model is selected

- **WHEN** the caller selects an eligible private model
- **THEN** its upstream ID goes to the endpoint and its stable ID goes to DA
- **AND** the four product models remain available in their existing order

#### Scenario: Choose one default across providers

- **WHEN** the user selects a private model or an OpenRouter model as the default
- **THEN** Desktop saves that one model ID as the account's default for new chats
- **AND** Settings shows exactly one selected default across both providers
- **AND** existing conversations keep their locked model and provider key route

#### Scenario: Manage models and connections together

- **WHEN** the user opens Settings
- **THEN** one Models & connections section contains the default model dropdown,
  private models, model providers, and MCP servers
- **AND** the dropdown remains usable when the catalog contains many models

### Requirement: Private inference is bound to a safe address and exact key

Desktop MUST validate the stored HTTPS URL at use time, resolve the hostname
for every request, reject all non-public addresses, and connect only to an
address it checked. TLS MUST verify the hostname and redirects MUST NOT be
followed. Desktop MUST resolve only `provider_key_id` from the selected model
and compare the key's ID, provider, and base URL with that model. A mismatch
MUST trigger one reload of both values; a second mismatch MUST fail closed.
The resolved key MUST remain in zeroizing memory and absent from logs and
errors.

#### Scenario: A hostname changes to a private address

- **WHEN** DNS resolves the model host to a private, loopback, link-local,
  mapped, metadata, or other non-public address
- **THEN** Desktop refuses the request before sending the key

#### Scenario: The binding changes between reads

- **WHEN** the resolved key no longer matches the selected model
- **THEN** Desktop reloads both once
- **AND** refuses inference if the second comparison still differs

#### Scenario: The endpoint redirects to a private host

- **WHEN** a validated endpoint returns a redirect
- **THEN** Desktop does not follow it or send the key to the new host

### Requirement: Private model failures give safe, actionable reasons

Desktop SHALL distinguish an address refusal, a 401 or 403 key rejection, an
unreachable endpoint, and a removed model. Errors MAY show the hostname but
MUST NOT show the full URL or key.

#### Scenario: The endpoint rejects the key

- **WHEN** the endpoint responds with 401 or 403
- **THEN** Desktop tells the user to update the private model's key

#### Scenario: An old removed model is no longer in DA

- **WHEN** DA returns 404 for a locked private model that has aged out
- **THEN** Desktop reports that the model was removed

### Requirement: The known company model has tuned transport

Desktop SHALL recognize the company gateway and exact upstream model
`ollama/qwen3.6:35b-a3b-coding-nvfp4` together. Discovery SHALL prefill settings
only from valid endpoint metadata. Stored settings SHALL remain authoritative.
Requests SHALL omit unset sampling values and report usage. The exact model
SHALL retain its serialized transport, reasoning controls, bounded retries,
and extended run limits.

#### Scenario: Known company model metadata is incomplete

- **WHEN** discovery returns the known company model without limits
- **THEN** Desktop leaves context, output, temperature, and Top P blank until supplied
- **AND** it requires confirmation of unknown tool and image capabilities
- **AND** other model IDs at the same host retain their own route and settings

#### Scenario: The gateway is busy

- **WHEN** the gateway answers HTTP 503 before streaming
- **THEN** Desktop waits 30, 60 and 120 seconds and retries up to three times
- **AND** an HTTP 504, an in-stream error or a timeout fails the run without a retry
- **AND** only connection failures, HTTP 502 and HTTP 503 before output are retried
- **AND** the transport allows 3,660 seconds in total and per read
- **AND** a run allows two hours and waits at least 3,720 seconds before declaring a stall

#### Scenario: A background request

- **WHEN** Desktop requests a title, a follow-up chip or a context summary
- **THEN** the request sends `reasoning_effort: "none"` so it finishes in seconds
- **AND** it waits behind a running answer instead of queueing a second request

### Requirement: A conversation that can be made to fit is never refused

When summarizing does not make a conversation fit a model's input room, or
fails, Desktop SHALL shorten the request itself without a model call: old tool
results and tool-call arguments become stubs, the biggest older messages are
cut in the middle, the oldest whole exchanges are dropped behind a note, and
only then are the newest messages cut. The model SHALL see every cut and the
note. Desktop SHALL refuse a request only when the model's instructions and
tool list alone exceed the room, and the refusal SHALL name the estimated and
allowed token counts.

#### Scenario: A message bigger than the whole window

- **WHEN** one message alone exceeds the model's input room
- **THEN** the run continues with that message cut in the middle and marked
- **AND** the person sees no error

#### Scenario: Summaries cannot shrink the history

- **WHEN** the summary request fails or returns nothing and the request is
  still over the input room
- **THEN** the oldest exchanges are dropped behind a note and the newest request
  is kept

### Requirement: A response cut off by the connection is continued

When a response ends with no `finish_reason` and no `[DONE]` marker, Desktop
SHALL treat the turn as cut off, not as a failure. It SHALL keep finished tool
calls, discard a half-written one, and ask the model for the rest in smaller
pieces. When a `finish_reason` already arrived, Desktop SHALL complete the turn.

#### Scenario: The connection closes mid-answer

- **WHEN** the stream closes after some text and no stop reason
- **THEN** the run continues from that text without an error
