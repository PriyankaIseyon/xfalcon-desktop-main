# Tasks

## 1. The reasoning switch becomes a preference

- [x] 1.1 `crates/contracts/src/settings.rs`: add `show_reasoning: bool` to
      `AppearanceSettings`, with the default NAMED
      (`reasoning_shown_by_default`) rather than derived — `bool::default()` is
      `false`, which would have hidden reasoning on every install that already
      had a settings document. Update `new` and every call site.
- [x] 1.2 Regenerate `src/lib/generated/contracts.ts`
      (`WSLENV=XFALCON_WRITE_CONTRACTS XFALCON_WRITE_CONTRACTS=1 cargo.exe test
-p xfalcon-contracts`), then re-run without the variable to confirm no
      drift.
- [x] 1.3 `settings/tests.rs`: a document written before the preference existed
      reads back with reasoning shown; an explicit `false` reads back as false.
- [x] 1.4 `app/appearance.ts` becomes the single source of truth — module store
      over `useSyncExternalStore`, installed once at boot, with
      `useShowReasoning` returning the primitive so a reader of that one
      preference does not wake for a theme change.
- [x] 1.5 `AppearanceSection` reads the store instead of loading its own copy,
      and gains the toggle. `Switch` learns `aria-labelledby` so a settings row
      can name it from the label at the other end of the line.
- [x] 1.6 `activityDisclosure.ts` drops `SHOW_REASONING` and says why the two
      remaining flags are build decisions and this one is not.

## 2. Store: name the content for what it is

- [x] 2.1 Rename `ChatStoreSnapshot.liveThinking` → `roundProse`, and its doc to
      say it is answer prose the round keeps durably — not thinking.
- [x] 2.2 `chatReduce` and `chatStore.applyToolInvocation` follow the rename.
      The capture rule itself does not change: the round's prose is frozen at
      the round's first `requested` and `streamingText` resets.
- [x] 2.3 `benchmarks/chat/storeApply.perf.test.ts`'s snapshot literal follows
      it too, so the typechecked bench keeps matching the store.

## 3. Prose renders as prose

- [x] 3.1 Delete `ThinkingBlock`, `ActivityStep`, and `LiveRunActivity`'s
      `liveText`/`narration`/`reasoning` props. `DurableToolTrail` takes calls.
- [x] 3.2 `ReasoningDisclosure`: its own `ToolActivityShell`, labelled
      "Thinking…" while the run is active and "Thought process" once it is not,
      collapsed by default.
- [x] 3.3 `buildDisplayItems`: an assistant turn with tool calls contributes its
      prose as a `message` item that CLOSES the open group; reasoning becomes a
      `reasoning` item at its `afterCallCount` position, closing the group too.
- [x] 3.4 `estimateItemHeight` gives the reasoning item the collapsed-card
      constant, for the same reason the tool group has one.

## 4. One ribbon per response

- [x] 4.1 `MessageRow` takes `showRibbon`.
- [x] 4.2 `buildDisplayItems` marks the first assistant prose item after a user
      turn, maintained incrementally as it walks.
- [x] 4.3 `runSurface.responseAlreadyOpen` answers the same question for the
      live timeline, which builds from live state and cannot see the walk. Held
      together by a table-driven test across every mix of durable turns.

## 5. The live run renders in the tail

- [x] 5.1 New `LiveRunTimeline.tsx`: the in-flight run as an ordered block
      sequence from `activity`, `roundProse`, `reasoning` and the in-flight
      text, grouping consecutive tool rows into one card and closing the group
      at every prose row and reasoning disclosure.
- [x] 5.2 `MessageList`: `streamText`, `streamRunId`, `streamItem`, `itemAt` and
      `rowCount` all removed; the virtualizer counts `items` only.
- [x] 5.3 `scrollToBottom` addresses the scroll element directly, because the
      last virtual row is no longer the last thing in the flow.
- [x] 5.4 `ChatSurface` renders the timeline as the tail;
      `runSurface.liveRunHasContent` decides whether it draws anything and
      `ThinkingPill` fills the place when it does not.
- [x] 5.5 `useTranscript` drops `showsLiveTextInCard`; `liveText` is one value,
      always for the transcript.
- [x] 5.6 `ChatPane`'s pill condition is `pending` — whether anything is on
      screen is the surface's own predicate, asked once.
- [x] 5.7 `ChatExportDialog` exports the round prose too. It used to miss the
      in-flight answer entirely for a tool-bearing run, because `streamText` was
      empty for exactly that case.
- [x] 5.8 Remove `aria-live` from the transcript's scroll container — it made
      every virtualization mount an announcement — and put it on the arriving
      prose row instead.

