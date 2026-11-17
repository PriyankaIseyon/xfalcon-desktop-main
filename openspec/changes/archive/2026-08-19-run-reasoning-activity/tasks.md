# Tasks — Run Reasoning Activity

## 1. Rust: final text = final turn only

- [x] 1.1 `turn_driver.rs`: finalize with `turn.text` (snapshot fallback
      only when the final turn streamed nothing); comment names the
      duplication/glue defect
- [x] 1.2 Agent test: multi-round run's finalized message text equals
      the final turn's text, not the concatenation
- [x] 1.3 `cargo test --lib` green

## 2. Store: live thinking segments

- [x] 2.1 `chatStore`: `liveThinking` snapshot field; capture on first
      admitted `requested` (text + beforeActivityIndex, clear
      streamingText); clear with activity + on terminal inference +
      run adoption + conversation switch/new/reset
- [x] 2.2 `chatReduce`: completed/failed also clear `liveThinking`
- [x] 2.3 Store/reducer tests: capture, parallel-request idempotence,
      clear sites

## 3. UI: shells + grouping + tail gating

- [x] 3.1 `buildDisplayItems`: steps model (thinking blocks stay inside;
      umbrella survives prose+calls; text-only closes) + tests
- [x] 3.2 `DurableToolTrail`: renders steps; thinking = inert quiet
      prose; label = call count
- [x] 3.3 `LiveRunActivity`: segments + streaming block inside, ordered;
      open-while-streaming / close-on-end (first-token collapse removed);
      elapsed label event-driven
- [x] 3.4 `useChat`: no stream tail row and no settle-hold once the run
      has tool activity
- [x] 3.5 `ChatSurface`/`AppShell`: thread `liveThinking` + streaming
      text into the card; drop the `answerStreaming` collapse prop
- [x] 3.6 Vitest suites green (store, MessageList, ToolActivityCard,
      ChatSurface)

## 4. Verify live

- [x] 4.1 Rebuilt (dist + exe). Live CDP drive blocked at the sign-in
      wall (the OAuth browser flow cannot run unattended; the session did
      not survive the app restart — pre-existing behavior worth its own
      look). Contract verified instead at every seam: Rust driver test
      (finalized content = final turn only, round prose preserved on its
      round), store segmentation tests, component tests (narration inside
      the live card and the durable umbrella, open-while-streaming /
      close-on-end), grouping tests (one umbrella, prose folded inside,
      final outside). 322 vitest + 1034 cargo green.
