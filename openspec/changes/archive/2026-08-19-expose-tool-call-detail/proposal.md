# Expose Tool Call Detail (Parameters & Result)

> STATUS (2026-08-05): APPROVED and IMPLEMENTED. Product sign-off recorded
> in-session ("reverse the stance and implement whatever xf web does, full
> details"). Implementation pivoted from the drafted lazy Tauri command to
> INLINE projection — investigation showed the conversation rows the app
> already fetches carry both payloads (assistant `tool_calls` arguments +
> `role="tool"` answer content), so web parity needs no new endpoint, no
> OpenAPI bump, and works offline. See tasks.md for the as-implemented
> record; design.md D1/D2 superseded accordingly. Additionally, the
> transcript now renders ONE umbrella disclosure per contiguous tool
> sequence (multi-round runs read as a single "N tools used" card above the
> final answer) — see the modified `chat-tool-trail` delta.

## Why

After `unify-tool-activity-ux`, tool rows show name + status + duration —
but never what a tool was asked or what it returned. xfalcon-web's tool
rows expand into "Parameters" and "Result" JSON panels (`ToolCallInfo`),
which is the single biggest remaining gap to reference-UX parity and the
main trust surface for a data-analysis product: users need to verify what
SQL ran and what came back.

Desktop currently withholds this **by documented design**: the chat
projection deliberately excludes tool arguments/results
(`crates/contracts/src/chat.rs:98-113`; `da_repository.rs:378-402` refuses
to project `role="tool"` rows). The execution data exists — the turn
driver records `ToolRoundCall { call_id, tool_name, arguments, result }`
and writes the `xf_chat.tool_calls` log (input/output/error/status;
`history.rs` calls it "the right source for showing a user what ran").

**Decision gate — this change reverses that documented stance and requires
explicit sign-off before implementation.** The mitigation is that detail
is never projected or stored in the frontend: it is fetched lazily,
per-call, on explicit user expansion, size-capped, and rendered as inert
text.

## What Changes

- **BREAKING (contract/policy)**: tool arguments and results become
  user-visible on demand. The "tool answers are not projected" invariant
  is narrowed to "…not projected into the conversation view; they are
  readable via an explicit per-call detail command."
- New typed Tauri command `chat_tool_call_detail(conversation_id,
call_id)` returning `ToolCallDetail` (arguments, result, error, status,
  timing when recorded) — plus `build.rs` command-list and
  `security/capabilities.rs` allowlist entries, and a contracts-crate type
  regenerated into `contracts.ts`.
- Tool rows (durable trail and live card, post-run) become expandable into
  labeled **Parameters** and **Result** panels: pretty-printed JSON in a
  capped, scrollable mono block with a copy affordance; error text panel
  when the call failed (web `ToolCallInfo` parity).
- Payload safety rails: per-panel byte cap with an explicit truncation
  indicator, plain-text rendering only (no markdown/HTML interpretation of
  tool payloads), session-scoped in-memory cache keyed by call id.
- Naming parity fix: `ToolCallDetail` carries the true tool name (and
  server identity where recorded), letting durable rows drop the
  underscore wire-name compromise from `unify-tool-activity-ux`.

Non-goals: streaming argument deltas (`tool_call_delta` stays dead),
persisting detail into the encrypted projection, showing detail for other
devices' runs if the local log lacks them (see Open Questions / D1),
redaction/secret-scrubbing policies (explicitly deferred; the gate
decision should note it), approval-risk annotations.

## Capabilities

### New Capabilities

- `chat-tool-call-detail`: on-demand fetch and display of a tool call's
  parameters/result/error — command contract, loading/error/truncation
  states, rendering safety, caching.

### Modified Capabilities

- `chat-tool-trail`: rows gain an expandable detail body (Parameters /
  Result / Error panels); the "naming does not guess" rule is upgraded —
  durable rows SHALL prefer the true name from detail data when available.
  Base is `unify-tool-activity-ux`'s delta; that change MUST be archived
  before this one.

## Impact

- Affected code: `src-tauri` (new command handler + repository read of the
  tool-call log, `build.rs`, `security/capabilities.rs`),
  `crates/contracts` (new `ToolCallDetail` type → regenerated
  `src/lib/generated/contracts.ts`), `src/lib/native.ts` (typed wrapper),
  `ToolActivityCard` family (detail body), `src/index.css` (panel styles).
- Tests: Rust command + repository tests, allowlist test, frontend
  adapter/panel tests, visual baseline for an expanded detail row.
- Sequencing: archive `unify-tool-activity-ux` first; if the detail source
  turns out to be DA-side only (see design D1), a DA endpoint + OpenAPI
  pin bump becomes a cross-repo prerequisite (DA merge+deploy before
  desktop consumption).
- Security review required: this is a deliberate policy reversal —
  reviewer must sign off on the narrowed invariant wording in
  `crates/contracts/src/chat.rs` and the mitigation set.
