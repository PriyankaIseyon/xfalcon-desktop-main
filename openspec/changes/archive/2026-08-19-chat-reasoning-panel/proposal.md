# Chat Reasoning Panel

## Why

Some models narrate their own thinking before they answer. Desktop already
carries that narration from the provider socket to the renderer — and then
throws it away.

The Rust half is complete and was built deliberately. `anthropic.rs` turns a
`thinking_delta` content block into `ProviderStreamEvent::ReasoningDelta`;
`openai_compat.rs` does the same for `delta.reasoning`, the wire OpenRouter
and the OpenAI-compatible adapters share, and its comment records that
reasoning is "passed through as its own event so the run record can keep
them apart". `local_executor.rs::inference_event_for` maps every one of
those onto `InferenceEvent::ReasoningSummary`; `da_events.rs` does the same
for a `run.thinking_step` record on the observed-run path;
`InferencePresentationSink` forwards them unchanged, because only text
deltas coalesce. Each fragment lands on
`xfalcon://chat-inference-event` with a run-wide sequence number, and
`stream_turn` is careful never to let it become answer text.

The renderer drops it. `reduceInferenceEvent` handles `text_delta`,
`completed`, and `failed`, then returns `{}` — so the one event that says
what the model is doing while the reader waits is the one event the store
ignores. What the reader gets instead is `ThinkingPill`: three dots and the
word "Thinking…", for as long as the model thinks.

## What Changes

- **The store keeps the run's reasoning, split where tool rows interrupted
  it.** `reduceInferenceEvent` admits `reasoning_summary` and appends its
  fragment to the current segment, opening a new one whenever a tool row has
  appeared since the last fragment. Fragments are ordered and de-duplicated on
  their **own** sequence ledger: reasoning and text share one run-wide counter,
  so folding reasoning into the answer's ledger would let a replayed reasoning
  fragment advance that ledger and silently swallow answer text.
- **Reasoning rides the coalesced commit path**, as text deltas do. It
  arrives at token cadence; committing synchronously per fragment would
  re-render the whole transcript — markdown re-parse of the growing answer
  included — once per token, which is the exact cost
  `retire-live-run-activity` removed for text.
- **One card per run, holding everything it thought and did, in order.**
  Reasoning, each round's narration and each tool row render inside the single
  `ToolActivityShell` disclosure the run already had. A stretch of thinking that
  followed a tool result sits below that result's row; one that preceded it sits
  above.
- **Both surfaces order on one axis: the tool rows that preceded a segment.**
  The live card counts the rows it has drawn and the durable trail counts the
  calls it has emitted, so a segment captured after N rows lands after N in
  either. That is what lets a run read identically while it streams and after it
  settles, rather than each surface re-deriving position from state only it has.
- **The card is the run's account of itself, not a tool inventory.** A run that
  reasoned and called nothing still gets one, named "Reasoning" rather than
  labelled with a count of calls that never happened.
- **Reasoning is handed to the durable umbrella when the live card retires.**
  Nothing in the durable snapshot carries reasoning, so the live card cannot
  simply drop it and the snapshot cannot restore it. Exactly one surface owns it
  at a time — the live card while the run is on screen as a live run, the
  umbrella afterwards — because both holding it would draw the same thinking
  twice and neither would lose it at the handover.
- **The card supersedes the thinking pill.** Once reasoning is on screen the
  generic pill is gone — two live indicators for one run is the same
  same-thing-twice defect the transcript has already been fixed for three
  times.
- **Reasoning renders inert and labelled.** Plain text in secondary ink, no
  markdown execution, no preview or table cards, no highlight pipeline —
  chain-of-thought routinely contains half-finished fenced blocks, and executing
  it would mint preview frames out of syntax the model has not finished writing.
  It carries a "Thinking" caption because it shares a body with round narration,
  and narration is answer text the turn keeps durably while this is neither.
- **The card is not an assistive-technology live region.** It is a named
  disclosure whose expanded state is exposed; announcing every reasoning
  token would bury the answer under the thinking.
