## 1. Contract and catalog

- [x] 1.1 Sync and pin the deployed DA staging OpenAPI through the repository
      script. Update every `ModelEntity` test literal and verify generated types.
- [x] 1.2 Admit eligible private rows to offered and locked selection while
      preserving the four product IDs, Sonnet default, and retired Luna behavior.

## 2. Guarded inference

- [x] 2.1 Add a separate `custom_openai` adapter with request-time DNS checks,
      checked-address pinning, TLS hostname verification, and redirect refusal.
- [x] 2.2 Resolve the model's exact DA key, compare identity and endpoint,
      reconcile a mismatch once, and fail closed. Use the upstream model ID on wire.
- [x] 2.3 Give distinct safe user errors for refused address, rejected key,
      unreachable endpoint, and removed model.

## 3. Management and UI

- [x] 3.1 Add typed IPC commands for create, edit, and remove, and admit them
      in the pinned capability set.
- [x] 3.2 Add Settings fields for URL and key only, with the endpoint prefilled
      for the company gateway. Require a fresh key on URL change.
- [x] 3.3 Refresh the account catalog after each mutation and mark private
      models in the picker without exposing keys.
- [x] 3.4 Show one visible default-model choice across OpenRouter and private
      models, with one checked option and provider labels.
- [x] 3.5 Keep the default choice in a compact dropdown and group models with
      provider keys and MCP servers under one Settings heading.

## 4. Verification

- [x] 4.1 Test address classes, DNS rebinding, redirect refusal, exact-key
      mismatch and retry, upstream request ID, key error, removal, and redaction.
- [x] 4.2 Test catalog selection, mutation refresh, UI behavior, and capability
      pin against the real consumers.
- [x] 4.3 Run `bash scripts/gate.sh`, inspect the complete diff, and document
      any platform-gated validation limit.

## 5. Company gateway settings

- [x] 5.1 Fix the model name, limits, sampling, timeouts, retry and single-request
      slot for gateway endpoints, and apply the model name and limits to stored
      rows on read. Proven by the `custom_openai`, `gateway` and
      `custom_models_wire` tests.
- [x] 5.2 Read `reasoning_content` as thinking. Proven by
      `reasoning_content_is_read_as_thinking`.
- [x] 5.3 Shed what can be shed instead of refusing a request over the input
      room, and name the token sizes in the refusal that remains. Proven by the
      `summary` shedding tests and the bounded-context turn-driver tests.
- [x] 5.5 Continue a response cut off by the connection. Proven by the
      `StreamCut` tests in `openai_compat` and `chat::tests::tool_run`.
- [ ] 5.4 Apply the two-hour run and 3,720-second stall limits in
      `spawn_local_run`. Implemented; no automated test drives that wiring yet.
