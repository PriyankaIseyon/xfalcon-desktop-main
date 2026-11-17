# Design: Provider-Native Context Compaction

## Context

XF Desktop is the only product run executor. It resolves tools locally, streams provider output, and mirrors durable state to the DA chat data plane. The model context is now a provider-neutral `Vec<ContextMessage>` with a byte budget. The selected model's `context_length` is converted to bytes by reserving 25% for output and assuming four bytes per token. Each tool result is cut to 4,096 bytes before admission. The builder later reduces older results to a short excerpt and evicts complete groups. The turn driver also stops at 512 messages. DA history rebuild reads one 200-row page.

Those safeguards solved a smaller-context system. They are not correct for a 1M-context BI agent. A table, query result, report plan, or MCP response can carry high-value evidence. A fixed first-prefix cut hides it. A hidden recent-message cap makes a restored conversation different from the conversation that produced the last answer.

The supported execution contract for this change is narrow:

- provider credential: OpenRouter;
- model family: Anthropic Claude Sonnet or Opus;
- context capacity: at least 1,000,000 tokens from the tenant model catalog;
- provider protocol: OpenRouter Anthropic Messages API; and
- durable context service: the approved DA `add-durable-context-compaction` contract.

This narrow contract lets the design use the provider's native semantic compaction. It does not add weak generic behavior for models that lack it.

## External Basis

Official documentation was verified on 2026-09-01:

- Current Claude Sonnet and Opus model families provide a 1M-token context window.
- Anthropic states that larger inputs can suffer context rot. A hard window is not a quality target.
- Native compaction uses `context_management.edits` with `compact_20260112`, a 700,000-token automatic trigger for the supported million-token models, and a minimum 50,000-token capability-probe trigger.
- A response can contain a `compaction` block. The client must return the complete block on future requests.
- `pause_after_compaction` lets an agent persist the block before it continues.
- Provider usage can contain multiple iterations. Compaction work must be included once in cost and token totals.
- OpenRouter exposes `/api/v1/messages`, `context_management`, and a stable `session_id` contract.
- OpenRouter says supported `x-anthropic-beta` headers can pass through routing, but its public routing guide does not explicitly promise this compaction beta on every route.
- OpenRouter's generic middle-out transform can remove middle content. It is an emergency transport transform, not durable semantic state.

The exact header and response behavior must be rechecked during implementation. The desktop never assumes beta support from a model name alone.

## Goals

1. Use the full product model capacity without treating the hard limit as the quality target.
2. Compact automatically with provider-native semantics and durable activation.
3. Restore a complete inference snapshot after restart or device change.
4. Preserve exact full BI tool results and let the model retrieve selected ranges.
5. Keep tool pairing, reasoning signatures, provider block order, usage, caching, cancellation, and retry correct.
6. Fail loudly on unsupported routes, stale snapshots, incomplete persistence, or unsafe capacity data.
7. Keep changes at the smallest complete cross-layer footprint.

## Non-Goals

- Do not support non-OpenRouter execution in the primary agent path.
- Do not support non-Claude models in this context manager.
- Do not keep the old OpenRouter Chat Completions request as a fallback.
- Do not call Anthropic directly.
- Do not build a local summarizer or a second summary format.
- Do not add vector retrieval or long-term semantic memory.
- Do not send every 8 MiB result into the base model context.
- Do not delete or rewrite raw DA transcript history when compaction activates.
- Do not expose signed payload URLs or exact full result bytes to the frontend contract by default.

## Architecture

```text
DA context snapshot
  active opaque block + complete tail + payload descriptors
                         |
                         v
capacity gate -> OpenRouter Anthropic Messages -> streamed events
                         |                            |
                         |                            +--> text / thinking / tool calls
                         |                            +--> usage iterations
                         |                            +--> compaction block
                         |                                      |
                         +<-- persist + CAS activate through DA-+

tool result -> exact DA payload (<= 8 MiB)
           -> structured model preview (<= 64 KiB) + opaque payload id
           -> internal range-read tool for more evidence
```

The context manager owns a state machine. Provider adapters only serialize and parse the exact wire. DA owns durable compare-and-set state. The tool layer owns exact bytes and structured previews. No layer silently invents state for another.

## Decision 1: One OpenRouter Claude Messages Wire

The OpenRouter adapter for primary agent execution moves from `/api/v1/chat/completions` to `/api/v1/messages`. It uses bearer authorization and the existing fixed attribution headers. The request uses the selected OpenRouter model slug unchanged.

