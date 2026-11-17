# Tasks

Sequenced upstream-first, and the order is load-bearing: the reducer decides
what a reasoning fragment _is_ before the store decides how long to keep it,
and the store owns the field before any component reads it. Building the panel
first would leave a component subscribing to state that does not exist, and
would tempt a local `useState` in the panel — which is exactly the shape that
loses a run's reasoning on the next re-render of the transcript.

No contract work opens this list. `InferenceEvent::reasoning_summary` already
exists, is already in the `decl()` list, and is already asserted in the
event-type snapshot; nothing is regenerated and no permission or capability
entry is added.

## 1. Admit the event in the reducer

- [x] 1.1 `chatReduce.ts`: `reduceInferenceEvent` handles
      `reasoning_summary`, appending `event.summary` to `state.reasoning`. It
      stays behind the existing run-scoping guard, so a broadcast fragment from
      another conversation's run is dropped exactly as a foreign `text_delta` is.
- [x] 1.2 Give reasoning its **own** sequence ledger — a second map beside
      `lastInferenceSequenceByRun`, not the same one. Reasoning and text deltas
      are numbered from one run-wide counter, so a reasoning fragment admitted
      through the answer's ledger would advance it past the next answer delta and
      swallow answer text. Both ledgers are cleared on `completed`/`failed` and
      from `resetInferenceSequenceState`.
- [x] 1.3 Tests in `chatReduce.inference.test.ts`: a fragment appends; a
      replayed fragment appends once; a foreign run's fragment is ignored; and
      the regression that motivates the split ledger — reasoning at sequence N
      followed by answer text at N+1 keeps the answer text.

## 2. Hold it in the store

- [x] 2.1 `chatStore.ts`: `reasoning: string` and `reasoningTrimmed: boolean`
      on `ChatStoreSnapshot`, both in `initialState`, documented as live per-run
      state that the backend never replays — like `activity`, unlike `messages`.
- [x] 2.2 A retention cap in the shape of `MAX_ACTIVITY_EVENTS`:
      `MAX_REASONING_BYTES` (32 KiB), trimming from the **oldest** end so the text
      a reader is watching survives, and setting `reasoningTrimmed` once it has
      bitten. A long-lived window must not be able to grow this string without
      limit, and a panel that silently begins mid-sentence reads as a bug in the
      model rather than a bound in the app.
- [x] 2.3 `applyInference` routes a `reasoning_summary` through
      `setStateCoalesced`, on the same condition shape the `text_delta` route
      uses. It arrives at token cadence; a synchronous commit per fragment
      re-renders the whole transcript — markdown re-parse of the growing answer
      included — once per token, the cost `retire-live-run-activity` removed for
      text. Terminal events keep their immediate `setState`.
- [x] 2.4 The retirement exemption: `mapConversationView` does **not** clear
      `reasoning`/`reasoningTrimmed` with the other four live fields. They retire
      on the snapshot because the snapshot carries them; nothing in the snapshot
      carries reasoning, so clearing it there would delete the only copy that
      ever existed.
