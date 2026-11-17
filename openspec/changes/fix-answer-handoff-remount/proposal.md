# Change: Do not rebuild a finished answer when its run settles

## Why

When a streamed answer finished, it was drawn again from scratch, so the response
looked as if it had been generated twice. Two things rebuilt the answer's rendered
markdown, and each was visible on its own:

1. The live row's key was derived from the run id. The run id clears when the run
   settles, a moment before the saved row lands, so the key changed from
   `stream-<run>` to `stream-active` and React replaced the row.
2. The live row lived in the tail and the saved row in the run's blocks, which are
   different elements in different places. When the saved row landed, the live
   row was unmounted and the saved row mounted, rebuilding every markdown block.

## What Changes

- Draw the newest run's answer with ONE element, `CurrentAnswer`, kept under one
  key by `MessageList`. The live text and the saved message are two sources for it.
  The tail keeps the work pill, thinking, approvals and notices.
- The element keeps the typewriter reveal across the handover. It takes the saved
  row's identity, and so its copy, feedback and regenerate actions, only once the
  whole answer is shown.
- `LiveRunTimeline` draws the work pill only. The stream benchmarks measure the
  answer through its new path.
- A notice or approval prompt now sits above the answer while it streams, the same
  place the saved transcript already put it.

Out of scope: when the next user message is sent, the previous answer moves into
the virtualized window and is mounted there. That is a separate, earlier turn and
is not part of this fix.

## Impact

- Affected specs: chat-run-activity (one requirement modified, one added).
- Affected code: `CurrentAnswer.tsx` (new), `MessageList.tsx`, `LiveRunTimeline.tsx`,
  `ChatSurface.tsx`, `benchmarks/chat/stream.bench.tsx`, and their tests.