The request shape contains:

- model;
- system content blocks;
- ordered user and assistant messages with ordered Anthropic content blocks;
- tools with JSON schemas;
- maximum output tokens from the DA model catalog;
- streaming;
- reasoning or thinking shape supported by the current routed model;
- `context_management` native compaction edit;
- provider routing that requires every requested parameter;
- a stable `session_id`; and
- the verified compaction beta header.

`session_id` is the canonical DA conversation UUID. It is present from the first provider call, stable across retries and compaction continuations, contains no tenant or user text, and remains under OpenRouter's 256-character limit.

The old OpenAI-compatible OpenRouter agent path is removed with all call sites. Direct provider adapters are outside this execution contract. If another product path still uses one, that use must be identified before implementation and left explicit. It cannot become an implicit agent fallback.

## Decision 2: Preserve Ordered Provider Content Blocks

The current neutral message carries text, tool calls, tool results, attachments, and separate opaque reasoning details. Native compaction adds another opaque block. The provider layer needs a lossless ordered representation for:

- text;
- thinking or reasoning with signatures;
- tool use;
- tool result;
- images or documents already supported by the request; and
- compaction.

The new provider content value is opaque outside serialization, pairing, and size accounting. The stream parser emits complete blocks only after their stop event. It never rebuilds a thinking signature, changes order, extracts only compaction text, or parses unknown nested provider fields into a closed lossy struct.

Provider-neutral application messages remain useful for other adapters and UI projection. The OpenRouter Claude wire uses a protocol envelope alongside them. The implementation must not add compaction as fake `ContextRole::System` text or as a fake chat row.

## Decision 3: Probe the Route and Fail Closed

Native compaction is beta and OpenRouter routes across providers. Credential verification performs a small Claude Messages request that includes the current compaction edit, beta header, and `provider.require_parameters=true`. It uses the minimum legal compaction trigger but does not send 50,000 tokens only to force a block.

The probe proves request acceptance and records a short-lived local capability result keyed by OpenRouter account credential ID, concrete model ID, protocol version, and beta version. It does not mark the DA model globally capable. The first real compaction block provides the round-trip proof.

If the route rejects the parameter or header, returns an incompatible wire, or silently drops a required request parameter, the run fails with `native_compaction_unsupported`. The UI explains that the selected OpenRouter route is not compatible. The adapter does not switch to Chat Completions, generic middle-out, or a local summary.

Capability cache expiry is 24 hours or earlier on credential, model, route, protocol, beta, or provider error change. A cached failure can be retried by credential verification. Secrets and response content never enter the cache.

## Decision 4: Native Compaction Is a Two-Request Durable Barrier

Every eligible request includes one native compaction edit with a trigger of 700,000 input tokens and `pause_after_compaction=true`. For the minimum supported 1M-token model with a 128,000-token output limit and the 16,384-token protocol reserve, this preserves 155,616 input tokens for large tool activity and estimator drift before the hard admission boundary.

When the stream returns a complete compaction block, the provider adapter emits `CompactionReady` with:

- the complete opaque block;
- provider request ID;
- model and protocol identity;
- usage aggregate and iteration details;
- source head and context fingerprint from the request; and
- trigger reason.

The turn driver enters `persisting_compaction`. It stops normal provider continuation and calls DA activation with the expected active compaction, history revision, snapshot head, source boundary, and digest.

- On commit, the driver replaces the covered prefix with the returned active block and sends the continuation request with the same `session_id`.
- On `context_conflict`, it discards the stale provider block, reloads context from page one, and restarts the turn only when idempotency and tool side effects make that safe.
- On an ambiguous network failure, it reconciles the compaction ID through DA before retry.
- On an explicit DA failure, it does not continue the provider turn.
- On cancellation, it completes an in-flight activation reconciliation before it reports the final durable run state.

The provider may compact more than once in one logical user turn. Each committed block becomes the parent of the next. Usage and run events remain ordered and idempotent.

## Decision 5: Restore Through the Context Snapshot, Not Bootstrap

Before a run starts, desktop requests `GET /api/v1/chat/conversations/{id}/context` and follows every cursor. It validates each page's conversation, active compaction ID, history revision, frozen head, order, uniqueness, and cursor continuity.

It then:

