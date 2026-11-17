# Design — conversation open, web parity

## Context

The reference implementation is xfalcon-web's chat path, mapped via codegraph
(2026-08-05, index: 626 files / 6987 nodes):

| Web element         | Location                                       | Contract                                                                                                                                                                               |
| ------------------- | ---------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Sidebar row click   | `src/components/Sidebar.tsx`                   | `setActiveConversationId(id)` only — no fetch in the click handler                                                                                                                     |
| Message hydration   | `src/hooks/useMessageLoader.ts:26-86`          | Effect on `activeConversationId`; abortable; prefetch-cache reuse; `suppressFetch()` after optimistic create                                                                           |
| Loader              | `src/lib/chat/conversation-runtime.ts:102-128` | Bootstrap RPC first (`/api/conversations/[id]/bootstrap`), silent fan-out fallback when bootstrap unavailable                                                                          |
| Not-found self-heal | `src/hooks/useMessageLoader.ts:58-72`          | `loadError === 'Conversation not found'` → `setActiveConversationId(null)`; `removeConversation(id)`; `history.replaceState('/')`. Transient errors keep the id so the user can retry. |
| Store shape         | `src/lib/store.ts`                             | Every slice keyed by conversation id; `removeConversation` sweeps all per-conversation auxiliaries                                                                                     |

Desktop already mirrors the happy path (keyed scoping, lazy create, stale
fallback). The gap is exclusively the failure path and the double reconcile.

## Goals / Non-Goals

- Goal: a click on a sidebar row either opens the conversation (live or stale)
  or visibly self-heals — never a silent dead click.
- Goal: one cloud round-trip per open.
- Non-goal: resolving why DA listed rows its own detail bootstrap 404s (server
  side; needs DA staging logs — tracked separately). The client must be correct
  in either world: if the list stops returning the row, refresh removes it; if
  the server keeps listing it, the user gets an explicit toast instead of
  nothing.
- Non-goal: web's hover/idle prefetch cache (an optimization, not correctness;
  desktop's local projection already gives instant stale opens).

## Decisions

### D1 — Single reconcile in `conversation_open`

`ensure_conversation_access` is itself just a reconcile; `open_and_emit`
reconciles again. For mutating commands (rename/archive/delete) the pre-flight
guard stays — the mutation must not run against a foreign/dead id. For open,
the open IS the reconcile; drop the pre-flight. Halves open latency and halves
the 404 noise that made the log hard to read.

### D2 — Preserve `NotFound` through the sync fallback

`reconcile_conversation`'s fallback (`sync.rs:90-97`) collapses
NotFound-without-local-copy into `Unavailable`. Callers cannot distinguish
"server says this conversation does not exist" (self-heal) from "network is
down" (retry later) — web treats these as distinct on purpose (the
`loadViaBootstrap` 404 discriminator comment). Keep the stale-copy fallback
for BOTH errors (desktop's offline advantage over web); only the
no-local-copy tail changes: `ok_or(...)` returns the ORIGINAL error.

### D3 — Self-heal lives in the service + hook, not the store

On `NotFound` from open: the Rust service purges the local projection row
(`ChatProjectionEvent::RemoveConversation`, same event `conversation_delete`
already uses at `service.rs:503`) and returns a typed not-found error string.
`useChat.openConversation` matches it, then: `toast.error("That conversation
no longer exists…")` → `refreshWorkspace()` (authoritative list re-fetch; drops
the row if the cloud agrees) → `beginNewConversation()` when the dead id was on
screen. This is exactly web's trio (clear active / remove row / go home) in
desktop idioms. Store gains no new concepts.

Typed matching: `ChatServiceError::Message("Conversation was not found.")` is
today's shape (`service.rs:1519-1521`). The TS side must not string-match a
human sentence — add a stable prefix marker (`[not_found]`) OR a dedicated
`ChatServiceError::NotFound` variant serialized distinctly. Decision: dedicated
variant, because string markers leak into toasts.

### D4 — Reject-log body snippet

`chat_api.rs` logs `status` + `path` on rejection. A 404 whose body is
`{"detail":"Not Found"}` is a missing ROUTE (deploy skew); a 404 whose body is
`{"code":"not_found",…}` is a missing ROW (RLS/ownership/deleted). Log the
first 200 bytes of the body at the existing WARN site. No behavior change.

### D5 — Composer: restyle only

The minimal look (reference: Bud composer screenshot, 2026-08-05) is a pure
presentation change to `Composer.tsx` + `.composer` CSS: soft rounded
container (`border-radius ~1.25rem`, subtle border, card background),
borderless textarea, placeholder "Ask anything", attach (`Plus`) affordance
bottom-left, circular filled send button bottom-right. Keyboard contract
(Enter sends, Shift+Enter newline, CJK composition guard), controlled-draft
API, a11y labels, and the project-name caption are all preserved. The model
badge moves out of the input row (it is chrome the reference deliberately
lacks) — its information remains available in Settings.

## Risks / Trade-offs

- Purging the local projection on NotFound deletes the offline copy of a
  conversation the server may later resurrect. Accepted: the purge only fires
  when there is no local snapshot worth keeping on the open path, or after the
  server authoritatively 404s the row — the same trust web places in that
  answer. (`conversation_delete` already sets this precedent.)
- Dropping the pre-flight reconcile from open removes a freshness pass that
  mutating commands keep. Open's own reconcile provides the same freshness.

## Open Questions

- Server side: why did the workspace list at 12:10Z include ten rows whose
  detail bootstrap 404s seconds later? Requires DA staging request logs
  (`chat.bootstrap.*` latency lines + RLS-denial counter). Not blocking this
  change; D4 makes the next occurrence self-diagnosing from the desktop log
  alone.
