# Change: Smooth the streamed answer into a steady typewriter reveal

## Why

A fast model delivers its answer in uneven clumps: a burst, a pause of a few
frames, another burst. The reveal ratcheted its per-frame step up to the largest
burst it had seen and held it until the backlog was gone, so clumps painted as
blocks of text (up to 70 characters in one frame after a 400 ms stall, in a
simulation of a 700 characters-per-second stream). When the provider finished,
whatever was still unrevealed was flushed in one frame.

## What Changes

- Pace the reveal with a proportional controller on the unrevealed backlog. Text
  is shown a short, fixed time after it arrives (0.2 s), so the reveal speed
  follows the arrival rate of whatever model is answering. The speed eases between
  frames and is measured in time, so 60 Hz and 120 Hz displays reveal alike.
- A backlog above the typewriter ceiling still scales the speed, so a very large
  burst clears quickly.
- When the provider finishes, drain the remaining text in a short bounded time
  instead of flushing it, and keep the streaming presentation (caret, in-progress
  artifacts, disabled feedback) until the whole answer is on screen.
- Reasoning, tool activity, the store, and the IPC cadence do not change.

Reveal speed is not forced to be the same for every model. Holding a fast model
to a slow model's speed only builds a backlog that either lags the real answer or
has to be dumped when the answer ends (measured: a 600 to 720 characters-per-second
ceiling left about 600 characters unrevealed, 0.7 s behind, when a 700
characters-per-second stream finished).

## Impact

- Affected specs: chat-streaming-performance (one requirement modified).
- Affected code: `src/features/chat/revealPacer.ts` (new),
  `useSmoothStreamingText.ts`, `LiveRunTimeline.tsx`, and their tests.