1. assembles the current system, agent, skills, tools, reasoning settings, and document manifest;
2. computes the versioned context fingerprint;
3. accepts the active compaction only when provider protocol, block format, model, fingerprint, digest, and lineage are compatible;
4. rebuilds every post-boundary message in order;
5. preserves complete assistant tool-call and tool-result groups; and
6. rejects an incomplete or inconsistent snapshot instead of continuing with partial history.

A `context_snapshot_stale` response restarts paging from page one. Repeated staleness uses bounded retry and then reports a conflict. The 200-message bootstrap and single-page message list remain UI paths only. No inference caller may use them.

The encrypted local projection may show stale history offline, but a provider run requires an authoritative DA snapshot. It does not compact or upload against a stale local-only view.

## Decision 6: Progressively Bootstrap an Oversized Legacy History

An existing conversation can have no active compaction and more raw history than one safe provider request can hold. Silently taking the newest rows is prohibited.

The context manager performs progressive bootstrap compaction:

1. Find the largest oldest closed prefix that fits the conservative hard input budget.
2. Send that prefix with native compaction and `pause_after_compaction=true`, without asking for a user answer.
3. Activate the returned block through DA for that exact prefix.
4. Reload the new snapshot.
5. Add the next closed transcript segment after the block and compact again when needed.
6. Repeat until the complete retained history and current turn fit.

Every step is durable and resumable. The operation never splits a tool exchange. It never executes historical tool calls. It shows a bounded “Preparing long conversation” state and remains cancellable between provider requests. Each provider request is a real billed request, so the UI reports this only when the migration is needed. A failure leaves the latest committed prefix active and can resume later.

## Decision 7: Replace Byte Percentage with Layered Token Safety

The catalog supplies `context_length` and `max_output_tokens`. The supported model must report at least 1,000,000 total tokens. The input hard limit is:

```text
hard_input_tokens = context_length - max_output_tokens - 16,384 protocol_safety_tokens
```

The 16,384-token reserve covers request framing, provider-added tokens, and estimator error. It is fixed by protocol version, not exposed as user configuration.

The context manager tracks:

- exact provider `input_tokens` and cache-token usage from the last completed request;
- exact bytes and block counts of additions since that request;
- one-token-per-UTF-8-byte worst-case cost for unmeasured new text;
- explicit attachment estimates from the provider protocol; and
- fixed per-message, tool-schema, and content-block overhead from tested wire fixtures.

The worst-case estimate is the hard admission gate. Provider-reported usage is the quality and compaction signal after each response. The old four-bytes-per-token conversion and fixed 75% input share are removed. The fixed 512-message ceiling is removed. Message count remains bounded only by the hard token budget, DA page bounds, and complete-group invariants.

The provider's 150,000-token native trigger is the normal quality boundary. If the projected next request can exceed the hard input limit before the provider can compact, the driver first performs a compaction-only request over the largest safe closed prefix. If no safe prefix exists, it fails `context_item_too_large` and names the offending item type and size without content.

## Decision 8: Preserve Full Tool Results, Send a Structured Preview

The 4,096-byte `MAX_TOOL_RESULT_CONTEXT_BYTES` limit is removed. A successful tool outcome produces two representations:

1. Exact serialized bytes, up to the existing 8 MiB MCP response boundary, persisted through DA `tool_result_payloads`.
2. A model-visible `ToolResultPreviewV1`, up to 64 KiB, plus the opaque payload ID, exact byte length, SHA-256, media type, format version, and an explicit `complete` flag.

The preview is type-aware:

- JSON keeps a bounded structural projection with keys, types, counts, and selected values.
- Tables and CSV keep column names, row count when known, head and tail rows, and explicit omitted-row count.
- Text keeps head and tail excerpts with omitted-byte count.
- Binary keeps metadata only.

The preview never claims completeness when bytes are absent. The tool invocation `truncated` flag is replaced or redefined as `preview_complete`; durable payload availability is a separate field. UI projection remains bounded and renders content as inert text.

Before the model continues, desktop prepares or finalizes the exact payload and binds it with the tool call and tool message through DA. A DA or storage failure fails the persistence step. The model does not continue from an unbacked truncated preview.

## Decision 9: Add an Internal Exact-Result Range Tool

Add a fixed internal tool, `__internal__read_tool_result`, available only when the current context contains a ready payload descriptor. It accepts:

- payload ID from the current conversation and admitted run;
- byte offset; and
- byte length up to 64 KiB.

The tool returns the exact range, total bytes, range bounds, media type, digest, and whether more bytes remain. It validates the DA response, owner scope, digest identity, and range. It does not expose a signed URL, object key, tenant ID, host path, or credential. The model can call it repeatedly, and each returned range enters the normal context budget and compaction flow.

