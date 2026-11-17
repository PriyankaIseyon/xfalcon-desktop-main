## Context

DA owns the private model row and encrypted key. Desktop owns inference. DA
validates URL syntax but does not check DNS or connect to the endpoint. The
Desktop contract generator consumes only the pinned staging OpenAPI URL.

## Decisions

### Model identity and selection

Keep the four fixed product IDs and the Sonnet default. Add only the signed-in
caller's `custom_openai` rows that are active, enabled, chat and streaming
capable, support tools, and have valid capacity and complete endpoint/key
metadata. A model without tools is unavailable in the picker. Store
`custom/<uuid>` in DA runs and conversations. Use its upstream ID only in the
provider request. A locked conversation with a removed or missing model fails
with a clear removal message. Keep the retired Luna path unchanged.
Drain DA's cursor pages so a caller with more than one page of private models
can still select an older row. Filter private rows to the active account.
Only offer a private row when the native context budget can reserve its output
and protocol safety tokens. Settings checks the same limit before submitting
new capacity values and explains how to make an unsupported row usable.

### Separate guarded transport

Existing providers use fixed origins. A dedicated `custom_openai` adapter may
use a caller-supplied origin, but only after validation of the DA model row.
It shares the existing OpenAI Chat Completions request and stream parser, not
the fixed-origin adapter's endpoint constant. Every request rechecks HTTPS,
host, userinfo, query and fragment; resolves the hostname; rejects any
non-public address, including mapped IPv6 and metadata ranges; and connects
only to a checked address. TLS still verifies the original hostname. The
client keeps `Policy::none()` and does not follow redirects. Tests may inject a
loopback-only resolver; production has no bypass.

### Exact credential binding

Read `base_url`, `upstream_model_id`, and `provider_key_id` from the selected
model. Resolve only that key through DA. Compare the returned ID, provider,
and base URL with the model. On a mismatch, reload both once and retry the
comparison. A second mismatch fails. No provider-wide fallback is allowed.
Keep the secret zeroized, mark the authorization header sensitive, and drop the
secret after the request. Errors and logs contain no key or full URL.

### Management and state

Create, edit, and remove use the interactive Casdoor JWT. Edit omits unchanged
fields and requires a new key when the URL changes. Never return or render a
stored key. Refresh the per-account catalog after each successful mutation,
including removal. Keep `custom_openai` out of Settings > Providers key entry;
DA permits management only through the custom-model routes. Add commands to
the pinned Tauri capability set and type every renderer invocation with the
generated contracts.
The management list queries each enabled/active filter state because DA's
picker route defaults both to true. It omits rows whose key was revoked and
deduplicates rows that change state between those queries. Settings can disable
and re-enable a model; re-enabling asks for a fresh key so a disabled key can
be made usable in the same update. An unconfirmed mutation closes its editor
and refreshes the list before the user retries. A successful empty catalog
refresh clears the previous snapshot so revoked models leave the picker.
Catalog and management responses are checked against the current account
before returning, so an in-flight request cannot expose a former account's
private model names after an account switch.
Settings groups the default dropdown, private models, provider keys, and MCP
servers under one Models & connections section. The dropdown scrolls for large
catalogs and shows the provider beside each model. The account has one saved
default model ID, so choosing either kind replaces the prior default. The
choice applies to new chats; existing conversations keep their locked model
and provider key route.

## Risks and verification

- DNS rebinding and redirects could leak a key. Pin the checked address per
  request and reject redirects; test both attack paths through the transport.
- Model and key can change between reads. Reconcile once, then fail closed.
- Generated DA types require the deployed, pinned staging OpenAPI. Do not
  hand-edit them or substitute the local branch's OpenAPI.
- The WSL quality gate runs Windows Rust. macOS execution remains unverified
  until the macOS CI gate runs.
