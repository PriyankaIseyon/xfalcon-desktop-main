# Tasks

## 1. The store owns the answer for its whole life

- [x] 1.1 `ChatStoreSnapshot.settlingText` — the streamed answer after the
      terminal inference event, before the durable row.
- [x] 1.2 `reduceInferenceEvent`: `completed` hands `streamingText` over
      to `settlingText`; `failed` drops both.
- [x] 1.3 `mapConversationView`: `streamingText` and `settlingText` join
      `activity` and `liveThinking` under the single `keepsLiveRun`
      retirement.
- [x] 1.4 `beginNewConversation` clears it.

## 2. `useChat` stops guessing

- [x] 2.1 Delete the `settleRef` hold, its `baseCount` arithmetic, and
      the `runHasTools` null-ing.
- [x] 2.2 `liveText = streamingText || settlingText`; a tool-bearing run
      shows it in the card, a tool-free run in the transcript tail.
- [x] 2.3 Expose `liveText` for the card; `AppShell` reads it from
      `useChat` instead of the raw snapshot field.

## 3. Tests

- [x] 3.1 The gap is held.
- [x] 3.2 The durable row retires the hold, the activity feed, and the
      narration together.
- [x] 3.3 A dead run drops its partial text.
- [x] 3.4 A conversation switch does not carry a held answer across.

## 4. Verification

- [ ] 4.1 `vitest run`, `tsc -b`, `eslint`.
- [x] 4.2 Live: a tool-bearing run and a tool-free run each show their
      answer exactly once, through the settle.
