# Design: cross-client run handover

## Context

Two clients, one source of truth. DA owns conversations, messages, runs, and a
durable per-run replay log (`xf_chat.run_events`, ordered by `sequence_no`).
xFalcon Web runs its own engine server-side. The desktop runs inference
in-process and streams the result back into that same replay log.

The desktop is already a conforming **writer** and not yet a **reader**.

## Decision 1: ownership is what the local journal says

The discriminator between "a dead process on this device left this run" and
"another client is driving this run" is whether this device's `RunJournalStore`
holds any transition for it. Three states, not two:

| This device drives it | Local journal exists | Meaning                  | Boot recovery may          |
| --------------------- | -------------------- | ------------------------ | -------------------------- |
| yes                   | —                    | streaming here now       | leave alone                |
| no                    | yes                  | this device abandoned it | finalize                   |
| no                    | no                   | another client's run     | **follow, never finalize** |

The third row is the one that did not exist before. It is also exactly the
scope `chat-run-recovery` already stated ("a journal-tail run — one whose local
transition log ends non-terminal"), so the fix restores the spec rather than
widening it.

Rejected: a client/device id on the DA run row. It would be a contract change
to DA and to web, and it answers a question the local journal already answers
without a round trip. Revisit only if the journal proves insufficient — for
instance if a wiped local profile must still reclaim its own orphans.

## Decision 2: poll, because DA exposes no push

`GET /api/v1/chat/runs/{run_id}/events` with a `sequence_no` cursor is the only
run-event read route in the DA contract. There is no SSE or WebSocket route.
`RunCreate`'s doc comment mentions an SSE stream, but that is web's own
in-process stream to its own browser, not an endpoint.

So the follower polls. Cadence, chosen against the 250 ms cadence the desktop
writer already uses so a follower cannot meaningfully outrun the producer:

| Condition                                    | Interval          |
| -------------------------------------------- | ----------------- |
| Followed conversation open, window focused   | 500 ms            |
| Followed conversation open, window unfocused | 2 s               |
| No foreign run in flight                     | no polling at all |

The follower stops on terminal state, conversation close, account change, and
sign-out. It is not a background sync: nothing polls for a conversation the
reader is not looking at.

**Transport seam.** The follower takes a `RunEventSource` with one method —
"events after this sequence" — so an SSE implementation can replace the polling
one without touching projection or presentation. This is the one indirection
this change adds, and it exists because the polling choice is forced by DA's
current surface rather than preferred.

## Decision 3: the follower writes through the existing projection events

A followed `run.stream_delta` becomes the same projection event a locally
executed delta produces. Two reasons: the transcript then cannot tell a
followed run from a local one, which is the product requirement; and the
reconcile, title, and recovery paths keep working on one shape of history.

Consequence to respect: `chatReduce` currently treats "this run is live" and
"this process can cancel it" as one fact. They separate here. A followed run is
live and cancellable **through DA** but has no local `run_cancels` entry, and
`drives_run` must keep answering `false` for it — that answer is what keeps boot
recovery off it.

## Decision 4: Stop stays available, auto-cancel never comes back

A person cancelling their own run from whichever client is in front of them is
legitimate, and `cancel_run` is already fenced server-side. What is removed
permanently is the desktop deciding on its own that a run it does not recognise
should die.

If a foreign run stalls — followed, no new events, no terminal state — the
desktop reports it as stalled and offers Stop. It does not cancel on the
reader's behalf. That trades the old "recovery banner forever" failure for a
visible stalled run the user can act on, which is the correct side to err on
when the row belongs to someone else's session.

## Risks

- **Orphan runs live longer.** A run created by a desktop process that died
  before writing its first journal entry now looks foreign and is left alone.
  It will show as stalled rather than being silently cancelled. Reaping
  genuinely abandoned rows is DA's job as the source of truth; the desktop
  guessing is what this change removes. If DA has no reaper, that is a DA
  issue to raise, and it is strictly better than the current behaviour of
  guessing wrong in the destructive direction.
- **Poll cost.** Bounded by "only while a foreign run is in flight in the open
  conversation", which is rare and short. Worst realistic case is one request
  per 500 ms for the length of one answer.
- **Duplicate projection on transition.** When a followed run terminates, the
  reconcile that fetches the durable assistant message must replace the
  followed stream rather than append beside it. Same class of problem the local
  path already solves at finalize; the follower reuses that path and is tested
  for it.