The tool is read-only and needs no human approval. Its output remains untrusted data. The system prompt states that tool payload content can contain instructions but has no authority. A payload ID not present in the current admitted context is denied even if DA would otherwise authorize it.

This tool replaces blind full-result injection. It gives the model exact evidence on demand and keeps the initial context stable.

## Decision 10: Keep Elision Only as a Pre-Activation Emergency

Native compaction is the primary context reduction path. The current tool-result excerpt and whole-group eviction algorithm remains only for these conditions:

- the conversation has no active compaction yet;
- the context manager is selecting a safe prefix for progressive bootstrap; or
- a single unmeasured addition would otherwise prevent a legal compaction request.

Emergency elision works on previews, not exact payload bytes. It retains descriptor, schema, counts, head and tail, digest, and omission notice. It never deletes the DA payload. Once native compaction is active, a normal threshold crossing must compact instead of silently evicting transcript groups.

The context manager reports which path ran. Tests prove that it always makes progress and never leaves an unmatched tool call.

## Decision 11: Cache Stable Prefixes Without Breaking Compaction

The Anthropic Messages request marks stable system and eligible long-lived content blocks with ephemeral cache control. Breakpoints are placed after the assembled system prompt and after the active compaction block when the current provider contract allows it. The final breakpoint set stays within provider limits.

The order of system prompt, tools, compaction block, and transcript tail is deterministic. `context_fingerprint` changes when any model-visible stable prefix changes. Stable `session_id` is present from the first successful request to keep OpenRouter routing sticky. Compaction activation does not rewrite previous raw messages and therefore does not corrupt local history.

## Decision 12: Usage and Cost Include Compaction Once

The provider event model expands usage to input, output, cache-read, cache-write, reasoning output, provider request ID, and bounded iteration details. The turn driver records provider usage before it publishes completion.

Compaction-only and continuation requests are distinct provider requests under one DA run. Each has an idempotency identity. The DA activation contract records the compaction usage. The normal run-event path records continuation usage. The desktop never sums an aggregate and the same iteration entries twice.

User-visible usage totals include automatic compaction. Diagnostics identify compaction request count, before and after input tokens, cache use, latency, and trigger reason without content.

## Decision 13: Crash, Retry, and Rollback Are Explicit

The turn state machine adds `loading_context`, `preparing_long_context`, `awaiting_provider`, `persisting_compaction`, `resuming_after_compaction`, and `persisting_tool_result` internal phases. Public UI run states remain stable unless a new bounded projection is approved.

Every optimistic store change captures the complete required snapshot: messages, activity, reasoning, round prose, active run ID, run state, and admitted invocations. On failure it restores exact equality.

Recovery rules:

- Provider failure before a compaction block uses the existing same-model retry policy.
- A complete block is never retried to the provider before DA reconciliation.
- DA timeout after activation is ambiguous. Read by compaction ID before retry.
- A stale activation discards the block and reloads authoritative context.
- Tool payload timeout reconciles payload ID and status before upload or finalize retry.
- A process crash after a durable provider or tool state commit resumes from DA.
- A process crash before commit may repeat provider computation, but cannot duplicate tool side effects because historical tools are never re-executed and live invocation IDs remain idempotent.
- Cancellation cannot report a clean terminal state while an ambiguous durable mutation remains unreconciled.

## Decision 14: Contracts Stay Generated and Boundaries Stay Typed

The DA OpenAPI artifact is deployed and pinned before desktop code consumes it. `src-tauri/src/clients/da/generated.rs` is regenerated, never hand-edited. Wrapper methods map every declared status into typed desktop errors. Context and payload responses are validated as untrusted external data.

Any frontend-visible change starts in `crates/contracts/`, regenerates `src/lib/generated/contracts.ts`, and uses explicit typed Tauri invocation. Full provider blocks, full tool bytes, signed URLs, and raw content do not cross the IPC boundary. A bounded tool detail may expose `payload_available`, byte length, media type, and a user action to retrieve or export through a dedicated typed command.

## Security and Privacy

