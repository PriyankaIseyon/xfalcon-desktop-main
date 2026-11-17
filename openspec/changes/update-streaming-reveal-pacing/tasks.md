## 1. Pacing

- [x] 1.1 The pacer eases its speed, follows the arrival rate, and measures time
      rather than frames (`revealPacer.test.ts`, `useSmoothStreamingText.test.tsx`:
      "ramps the speed up", "keeps the reveal speed independent of the display's
      frame rate", "does not jump ahead after a stalled frame").
- [x] 1.2 An uneven provider reveals in a narrow band, and a large burst is not a
      single block (`useSmoothStreamingText.test.tsx`: "smooths an uneven provider",
      "reveals a large burst as a fast typewriter", "clears a very large burst").
- [x] 1.3 Order, surrogate pairs, replacement, new run and reduced motion are
      preserved (`useSmoothStreamingText.test.tsx`).

## 2. Finishing

- [x] 2.1 The reveal drains after the provider finishes, within half a second, with
      no extra render at the terminal event (`useSmoothStreamingText.test.tsx`:
      "keeps revealing after the provider finishes").
- [x] 2.2 The live answer row keeps its streaming caret until the reveal is done
      (`LiveRunTimeline.test.tsx`: "finishes the reveal after settle").

## 3. Verification

- [ ] 3.1 Frontend, lint and type checks pass across the whole workspace before the
      next push.
- [ ] 3.2 A fast model (for example Haiku) is checked by eye in the relaunched app.
