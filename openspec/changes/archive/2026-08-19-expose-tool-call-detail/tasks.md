# Tasks — Expose Tool Call Detail

> GATE RESOLVED: product sign-off recorded 2026-08-05, in-session (jeevan.tm):
> "reverse the stance and implement whatever xf web does, full details."
> Approach pivoted at implementation from the drafted lazy Tauri command to
> INLINE projection (web parity; the conversation rows already carry both
> payloads — see investigation 1.1). The original command-based task list is
> superseded by the as-implemented list below.

## 0. Decision gate

- [x] 0.1 Sign-off recorded (above). The narrowed stance is documented on
      `ChatToolCallView` (`crates/contracts/src/chat.rs`): payloads project
      as capped, display-only, inert text; the execution log stays the
      authoritative uncapped record.

## 1. Investigation (design D1)

- [x] 1.1 Source of truth resolved: tool rounds are durable IN THE
      CONVERSATION (`xf_chat.chats`) — assistant turn carries `tool_calls`
      (id + function.name + function.arguments), one `role="tool"` row per
      answer (content = result), written identically by desktop
      (`record_tool_round`) and web. The bootstrap the app already fetches
      on open contains everything; no new endpoint, no OpenAPI bump, no
      lazy command needed. (`xf_chat.tool_calls` remains the execution log
      only.)
- [x] 1.2 Cap set at 32 KiB per payload
      (`TOOL_PAYLOAD_PROJECTION_CAP_BYTES`), enforced in
      `ToolCallRef::with_payloads` (char-boundary cut + flag), re-applied on
      snapshot deserialize so the cap is a type invariant

## 2. Contract + projection (Rust)

- [x] 2.1 `ChatToolCallView` extended: `arguments`, `arguments_truncated`,
      `result`, `result_truncated` — `#[serde(default)]` so pre-existing
      sealed snapshots still deserialize; stance doc rewritten (narrowed,
      not deleted); `contracts.ts` regenerated via
      `XFALCON_WRITE_CONTRACTS=1 cargo test -p xfalcon-contracts`
- [x] 2.2 Domain `ToolCallRef` carries the payloads; capping centralized in
      `with_payloads`
- [x] 2.3 `da_repository`: answer pairing upgraded from presence-set to
      first-answer-wins row map (same rule as `chat::history`); arguments
      harvested from the advertised call (JSON text kept, object
      re-serialized, nothing fabricated); answer row content projected onto
      its call; module docs updated
- [x] 2.4 Rust tests: payload projection asserted in the tool-round test;
      new oversized-payload truncation test; full workspace suite green
      (1046 tests)

## 3. Frontend detail panels

- [x] 3.1 `formatPayload` (pretty-print valid JSON under a 20k-char
      threshold, verbatim otherwise, null passthrough) + tests
- [x] 3.2 Durable rows with recorded payloads expand into
      Parameters/Result panels (`ToolInput`/`ToolOutput` from the vendored
      AI Elements family); payload-less rows stay static headers
- [x] 3.3 Truncation note ("Truncated at 32 KiB — the full record stays in
      the execution log") on capped panels; hostile-payload inert-render
      test (literal text, no markup)

## 4. One umbrella per tool sequence

- [x] 4.1 `MessageList` renders display items: consecutive tool activity
      folds into ONE `tools` item (multi-round runs = one "N tools used"
      card above the final answer); tool-only turns stop rendering as
      turns; empty assistant turns render nothing
- [x] 4.2 A prose turn that itself carries calls contributes them to the
      open umbrella, which closes above the prose; user turns close the
      umbrella
- [x] 4.3 Virtualizer: items keyed by display-item id, umbrella estimated
      as one summary row; entrance guard + accent-rail selection moved onto
      items; grouping + empty-turn + separation tests added

## 5. Verification

- [x] 5.1 Full Rust workspace suite on Windows (1046 green) +
      contracts regenerated without drift
- [x] 5.2 Frontend chat suite green (97 tests), typecheck clean
- [x] 5.3 Visual baselines + Windows live pass (shared backlog with
      changes 1–2: regenerate `tests/visual` snapshots on Windows, then a
      live run against a real IDA MCP source — expand panels, verify
      truncation + umbrella grouping on a multi-round run)