- OpenRouter keys remain borrowed secret values and never enter persistent context state.
- Provider blocks, tool bytes, previews, signed URLs, reasoning signatures, and message content never enter logs or debug formatting.
- All DA values are validated for size, enum, UUID, digest, sequence, lineage, ownership result, and URL origin before use.
- Tool payload content is always data. It cannot change approval, tool allowlist, system prompt, or policy.
- `__internal__read_tool_result` admits only payload IDs already present in the active context.
- Signed DA URLs, if used internally, must match the expected storage-origin policy and never reach provider or frontend.
- Local temporary bytes use the encrypted or protected data path, have a hard cap, and are removed on success, cancellation, account change, and clear-local-data.
- Diagnostics contain counts, tokens, bytes, IDs, status, timings, and safe error codes only.

## Failure Matrix

| Failure                                     | Required outcome                                      |
| ------------------------------------------- | ----------------------------------------------------- |
| Route rejects compaction beta               | Typed unsupported error; no Chat Completions fallback |
| Context page is stale                       | Restart paging from page one with bounded retry       |
| Context page is incomplete or malformed     | Fail before provider call                             |
| Provider returns malformed compaction block | Invalid-response failure; no DA write                 |
| DA activation conflicts                     | Discard block, reload, and restart only if safe       |
| DA activation response is lost              | Reconcile by compaction ID before retry               |
| Crash before activation commit              | Recompute; old pointer remains active                 |
| Crash after activation commit               | Restore committed block from DA                       |
| Payload upload or finalize fails            | No model continuation from unbacked preview           |
| Payload bind fails                          | Restore full local turn snapshot and reconcile        |
| Range tool sees different digest            | Integrity failure; do not return bytes to model       |
| A single item exceeds hard budget           | Typed item-too-large failure with safe metadata       |
| Provider compacts twice in one turn         | Commit ordered lineage and usage once per request     |
| User cancels during bootstrap               | Stop between requests; keep last committed checkpoint |

## Migration Plan

1. Approve and implement DA `add-durable-context-compaction`.
2. Deploy DA to staging and pin the new OpenAPI artifact.
3. Regenerate the desktop DA client and add read-only context restore behind an internal test gate.
4. Add tool payload persistence and the internal range-read tool. Keep provider wire unchanged until end-to-end payload tests pass.
5. Add the OpenRouter Claude Messages adapter, capability probe, ordered blocks, usage, and native compaction state machine.
6. Replace history bootstrap and old capacity logic in one pass. Remove every inference call site that uses the old contracts.
7. Validate fresh, long, legacy, rewound, restarted, concurrent, failed, and cancelled conversations against staging.
8. Enable for the supported Sonnet and Opus catalog rows after route probe.
9. Remove the obsolete OpenRouter Chat Completions agent path, 4 KiB result cap, 512-message cap, 75% byte formula, and hidden 200-message inference read.
10. Soak with restore, compaction, conflict, payload, token, cache, latency, and cost telemetry.

There is no dual provider-write path. Each implementation layer remains runnable, but cutover removes all old inference callers in the same verified change.

## Rollback

Before native writes, rollback restores the prior desktop build. After compaction or payload writes exist, a rollback build must still read the new DA resources or refuse to run that conversation with a clear upgrade-required error. It must not ignore an active compaction and silently use a recent transcript page. Provider compaction writes can be disabled while restore and full-payload reads remain enabled.

## Rejected Alternatives

- **Only remove the 4 KiB constant.** One huge result can consume or exceed the context and still cannot restore after restart.
- **Send all 8 MiB results.** A single result can exceed a safe 1M input and harms latency and recall.
- **Keep using Chat Completions.** It cannot preserve Anthropic's native compaction contract with the required block semantics.
- **Use generic OpenRouter compression.** It can discard middle evidence and has no durable semantic lineage.
- **Create a local summary fallback.** It adds a second compaction protocol and hides route incompatibility. The supported Claude route must provide the native contract.
- **Use the UI bootstrap for context.** Its bounded recent page can never prove completeness.
- **Store full bytes in the local transcript.** It duplicates DA object storage and expands encrypted projection and IPC blast radius.
- **Expose a signed URL to the model.** It leaks a transport credential and asks the provider to fetch private BI data.
- **Treat a provider block as plain summary text.** It loses opaque fields and can break later provider requests.

## Resumption Checklist

An implementation session must first read this design, all five delta specs, `tasks.md`, the approved DA change, `src/lib/generated/contracts.ts`, and `crates/contracts/`. It must verify the current OpenRouter and Anthropic wire again. It must regenerate pinned contracts before editing wrappers. It must preserve current append-only, rollback, tool-pairing, event-admission, and local-data invariants. A task checkbox needs automated or verified evidence.