## 6. Tests

- [x] 6.1 `activityDisclosure.test.tsx` rewritten for the shipped surface:
      reasoning IS reachable behind its disclosure; with the preference off no
      surface draws it and none draws an empty box; payloads, error text and
      step summaries are still absent from the DOM though present in props.
- [x] 6.2 `MessageList.test.tsx`: prose-as-prose above its calls, the split when
      prose lands between rounds, one ribbon for a two-prose response, reasoning
      as its own disclosure in order, the umbrella expanding onto tool rows
      only.
- [x] 6.3 New `LiveRunTimeline.test.tsx`: the ordering, the empty predicate, the
      contiguity rule, markdown-not-card-text, the single live region, the
      ribbon rules — and THE SETTLE, asserted as an equivalence between the live
      sequence and the durable one.
- [x] 6.4 `ChatSurface.test.tsx`, `ToolActivityCard.test.tsx`,
      `chatSelectors.test.tsx`, `AppearanceSection.test.tsx` follow the rename
      and the new props.
- [x] 6.5 `benchmarks/chat/stream.bench.tsx` drives the new shape, and its
      transcript-overhead scenario is isolated properly now that the in-flight
      row is always drawn.
- [x] 6.6 A regression test for each audit defect: answer never inside a card,
      round prose never withdrawn, no empty card, one ribbon per response.

## 7. Verify

- [x] 7.1 `npm run typecheck` (both projects), `npm run lint`,
      `npm run format:check`, `npm test` — 72 files, 1035 tests.
- [x] 7.2 `cargo.exe test --lib` (1291), `clippy --all-targets -D warnings`,
      `fmt --check`.
- [x] 7.3 Budgets: 60 fence renders (unchanged, and now identical at 0 and 500
      prior turns), transcript overhead 74 ms at 500 turns.
- [x] 7.4 Live: a narrated tool run reads prose → tools → answer with no text
      moving, and a reasoning model shows a collapsed thinking row instead of an
      empty card. Needs a machine with a provider key.

## 8. Bug fixes shipped alongside (no spec delta — restore intended behaviour)

- [x] 8.1 `providers/http.rs`: decode the SSE body incrementally.
      `String::from_utf8_lossy` per chunk replaced any multi-byte character
      split across a chunk boundary with U+FFFD — visible corruption in any
      answer containing CJK or emoji. Extracted `SseLineReader` so a test can
      choose the boundary the network otherwise chooses.
- [x] 8.2 `providers/http.rs`: an idle `read_timeout` (90 s) replaces the 180 s
      TOTAL request timeout, and the total rises to the run's own wall clock
      (600 s). A turn that legitimately streamed for more than three minutes was
      being killed mid-answer.
- [x] 8.3 `agent/stream.rs`: coalesce reasoning on the same bounds as text. Every
      fragment was one IPC hop to the webview, at token cadence, now that
      reasoning is requested from every shipped vendor — and a fragment the
      contract refused was silently dropped by `.ok()`.
- [x] 8.4 `providers/openai_compat.rs`: correct the `parse_event` comment. It
      claimed reasoning is never requested and `reasoning_details` are not
      carried back; both have been false since
      `add-openrouter-advanced-streaming`.

## 9. The markdown split stops re-reading what settled

Found by measurement during the re-audit, not by inspection: parsing the whole
message per token cost 261 ms for a 3.7 KB answer in 80 steps, 1090 ms for
12.6 KB in 260, and 4521 ms for 25.4 KB in 520 — quadratic, and by 25 KB the
dominant per-frame cost of a streaming answer.

- [x] 9.1 `splitMarkdown(source, previous?)` carries its last result forward and
      re-parses only the growing tail, refusing reuse unless the previous parse
      produced real boundaries, the new source extends the old one, and the tail
      begins at a line start — an indented code block's node can start after its
      indentation, and a slice taken there parses as a paragraph.
- [x] 9.2 `AssistantMessage` holds the cache in a ref, because the derivation
      depends on its own last result rather than on `text` alone.
- [x] 9.3 A differential test over EVERY prefix of 21 corpora — indented code,
      loose lists, late setext headings, definitions, raw HTML, CRLF, multibyte —
      asserting the incremental blocks equal the whole-parse blocks. The
      incremental path exists only for as long as this passes.
- [x] 9.4 `benchmarks/chat/split.bench.ts`: a RATIO gate (cost per byte must stay
      flat as the answer grows fourfold) rather than a duration, plus a recorded
      comparison against the whole-message split. Measured: 25 KB over 520 steps
      went from 4077 ms to 93 ms, and cost per byte from 4.3x growth to 0.41x.
