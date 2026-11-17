## 1. One element for the answer

- [x] 1.1 The answer's rendered element is the same, connected, and drawn once from
      the first token through the provider finishing, the run settling and the saved
      row landing, for a run with and without tools, with and without the typewriter
      reveal (`answerHandoff.test.tsx`).
- [x] 1.2 The existing handover contracts hold: no entrance replay, one ribbon per
      response, the accent on the newest answer, the answer outside every card
      (`MessageList.test.tsx`, `LiveRunTimeline.test.tsx`, `ChatSurface.test.tsx`).
- [x] 1.3 The streaming render budgets are unchanged: 60 fence renders over 80
      steps, one tool-card render, and the transcript overhead
      (`benchmarks/chat/stream.bench.tsx`).

## 2. Verification

- [ ] 2.1 Frontend, lint and type checks pass across the whole workspace before the
      next push.
- [ ] 2.2 A finished answer is checked by eye in the relaunched app, with and
      without tool calls.