- **Reasoning is the one live field the durable snapshot does not retire.**
  Every other live surface clears when the durable record lands, because the
  record carries the same content. There is no durable reasoning, so
  retiring it there would delete the only copy that ever existed — the same
  reason a run whose snapshot never arrives keeps its live card. It clears
  when the view stops being about that run: a new run is adopted, the
  conversation changes, or the store resets.
- **Two Rust tests pin the accepted limitation** below so it stays a
  decision: `anthropic.rs` sends no `thinking` key, and a `ReasoningDelta`
  becomes a `ReasoningSummary` event and never answer text.

## Capabilities

### Added Capabilities

- `chat-reasoning-panel`: accumulate a run's streamed reasoning and show it in
  the run's own card, in the order it happened relative to the run's tool rows.

### Modified Capabilities

- `chat-run-activity`: reasoning is exempt from the one-signal retirement, and
  is handed from the live card to the durable umbrella at that moment rather
  than living on a surface of its own.
- `chat-thread-layout`: the card's place in the transcript — no response
  ribbon, neither accent mark, and it precedes the answer it explains.

## Impact

- Changed: `src/features/chat/chatReduce.ts` (admit `reasoning_summary`, own
  sequence ledger, segment the run's reasoning by tool-row count),
  `chatStore.ts` (`reasoning` segments, byte cap, coalesced route, clear
  sites, the retirement exemption in `mapConversationView`, narration captured
  on the shared axis), `ToolActivityCard.tsx` (one merged body, the reasoning
  step kind, the zero-call label), `MessageList.tsx` (the durable handover),
  `useChat.ts` (the card's inputs, the pill's condition), `ChatSurface.tsx`
  (which surface owns reasoning right now), `src/app/AppShell.tsx` (threading).
- Rust: tests only — `src-tauri/src/providers/anthropic.rs`,
  `src-tauri/src/agent/local_executor.rs`.
- No contract change: `InferenceEvent::ReasoningSummary` already exists, is
  already registered in the `decl()` list, and is already asserted in the
  event-type snapshot. Nothing is regenerated.
- No storage change, no new Tauri command, no new capability or permission
  entry.

## Amendment (2026-08-10): the request side is now in scope

The scoping below was written when nothing requested reasoning. That changed:
this build targets Anthropic models reached through an OpenRouter key, so the
adapter now sends `reasoning: {"enabled": true}` for `anthropic/` model ids
only, and `openai_compat` merges the streamed `reasoning_details` fragments per
`index` and echoes them back verbatim on the assistant turn.

Three constraints shaped that and are pinned by tests:

- **`enabled`, not `max_tokens`.** OpenRouter documents `reasoning.max_tokens`
  for Anthropic, which maps to `thinking: {type: "enabled", budget_tokens: N}`
  — deprecated on Claude 4.6 and rejected with a 400 on 4.7 and later. The
  model ids here are `-latest` aliases, so `enabled` (which OpenRouter
  normalizes per model, choosing adaptive thinking where required) is the only
  version-stable shape.
- **Anthropic only.** OpenRouter forwards the key to whatever it routes to, and
  a model that does not reason answers 400/500 rather than ignoring it.
- **Echoing is not optional.** A model that reasons over tool results rejects a
  follow-up whose thinking was rebuilt, reordered, or dropped, so the merge is
  by `index` and the replay is a copy.

The per-turn ceiling is gone rather than raised. `max_tokens` is optional on the
OpenAI-compatible wire, so it is now omitted and the provider applies the
model's own maximum — the only value that is correct for every model behind an
OpenRouter key, and one that cannot truncate an answer whose thinking shared its
budget. Inflating the old constant would have been worse than omitting it:
Anthropic rejects a `max_tokens` above the model's own maximum, and that
maximum differs per model. The wires that require the field (Anthropic direct,
Google) fall back to `REQUIRED_MAX_OUTPUT_TOKENS`, which holds their previous
value; neither requests reasoning, so nothing there changed. `RunOutputBuilder`'s
256 KiB run-wide budget remains the bound that actually protects the user.

Everything below still holds for Anthropic _direct_, which continues to request
nothing.

## Amendment (2026-08-10): one card, not two

The first cut gave reasoning a panel of its own above the live activity card.
Seen against a real OpenRouter run it was wrong on both counts the original
reasoning claimed to protect.

It argued that a reasoning stream is continuous prose and that cutting it at
tool boundaries would split it mid-sentence. It does not: a round's thinking
ends when the model stops thinking and calls something, and the next round's is
a new thought about the result. What the single panel actually produced was a
later round's thinking rendered **above** the tool row it was reacting to,
because the panel accumulated in one place while rows arrived beside it — the
one ordering the run never had.

It also argued that one panel spares the reader hunting for the live one. Two
cards is what made them hunt. The run is one sequence and now renders as one.

The panel is deleted rather than deprecated: `ReasoningPanel.tsx` and its test
are gone, and its choreography (open on the run's rising edge, close on the
falling edge, a manual toggle winning for the rest of the run) was already the
activity card's, so nothing was reimplemented.

One property the panel had is kept, and it is what makes this more than a move:
reasoning outlives the durable snapshot, because nothing persists it. The live
card retires on that snapshot like every other live surface, so the reasoning is
handed to the durable umbrella in the same positions — which is why both
surfaces now order on the same axis (`afterCallCount`) instead of the live
card's private activity-event index.

## Not in this change

**No request-side reasoning parameter for providers other than OpenRouter →
Anthropic.** Those requests ask for nothing, and this change adds nothing:

- `anthropic.rs::build_request_body` sends `model`, `messages`,
  `max_tokens`, and `stream`, plus `system` when there is one and `tools`
  when there are any. There is no `thinking` block — so Anthropic never
  streams a `thinking_delta`, and the parser at the other end of that path
  never fires.
- `openai.rs` shares that same request builder. `google.rs` neither requests
  nor parses reasoning at all.

**The accepted consequence, stated plainly: the card renders whatever reasoning
arrives and none otherwise, which means a reader on an Anthropic _direct_
credential never sees any.** That is the correct outcome for this change: a
card that opened empty because nothing was ever requested would be worse than
none, and the request decision was made on its own merits for the one route
this build actually uses.

**Why enabling requests elsewhere is a product decision and not a follow-up
refactor.** What these providers stream is raw chain-of-thought, and the
tree already says so in three places even though the type name does not.
`ReasoningSummary::provider_designated` performs exactly one check — a 4 KiB
length bound — on a string that arrived as
`ProviderStreamEvent::ReasoningDelta`; nothing summarizes it and no provider
designated it. `ModelCapabilitiesView.reasoning_summary` is hardcoded
`false` at the DA boundary, with the comment "DA does not identify a
provider-designated user-safe summary" and a test asserting that "raw
reasoning flags do not prove a user-safe reasoning summary". And the
contract guard forbids the token `reasoning_delta` from appearing anywhere
in the generated `InferenceEvent` declaration — while `reasoning_delta` is
precisely what fills the field it guards. So turning requests on for another
route means deciding to expose chain-of-thought there, and designing the
per-model gating and the echo-back state its tool loop needs.

**No persistence.** Reasoning is never written to the encrypted local
projection and does not appear in `ChatConversationView`, so it lives
exactly as long as the conversation stays on screen. Reopening a
conversation shows its answers and its tool umbrellas and no reasoning.
Keeping chain-of-thought at rest is a retention decision of its own.

**A fragment over the contract's bound is still dropped silently.**
`inference_event_for` maps a `ReasoningDelta` through
`ReasoningSummary::provider_designated(...).ok()`, so one fragment larger
than 4 KiB becomes no event at all, and the card cannot tell that gap from
a provider that fell quiet. Providers stream reasoning in small deltas, so
this is rare rather than theoretical.

**No per-model toggle and no "show reasoning" preference.** A setting would
advertise a control over routing behaviour the app does not yet have.
