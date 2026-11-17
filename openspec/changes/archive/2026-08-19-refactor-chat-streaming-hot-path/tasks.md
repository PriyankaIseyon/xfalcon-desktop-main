## 1. Core: make the stream progressive

- [x] 1.1 `CoalesceConfig` gains `max_coalesce_bytes`. Set it on
      `for_delta_coalescing()` and `for_tests()`. State the figure's reason in the
      doc comment, the way the file's existing comments do.
- [x] 1.2 `InferencePresentationSink` gains `pending_since: Option<Instant>` and a
      public `flush()`. `emit` forwards buffered text when `min_coalesce` has
      elapsed or `max_coalesce_bytes` is reached.
- [x] 1.3 `stream_turn` adds a presentation tick to its `select!` at
      `min_coalesce` that calls `presentation.flush()`. Leave the DA batch tick
      and its interval alone.
- [x] 1.4 Rewrite `inference_presentation_sink_coalesces_adjacent_text_deltas`.
      It asserted `forwarded.len() == 1`, which is the defect. Assert instead that
      adjacent deltas inside one interval coalesce AND that text is forwarded
      before the terminal event.
- [x] 1.5 New tests: the byte cap forwards early; an elapsed interval forwards on
      the next delta; `flush()` forwards a stalled buffer; sequences increase
      across consecutive coalesced forwards.
- [x] 1.6 Apply the same review to the other three sink construction sites
      (`engine.rs:131`, `engine.rs:366`, `turn_driver.rs:621`). The DA polling path
      emits a batch at a time, so confirm the per-emit clock covers it and record
      the finding either way.
- [x] 1.7 Verify on Windows: `cargo build --features custom-protocol`,
      `cargo test --workspace --all-features`,
      `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
      `cargo fmt --all -- --check`.

## 2. Store: selector subscriptions

- [x] 2.1 Add `useChatSelector<T>(select)` to `chatStore.ts` on top of the
      existing `subscribeChat`/`getChatSnapshot` pair.
- [x] 2.2 Add a test that every selector this change ships returns an identical
      value or reference for unchanged state. A selector that allocates loops
      forever under `useSyncExternalStore`, so this test is the guard rail.
- [x] 2.3 Split `useChat` into the cold surface (actions plus the fields that
      change per conversation) and a `useTranscript` hook for the fields that
      change per token.

## 3. Store: composer draft moves out

- [x] 3.1 New `src/features/chat/composerStore.ts` with its own listener set and
      a `useComposerDraft()` hook.
- [x] 3.2 Remove `composerDraft` from `ChatStoreSnapshot`, `initialState`,
      `setComposerDraft`, and every place that reads it. No alias, no shim.
- [x] 3.3 Update every call site in the same pass: `useChat.ts`, `AppShell.tsx`,
      and the tests that assert draft behaviour.
- [x] 3.4 Test: a keystroke notifies the composer store and does not notify a
      transcript subscriber.

## 4. Transcript: stable identity and bounded rebuild

- [x] 4.1 `useChat` stops synthesizing the in-flight row. It returns
      `snapshot.messages` unchanged plus the streamed text as its own value.
- [x] 4.2 `MessageList` takes `streamingText` and `streamRunId` and builds the
      tail item itself. The tail keeps the `stream-` id prefix, so the caret rule
      and `isDurableMessageId` are unchanged.
- [x] 4.3 `buildDisplayItems` takes index bounds and splits at `runStart`. Memoize
      the settled half on `messages`; rebuild only the run's half when reasoning or
      streamed text changes. Do not slice the transcript.
- [x] 4.4 Fix `AppShell`'s `chatFiles(chat.messages)` memo, which re-ran per frame
      only because the array identity changed.
- [x] 4.5 Wrap `MessageRow`, `AssistantMessage`, `UserMessage`, and
      `DurableToolTrail` in `memo`. Confirm every prop is a primitive or a
      store-owned reference, so the default comparison is correct.
- [x] 4.6 Tests: the durable array keeps its identity across a delta; a settled
      turn's row does not re-render when the tail grows.

## 5. Markdown: block splitting and memoization

- [x] 5.1 Declare `mdast-util-from-markdown`, `mdast-util-gfm`, and
      `micromark-extension-gfm` as direct dependencies, pinned to the versions
      `react-markdown` and `remark-gfm` already resolve.
- [x] 5.2 New `src/features/chat/richContent/splitMarkdown.ts` returning the exact
      source slice of each top-level node. Render the whole message as one block
      when it holds a top-level raw `html` node, or any reference or footnote
      definition.
- [x] 5.3 New `MarkdownBlock`, memoized on its source slice, rendering through the
      existing `REMARK_PLUGINS`/`REHYPE_PLUGINS`. `AssistantMessage` renders the
      block list.
- [x] 5.4 Confirm no base-index threading is needed: `htmlDocumentIndex` resolves
      an ordinal by content, and `saveable` is `null` for the whole stream. Add a
      test that a two-document answer keeps both Save actions pointing at the right
      ordinal after splitting.
- [x] 5.5 Test: a message holding each whole-message escape renders identically
      split and unsplit.
- [x] 5.6 Test: a settled fence's card does not render again when later text
      arrives.

## 6. Shell: `ChatPane` owns the hot subscription

- [x] 6.1 New `src/features/chat/ChatPane.tsx` holding the `useTranscript`
      subscription and rendering `ChatSurface`.
- [x] 6.2 `AppShell` renders `<ChatPane />` and keeps only the cold chat fields.
- [x] 6.3 Test: a streamed delta re-renders `ChatPane` and does not re-render
      `AppShell`.

## 7. Gates

- [x] 7.1 Promote `benchmarks/chat/stream.bench.tsx` to a budget. Assert the
      fence-card render count and the per-frame React time for both scenarios,
      with the numbers stated in the file.
- [x] 7.2 Add a core-side budget: for a fixed provider stream, assert the forwarded
      `text_delta` count is above one and below the delta count received. This is
      what stops a regression in either direction.
- [x] 7.3 `tsconfig.app.json` covers `benchmarks` and `tests`. Repair the
      `ChatStoreSnapshot` in `benchmarks/chat/storeApply.perf.test.ts`, which is
      missing five required fields.
- [x] 7.4 `vitest.config.ts` pins `pool: "threads"`.
- [x] 7.5 `benchmarks/chat/README.md` records every budget, its number, the
      measured baseline it replaced, and the statement that a jsdom timing is a
      relative signal and not a product service level.
- [x] 7.6 Confirm each budget fails when its number is lowered by a small margin.
      A budget that cannot fail is not a gate.

## 8. Verify

- [x] 8.1 `npm run typecheck`, `npm run lint`, `npm run format:check` clean.
- [x] 8.2 `npm test` passes with no dropped files. Check the reported file count
      against the baseline in `README.md`, and check the Errors line.
- [x] 8.3 Windows: `cargo test --workspace --all-features`, `cargo clippy` with
      `-D warnings`, and `cargo fmt --check` all clean.
- [x] 8.4 Record the new baseline numbers in `README.md`, replacing the ones this
      change measured.
- [x] 8.5 Live: run the app and confirm an answer from a non-reasoning model
      appears progressively rather than at once. This is the only check that
      proves the headline defect is gone, and it needs a machine with a provider
      key.
