# Change: Ask OpenRouter for what the chat surface already knows how to show

## Why

The product assumes one provider — OpenRouter — and the latest OpenAI, Anthropic,
and Google models behind it. The adapter was written when that was not a safe
assumption, and three of its decisions no longer hold.

**Reasoning reaches one vendor out of three.** `requests_reasoning` tests for the
`anthropic/` prefix alone. The comment says why: OpenRouter forwards `reasoning`
to whatever provider it routes to, "this build offers every model the account can
reach", and a model that cannot reason may answer 400. That premise is gone. With
the model set fixed to the latest OpenAI, Anthropic, and Google models, all of
which reason, the guard now only withholds a feature the app has already built —
`chat-reasoning-panel`, `run-reasoning-activity`, and the reasoning retention cap
in `chatReduce` are all live, and a GPT-5 or Gemini answer lights none of them.

**A silently degraded route reads as a broken model.** OpenRouter's documented
default is to route to a provider even when that provider ignores a parameter of
the request. For `tools` that is not a degradation, it is a failure: the agent
loop asks for a tool call and gets prose, and the run ends having done nothing it
was asked to do. `provider.require_parameters` exists for exactly this and the
adapter does not send it.

**Anthropic caching is off.** Most providers on OpenRouter cache prompt prefixes
automatically. Anthropic does not — it requires an explicit `cache_control`
breakpoint — and this app sends a large system prompt on every turn of every
conversation. A cache read is 0.1x the price of a fresh read and lands sooner,
which is time to first token on every turn after the first.

## What Changes

- `requests_reasoning` covers the vendor namespaces the product actually ships
  against: `anthropic/`, `openai/`, `google/`, and `x-ai/`. The request shape
  stays `{"enabled": true}` for all of them — OpenRouter documents it as "enable
  with defaults" and translates per model, which is the only form that survives
  the `-latest` aliases this app carries.
- `provider: { require_parameters: true }` is sent **when the turn carries
  tools**, and only then. A dropped tool definition breaks the run; dropped
  reasoning only makes it quieter, and demanding a reasoning-capable endpoint
  could leave a model with no route at all.
- A `cache_control` breakpoint on the system message for `anthropic/` models,
  above a size floor. Other vendors cache without being asked and are left alone.
- The stale comments that justified the old behaviour are replaced by the reasons
  for the new behaviour.

## What This Change Deliberately Does Not Do

- **No `session_id`.** OpenRouter's sticky routing already keeps a conversation
  on one provider by account, model, and conversation; the explicit id is finer
  control over something that already works. It would need a conversation
  identifier threaded through `ProviderChatRequest` — 3 production sites and 26
  test fixtures — for a gain nothing here can measure. Revisit with a measurement,
  not with an assumption.
- **No `models` fallback chain.** Unchanged and still deliberate: the run records
  the model it chose, and answering from another one makes that record a lie.
- **No `usage: {include: true}` / `stream_options`.** Re-checked against the
  current docs: both remain deprecated and inert on OpenRouter, and usage rides
  the final SSE message unconditionally. The existing comment was right.
- **No `provider.sort`.** A latency or throughput preference is account-level
  policy, and hardcoding one here would silently override the account's own.

## Impact

- Affected specs: `openrouter-request-shape` (new)
- Affected code: `src-tauri/src/providers/openrouter.rs`
- No contract change, no `contracts.ts` regeneration.
- Depends on nothing. Independent of `refactor-chat-streaming-hot-path`, which
  touches the presentation path rather than the request.
