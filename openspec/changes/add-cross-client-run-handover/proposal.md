# Change: Follow a run another client is driving, instead of cancelling it

## Why

xFalcon Web is a second interface onto the same conversations, and a person is
expected to move between it and the desktop mid-conversation. DA is the source
of truth for both. The desktop does not behave as one of two clients.

**It destroys the other client's work.** `ChatService::workspace_bootstrap`
finalized every in-flight run it was not itself driving, on the premise — stated
in the code — that "this device is the only thing that executes runs". It is
not: the generated DA contract documents the web run engine minting run ids for
its own SSE stream, and `RunFinalize` is described as mirroring web's
`finalizeRunCompletion`. The finalization issues `PATCH /runs/{id}/status` with
`cancelled`, recorded as the user's own cancel, against a run that is streaming
in the other client. It fires on launch, on workspace refresh, after pin/unpin,
and after a title patch, over the eight most recent conversations — which is
exactly where the conversation just used on web will be. DA's `expected_from`
fencing protects the row from corruption; it does not protect the answer.

That destruction is a defect against `chat-run-recovery`, which already scopes
boot recovery to "a journal-tail run — one whose local transition log ends
non-terminal". It is fixed ahead of this proposal and is documented here as a
spec delta, not as new work.

**It cannot see the other client's work.** That is what this change adds. The
desktop already writes a complete live stream to DA: `local_executor` batches
`run.stream_delta`, `run.thinking_step` and `run.complete` records to
`POST /runs/{id}/events:batch` on a 250 ms cadence, deliberately separate from
its 16 ms paint cadence. So a desktop run is already followable by web. The
reverse is not: `DaChatSession::list_run_events(run_id, after_sequence)` is
implemented, cursor-filtered, and has no caller anywhere in the app. A run
started on web is invisible on the desktop until it terminates and the
conversation is reconciled.

The handover is therefore one-directional today, and the direction that is
missing is the one that also caused the destruction: the desktop had no way to
tell a live foreign run from a dead local one, so it treated both as dead.

## What Changes

- Follow an in-flight run this device does not own by polling
  `list_run_events` from the last sequence the projection holds, and project
  those events into the transcript the same way a locally driven run's events
  are projected.
- Present a followed run as live and not-ours: streaming text and thinking
  arrive as they do for a local run; Stop remains available, because a user
  cancelling their own run from either client is legitimate; retry and revise
  stay disabled until it reaches a terminal state.
- Stop following on terminal state, on conversation close, or on sign-out, and
  reconcile once on terminal so the durable message replaces the followed
  stream.
- Back the poll off when the window is not focused, and never poll when no
  foreign run is in flight.
- Never auto-cancel a run this device cannot prove it owns. (Already landed as
  the defect fix; stated here as the requirement it restores.)

## SOTA Basis

Session handover between a web client and a desktop client over one
server-owned conversation is the model ChatGPT and Claude present: the
conversation is server state, any client may attach to a generation in
progress, and no client terminates another's generation as a side effect of
opening. This change adopts the attach half of that model with the transport DA
actually exposes.

## Impact

- Affected specs: `chat-run-recovery` (MODIFIED — finalization scope),
  `cross-client-run-handover` (ADDED)
- Affected native code: `src-tauri/src/chat/service.rs` (follower lifecycle),
  a new follower module beside `chat/sync.rs`, `chat/da_repository.rs` (event
  page → projection events)
- Affected frontend: `src/features/chat/chatStore.ts`,
  `src/features/chat/useChat.ts`, `src/features/chat/runState.ts` — a run may
  now be live without this process owning its cancellation handle
- Affected contracts: none. `list_run_events` already exists on the client and
  the route already exists on DA.
- Persistence: no schema change. Followed events land in the existing
  `chat_runs`/`chat_messages` projection through existing projection events.
- Data Analyst repository: no change requested. See Non-Goals on SSE.

## Non-Goals

- Making the local projection authoritative. DA remains the source of truth;
  the follower is a reader of DA's durable replay log.
- Executing a foreign run locally, or taking over its execution. The client
  that started a run finishes it. The desktop observes.
- Adding an SSE or WebSocket route to DA. The contract exposes only
  `GET /runs/{id}/events`, so this change polls. If DA later adds a push
  transport, the follower swaps transport without changing the projection path
  — the design keeps that seam explicit.
- Approving a foreign run's tool call from the desktop. A run awaiting approval
  is followed and displayed; the approval stays with the client that owns the
  tool session.
- Changing how the desktop writes its own run events. It is already a
  conforming writer.
