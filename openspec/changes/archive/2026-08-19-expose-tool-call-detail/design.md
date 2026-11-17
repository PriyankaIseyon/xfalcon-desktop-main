# Design — Expose Tool Call Detail

> AS-IMPLEMENTED NOTE (2026-08-05): D1's investigation resolved against the
> lazy-command plan below. Tool rounds are durable in the conversation
> itself (`xf_chat.chats`: assistant `tool_calls` entries carry arguments;
> `role="tool"` rows carry results, paired by call id — written identically
> by desktop's `record_tool_round` and web). The bootstrap already fetched
> at conversation open contains everything, so payloads are projected
> INLINE onto `ChatToolCallView` (capped at
> `TOOL_PAYLOAD_PROJECTION_CAP_BYTES` = 32 KiB in
> `ToolCallRef::with_payloads`, truncation-flagged, `#[serde(default)]` for
> old sealed snapshots) instead of fetched per-expand. D2 (command
> contract) and D3 (lazy fetch/cache) are superseded; D4 (inert rendering)
> and D5 (panel presentation) shipped as designed; D6 (name upgrade)
> dropped — the conversation records only the wire name, so durable rows
> keep it. `xf_chat.tool_calls` remains the uncapped execution log.

## Context

- Frontend today (post `unify-tool-activity-ux`): rows =
  `{name, status, duration?}`; no arguments/results anywhere in
  `contracts.ts` (`ChatToolCallView` is deliberately minimal;
  `ToolInvocationEvent` carries lifecycle only).
- Rust records the full execution: `ToolRoundCall { call_id, tool_name,
arguments, result }` written by the turn driver
  (`turn_driver.rs:339-378`) and a tool-call log with
  input/output/error/status/sequence (`turn_driver.rs:380-411`,
  `history.rs` — "the right source for showing a user what ran").
- The exclusion is documented as a security stance
  (`crates/contracts/src/chat.rs:98-113`, `da_repository.rs:378-402`).
  This change narrows it deliberately (proposal decision gate).
- Reference UI: web `ToolCallInfo.tsx` — 9px uppercase label
  ("Parameters"/"Result"), pretty-printed JSON `<pre>` (10px mono,
  `max-h-56`, scroll, copy button with check feedback), rose error panel;
  expanded body indented under the row with a hairline rail.

## Goals / Non-Goals

**Goals:**

- User can answer "what exactly did this tool run, and what came back?"
  for any call in the current device's history — without the data living
  in the projection or the frontend store.
- Web `ToolCallInfo` visual parity inside the `ToolActivityCard` rows.
- Payloads treated as untrusted: inert rendering, size caps, no
  interpretation.

**Non-Goals:**

- Streaming argument deltas; redaction policy; cross-device detail
  backfill; persisting detail client-side; DA API changes unless D1 forces
  them.

## Decisions

### D1 — Source of truth: local tool-call log first (investigate at task 1)

Preferred path: the new command reads the **local** tool-call log the turn
driver writes (this device is the ONLY run executor — every run in this
app's history executed here, so local records should be complete for
conversations run on this install). Task 1 verifies where
`xf_chat.tool_calls` rows physically live (local sqlite vs DA data plane
via mirror):

- If local: command = sqlite read, no network, works offline. Chosen if
  available.
- If DA-only: requires a DA endpoint + OpenAPI pin bump (DA merge+deploy
  first — cross-repo sequencing), and offline expansion shows a "detail
  unavailable offline" state. Fallback, not preferred.
- Conversations synced from other devices (no local rows): detail request
  returns a typed `not_recorded` outcome → row shows "Detail not available
  on this device" quietly. (Today desktop is the only executor, so this is
  an edge case, not a launch blocker.)

### D2 — Command contract

```rust
// crates/contracts
pub struct ToolCallDetail {
    pub call_id: String,
    pub tool_name: String,          // true internal name, e.g. mcp.tenant.ida_query
    pub arguments_json: String,     // raw JSON text as recorded (may be "{}")
    pub result_json: Option<String>,// None when unanswered/failed-before-result
    pub error: Option<String>,      // sanitized error text
    pub status: ToolCallDetailStatus, // completed | failed | unanswered | cancelled
    pub truncated: TruncationFlags, // { arguments: bool, result: bool }
}
```

