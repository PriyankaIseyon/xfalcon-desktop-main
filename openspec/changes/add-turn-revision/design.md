# Design

## What the code looks like today (reconnaissance, 2026-08-18)

Recorded because the plan below only makes sense against it.

### The backend already has the primitive, unexposed

DA is the source of truth. The local encrypted projection is a mirror that
`ChatSyncService::reconcile_conversation` replaces WHOLESALE with
`ChatProjectionEvent::ReplaceSnapshot`, falling back to the local copy only when
the cloud is unreachable. So a cut made at DA needs no local delete path of its
own: the reconcile that follows it produces the correct local state.

DA exposes two counterparts:

- `POST /api/v1/chat/conversations/{id}:rewind` — SOFT. Supersede plus head
  reset, no rows deleted. Anchored on a checkpoint id.
- `POST /api/v1/chat/conversations/{id}:truncate` — DESTRUCTIVE. `from_message_id`
  is the cutoff; that message and everything after it (its tool calls, artifacts,
  attachments and trailing runs) is hard-deleted in ONE transaction, and the cut
  attachments' storage keys come back so the caller can reclaim the objects.

`TruncateRequest`/`TruncateResult` are already generated in
`clients/da/generated.rs`. `DaChatSession` exposes no method for either. Web's
`/api/conversations/[id]/rewind` route calls `:truncate`, so the destructive one
is the shipped semantics on both clients and is what this change uses.

### The submission path is one linear function

`ChatService::message_send_inner` is the only path that writes a user turn and
starts a run:

1. `authenticated_fresh` — session, or a typed failure
2. in-flight run cap (`MAX_IN_FLIGHT_RUNS`)
3. DA session construction
4. `list_models` → `select_model`
5. `inference_credential` — the provider key this device would run with
6. resolve or create the conversation
7. title bookkeeping (derived-title claim, upgrade claim)
8. `create_user_message_idempotent` with a per-submission key
9. `create_run`
10. register the cancel token — before any further await, because membership in
    `run_cancels` is what marks the run as owned by this process
11. reconcile + emit `snapshot_replaced`
12. emit `run_state_changed{queued}` — the conversation-to-run binding the UI
    store admits events by
13. spawn the local run, spawn the title upgrade

Steps 1–7 can all fail without having changed anything durable. That ordering is
what makes a safe cut point possible (below).

`message_send` wraps it in the `send_in_flight` mutex, which already serializes
submissions — the desktop equivalent of web's `rewindLockRef`.

### `chat_message_regenerate` is present, wired, wrong, and unreachable

It is a registered command, in `APP_COMMANDS` and the window permission list,
and no frontend code invokes it. `ChatService::message_regenerate` reconciles,
finds the last user message's text, and calls `message_send` with it — which
appends a duplicate user turn and leaves the previous answer standing. Had it
been reachable, one click would have produced: question, answer, the same
question again, another answer.

### The frontend keeps durable and live state apart, deliberately

`chatStore.messages` is the durable transcript only. Everything a token can move
(`streamingText`, `settlingText`, `activity`, `roundProse`, `reasoning`) is
separate, retires on one signal, and is read only by `ChatPane` through
`useTranscript`. `MessageRow` renders a user bubble and a copy button; there is
no edit or regenerate affordance anywhere. `MessageList` virtualizes the durable
rows with a grow-only measurement cache and remeasures on a `REMEASURE_EVENT`
that disclosures dispatch when their height changes.

## Decisions

### One backend call, not two

The reader asked for the backend work to happen "in parallel". It cannot, and it
does not need to.

The run's prompt is assembled from conversation history read out of DA inside
`spawn_local_run`. A run started before the cut lands would be given the very
answer it is supposed to replace — silently, and only sometimes, depending on
which request won. That is a correctness failure the reader would experience as
a model that ignores an edit.

What the request is actually asking for — that the app feel immediate — is
delivered by the optimistic frame instead: the removed turns disappear and the
thinking indicator appears BEFORE any IPC is issued. The perceived latency of the
click is one frame either way.

So the two server operations stay ordered, and they are put in ONE command:

```
chat_message_edit(conversation_id, message_id, text) -> ChatSubmitResult
chat_message_regenerate(conversation_id, message_id) -> ChatSubmitResult
```

One IPC hop instead of two, both under `send_in_flight`, and no window in which
the conversation is cut but nothing is running. Web's two-call shape has exactly
that window: if its second call fails, the reader's question is gone.

### Resolve first, then cut

The cut is placed AFTER step 7 and immediately before step 8. Everything that
can fail cheaply — session, model list, provider credential, conversation
access — has already been resolved by then. A revision that was going to fail
with "no usable key" therefore fails with the conversation intact, rather than
destroying an answer to report that it cannot produce another one.

### One enum, so the two facts cannot drift