- [x] 2.5 The three sites that _do_ clear it, because the view has stopped
      being about that run: `adoptActiveRun` (a fresh run must not inherit the
      last run's thinking), `beginNewConversation`, `resetChatStore`. A
      conversation switch clears it through `mapConversationView`'s
      `sameConversation` arm — confirm that path, since the exemption above must
      not accidentally carry one conversation's reasoning into another.
- [x] 2.6 Tests in `chatStore.test.ts`: fragments accumulate in order; a burst
      inside one frame notifies subscribers once while `getChatSnapshot` reads
      exact values immediately; the cap drops the oldest and raises the flag; the
      durable snapshot retires the answer and the activity feed but **not** the
      reasoning; adopting a new run, switching conversation, and resetting each
      clear it.

## 3. The panel

- [x] 3.1 `src/features/chat/ReasoningPanel.tsx`, rendering through the
      existing `ToolActivityShell` disclosure — the same quiet card language as
      the tool umbrella, and no component library. Props are the run's reasoning,
      the trimmed flag, and whether the run is still working; it owns no fetch and
      no store subscription of its own.
- [x] 3.2 Open-state choreography, mirroring `LiveRunActivity` but keyed on
      the **first fragment** rather than the run's rising edge: opening on the
      rising edge would show an empty box on every run, and most runs in this
      build stream no reasoning at all. Open on the first fragment, stay open
      while the run is working, collapse on the falling edge, keep the header.
      `manualOpen ?? autoOpen`, so a reader's toggle wins for the rest of the run.
- [x] 3.3 Body renders inert, as `ThinkingBlock` does: `whitespace-pre-wrap`
      plain text in secondary ink, no markdown execution, no rich-content
      detection, no highlight pipeline. Chain-of-thought routinely contains
      half-finished fenced blocks, and executing it would mint a preview frame out
      of syntax the model has not finished writing.
- [x] 3.4 Not a live region: no `aria-live`, no `role="status"`. It is a named
      disclosure whose expanded state Radix already exposes. Announcing every
      reasoning token would bury the answer under the thinking. Any duration on
      the header is client-observed and advances with event or render activity,
      never a `setInterval` (frontend privilege boundary).
- [x] 3.5 The trimmed notice renders inside the panel, once, at the top — the
      end the text was dropped from.
- [x] 3.6 `ReasoningPanel.test.tsx`: absent with no reasoning; expanded on
      first content while working; collapsed-but-present after the run ends and
      reopenable; a manual collapse survives further fragments; an unterminated
      fenced block stays visible text and mints no frame; the trimmed notice
      appears only when the flag is set; no live region in the tree.

## 4. Place it in the transcript

- [x] 4.1 `ChatSurface.tsx`: `reasoning`, `reasoningTrimmed`, and the existing
      `runActive` feed a `<ReasoningPanel>` as the **first** element of
      `MessageList`'s `tail`, above the activity card. Reasoning precedes the
      tools it motivates and the answer it produces, and the response ribbon stays
      the last boundary before the answer, so it keeps meaning "the answer begins
      here".
- [x] 4.2 The panel carries neither accent mark — no accent rail, no accent
      rule — exactly as the tool umbrella carries neither. An accented panel of
      raw thinking directly above the real answer would send a reader scanning for
      the newest answer to the wrong text.
- [x] 4.3 The pill's condition gains "and no reasoning has arrived"
      (`AppShell.tsx`, where `thinking` is composed from `pending && !streaming`).
      Two live indicators for one run is the same same-thing-twice defect this
      transcript has been repaired for three times.
- [x] 4.4 `useChat.ts` exposes the panel's inputs from the snapshot, beside
      `liveText`/`activity`, so `AppShell` threads props and computes nothing.
- [x] 4.5 Tests in `ChatSurface.test.tsx`: a run with reasoning shows the
      panel and no pill; a run with neither shows the pill and no panel; a
      tool-free reasoning run shows the panel with no activity card; ordering is
      panel → activity card → notice.

## 5. Pin the accepted limitation in Rust

Tests only — no production Rust changes. These exist so the scoping decision
in the proposal stays a decision instead of decaying into a bug report.

- [x] 5.1 `providers/anthropic.rs`: a test asserting `build_request_body`
      carries no `thinking` key, in the shape of openrouter.rs's
      `never_requests_reasoning_it_cannot_echo_back`. Its comment records _why_
      the panel is therefore absent on Anthropic direct, so the next reader finds
      the decision at the site rather than only in this change.
- [x] 5.2 `agent/local_executor.rs`: a test that a `ReasoningDelta` becomes an
      `InferenceEvent::reasoning_summary` and never reaches
      `RunOutputBuilder::push_text` — reasoning must not be able to become answer
      bytes on the durable record.

## 6. Verification

- [x] 6.1 `vitest run`, `tsc -b`, `npx eslint` clean; `src/security/frontend-boundary.test.ts`
      still passes (the panel adds no `fetch`, no storage, no `setInterval`).
- [x] 6.2 `cargo test --lib` and `cargo test -p xfalcon-contracts` green;
      re-run the contracts test **without** `XFALCON_WRITE_CONTRACTS` to confirm
      no generated TypeScript drift — this change should produce none.
- [x] 6.3 `cargo clippy --all-targets -- -D warnings`.
- [x] 6.4 Live, on an OpenRouter credential and an `anthropic/` model: a run
      requested reasoning, the fragments arrived, and the card rendered them.
      This is what showed sections 3 and 4 to be the wrong shape — see section 7.
- [x] 6.5 Live, on an Anthropic credential: no reasoning, no empty header, and
      the thinking pill still reports the wait — the accepted limitation,
      confirmed rather than assumed.

## 7. Rework: one card, not two (2026-08-10)

Sections 3 and 4 built reasoning as a panel above the activity card. The first
live run showed two cards for one run, with a round's thinking above the tool
row it was reacting to. What follows replaces that panel; the proposal's second
amendment records why the original argument was wrong.

- [x] 7.1 One ordering axis. `LiveSegment.afterCallCount` — the tool rows that
      existed at capture — replaces the live card's private `beforeActivityIndex`,
      and now carries narration as well as reasoning. The live card counts rows
      drawn and the durable trail counts calls emitted, so both place a segment
      identically; the previous axis was an activity-**event** index, which the
      durable trail has no way to compute.
- [x] 7.2 `chatReduce.ts`: `reasoning` becomes `LiveSegment[]`. A fragment
      extends the last segment while no row has appeared since it began, and opens
      a new one once one has. `toolRowCount` counts distinct invocations (one
      invocation emits several events but draws one row), memoized on the activity
      array's identity so a token-cadence fragment does not pay for a rescan. The
      cap sheds whole spent segments before cutting into a survivor, and still
      never splits a surrogate pair.
- [x] 7.3 `ToolActivityCard.tsx`: one merged body. `ActivityStep` gains a
      `reasoning` kind; `ReasoningBlock` carries a "Thinking" caption so raw
      thinking is not read as something the model said; `LiveRunActivity` takes
      `narration`/`reasoning`/`reasoningTrimmed` and interleaves both segment lists
      with the rows, reasoning first where the positions tie. A run with no rows
      labels itself "Reasoning" rather than "0 tool calls".
- [x] 7.4 `MessageList.tsx`: the durable handover. `buildDisplayItems` takes the
      run's reasoning and places it among the calls on the same axis, restricted to
      the turns after the last user message so it cannot reach an earlier run's
      trail, and opens a group for a run that reasoned and called nothing.
- [x] 7.5 `ChatSurface.tsx`: exactly one owner at a time. The live card holds
      reasoning while `activity` is non-empty or the run is still in flight; the
      durable trail holds it otherwise. Both would draw it twice; neither would
      lose it at the snapshot.
- [x] 7.6 `ReasoningPanel.tsx` and its test are deleted rather than deprecated.
      Its choreography was already `LiveRunActivity`'s, so nothing was
      reimplemented.
- [x] 7.7 Tests for the behaviour the rework exists for: a fragment after a row
      opens a new segment (`chatStore.test.ts`); the live card interleaves
      reasoning, narration and rows in that order and labels a tool-free run
      (`ToolActivityCard.test.tsx`); the durable trail carries a settled run's
      reasoning among its calls, keeps it off an earlier run's trail, and gives a
      tool-free run its own group (`MessageList.test.tsx`); the surface shows one
      card and orders reasoning around the row it happened between
      (`ChatSurface.test.tsx`).
- [x] 7.8 Gates re-run: `vitest run`, `tsc -b`, `eslint`, `prettier --check`.