`chat_tool_call_detail(conversation_id, call_id)` — errors are `thiserror`
variants with stable machine tokens (`detail_not_recorded`,
`conversation_not_found`). **Truncation happens Rust-side** (per-field byte
cap, default 256 KiB, `truncated` flags set) so multi-megabyte results
never cross the IPC boundary. Timing fields are included only if the log
already records them (task 1 confirms; do not extend the write path in
this change).

New command requires `build.rs` command-list + `security/capabilities.rs`
allowlist updates (project convention), each with test coverage.

### D3 — Fetch policy: lazy, explicit, session-cached

- Fetch fires only on first expansion of a specific row (chevron click) —
  never on card expand, never eagerly.
- In-flight → skeleton panels; failure → inline error with retry;
  success → cached in a module-level `Map<call_id, ToolCallDetail>` scoped
  to the chat store's conversation lifecycle (cleared on conversation
  switch, never persisted).
- Live card rows become detail-expandable only once their invocation is
  terminal (completed/failed) — mid-run rows show status only (no deltas
  exist by design).

### D4 — Rendering safety (payloads are attacker-influenceable)

Tool results can embed hostile content (prompt-injection strings, HTML,
terminal escapes). Rules, aligned with the upload-cell-injection doctrine
(display-time neutralization, never mutate stored data):

- Panels render via React text nodes into a `<pre>` — no markdown
  rendering, no `dangerouslySetInnerHTML`, no linkification of payload
  content.
- Pretty-print (`JSON.parse` → `JSON.stringify(_, null, 2)`) only when the
  text parses as JSON **and** is under the pretty-print size threshold;
  otherwise show raw text verbatim.
- Truncated panels show an explicit "truncated — N KiB shown of M KiB"
  line; copy copies the truncated text (full-payload export is out of
  scope).
- Stored records are never mutated; caps/pretty-printing are display-time
  only.

### D5 — Panel presentation (web parity)

Expanded row body: indented under the glyph with a hairline rail
(`ml-5 pl-3 border-l`), stacked sections — "PARAMETERS" (always, `{}`
renders as empty-object), "RESULT" (when present), "ERROR" (rose panel

- existing retry affordances when failed). Each JSON block: mono,
  ~10-11px, elevated code surface (`.code-surface` tokens), `max-h-56`
  scroll, copy button with `badge-pop` check (change-1 pattern). Uses the
  same `.accordion-shell` collapse as the row disclosure (change 2).

### D6 — Naming upgrade

`ToolCallDetail.tool_name` is the true dotted internal name. Once a
detail is fetched, the row's display name upgrades to the parsed
`server · tool` form; the durable wire-name compromise (change 2, D5)
remains only for never-expanded rows. No bulk fetch just for names.

## Risks / Trade-offs

- [Policy reversal regret] → gate in proposal; invariant comment in
  `chat.rs` is rewritten (narrowed, not deleted) in the same PR the
  reviewer signs off; lazy-fetch + no-store keeps the blast radius one
  call at a time.
- [Huge results over IPC] → Rust-side truncation before serialization
  (D2), pretty-print threshold (D4).
- [Hostile payload content] → inert text rendering (D4); no interpretation
  anywhere.
- [Local log missing for some conversations] → typed `not_recorded`
  outcome with a quiet UI state (D1); never an exception.
- [Cross-repo dependency if DA-only] → decided at task 1 before any
  frontend work; if DA path, this change pauses at the spec boundary until
  the DA endpoint deploys (OpenAPI pin rule).

## Migration Plan

Sequenced after `unify-tool-activity-ux` archives. One branch:
(1) investigation + decision record (D1), (2) contracts type + command +
allowlist + Rust tests (Windows cargo), (3) `native.ts` wrapper +
fetch/cache layer + tests, (4) panel UI + visual baseline, (5) invariant
comment rewrite + security review sign-off. Rollback: revert branch;
command removal restores the old invariant wording.

## Open Questions

- Where do tool-call log rows physically live (local sqlite vs DA)? —
  task 1; determines D1 branch.
- Are timestamps/durations recorded in the log today? If absent, durable
  duration display silently stays unavailable (do NOT extend the write
  path in this change).
- Byte-cap default (256 KiB proposed) — confirm against real IDA result
  sizes during task 1 (typical `ida_query` result payloads).