A revision decides two things: where to cut, and whether the prompt becomes a new
user row. Passing those as two parameters invites a call site that sets one and
not the other, which would either duplicate a turn or lose one.

```rust
/// What a submission does to the conversation before its run starts.
enum TurnRevision {
    /// A new turn at the end of the conversation.
    None,
    /// The turn at `cutoff` and everything after it is removed, and the prompt
    /// takes its place. The reader edited their own message.
    Replace { cutoff: Uuid },
    /// Everything from `cutoff` on is removed and the prompt already stored
    /// above it runs again. No new user row is written.
    Rerun { cutoff: Uuid },
}
```

`writes_user_message` is derived (`None` and `Replace` write; `Rerun` does not),
and the cutoff comes off the same value. One field, two derivations, no drift.

### The cutoff is resolved from the server's snapshot, never from the client

The command receives a message id and nothing else. The service reconciles, finds
that id in the reconciled snapshot, and derives the cutoff itself:

- **edit** — the id must name a `user` turn. Cutoff is that turn.
- **regenerate** — the id names an assistant turn. Walk back to the nearest
  preceding `user` turn; the cutoff is the message immediately AFTER it, and the
  prompt is that turn's text.

An id that names no message, names the wrong role, or has no user turn above it
is refused with a typed error and nothing is cut. Message ids arrive from the
webview, which is a trust boundary; an index or a cutoff computed there would let
a stale renderer cut a different part of the conversation than the reader saw.
This is also why the client's optimistic cut is a DISPLAY decision only — the
server does not learn the boundary from it.

### A run in flight is cancelled before the cut

`send_in_flight` serializes submissions, not runs: a submission completes once
the run is spawned. So a revision can arrive while a run is streaming, and its
`finalize_run` would write an answer into a conversation whose tail has just been
removed.

Before truncating, the service cancels every active run of that conversation it
owns — the run ids come from the reconciled snapshot (`wire::is_active_run`
already names the states that count) and the tokens from `run_cancels`. The
cancel is not conditional on a prompt: the reader asked for a different answer,
which is itself the instruction to stop producing this one. Both ChatGPT and
Claude behave this way.

### Orphaned run journals are purged by set difference

`Journal::purge_run(account_id, run_id)` exists, and DA's truncate reports how
many runs it deleted but not which. Rather than reconstruct the boundary from
timestamps — `RunRecord` carries no timestamp, only a sequence — the service
compares the run-id set of the pre-cut snapshot against the post-cut one and
purges the journals of the ids that disappeared. Exact, and it needs no clock.

### The edit draft lives outside the row

`turnEditStore.ts`, in the same module-store shape as `composerStore` — which
exists for the analogous reason: draft text is not transcript state, and a
keystroke must not notify transcript subscribers.

It holds one message id and one draft, because only one turn can be under
revision at a time. Keeping it outside the row is not a preference: this
transcript is VIRTUALIZED, and a row scrolled out of the window unmounts. Web
keeps the draft in the row's `useState` and can afford to, because its list is
not virtualized. Here that would delete the reader's typing when they scrolled up
to re-read the question they were editing.

### Entering edit mode remeasures

A textarea plus two buttons is taller than the bubble it replaces. The
virtualizer's measurement cache is grow-only and keyed by row, so without an
explicit remeasure the rows below would be positioned against the old height —
the overlapping-text failure this surface has an explicit budget against. The
edit row dispatches the existing `REMEASURE_EVENT` on entering and on leaving,
the same mechanism disclosures already use.

### One regenerate affordance per response

A response can hold several assistant prose rows: a round's narration above its
tool umbrella, then the answer below it. Regenerating from any of them means the
same thing — run the user turn above again — so offering it on each would be
three buttons doing one job.

It is offered on the LAST prose row of each response, maintained incrementally by
`buildDisplayItems` as it walks, the same way `opensResponse` already is. That
build is on the streaming hot path; asking the question per row would make it
quadratic.

## Rejected alternatives

**DA's `:rewind` (soft supersede) instead of `:truncate`.** It would keep the old
answer as history and make branching possible later. Rejected for now: nothing in
this app reads superseded rows, so they would be invisible weight, and web ships
`:truncate` — two clients disagreeing about whether a revision is destructive is
worse than either choice. Branching is parked, and if it ships it will need the
soft path plus a pager, which is a change of its own.

**Client sends the cutoff index.** Cheaper — no server-side resolve. Rejected:
the index is computed from a snapshot the renderer may no longer share with the
server, and it arrives across a trust boundary. A stale index cuts the wrong
turn, and the reader's own transcript is the thing it damages.

**Truncate first, then submit, as two commands.** Matches web exactly and is
simpler to test. Rejected: it reproduces web's window where a failure between the
two leaves the conversation cut with nothing running.

**Leave `chat_message_regenerate` in place and add a second command beside it.**
Rejected by the project's no-backcompat contract, and it would leave a registered
command whose one behaviour is to duplicate a turn.
