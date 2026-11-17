# Change: Size what the model sees to the model it runs on

## Why

Every tool result the model sees is capped at 32 KiB (about 8k tokens), whatever the
model. The models this app runs have 500k to 1M-token windows. Above the cap a file
page, a listing, a search, or a query result reaches the model as 4 head and 4 tail
rows, 2 KiB of each string end, or a bare count. That costs extra rounds and loses
data the model needed.

Four related defects sit on the same path:

- A read page can grow to about 48 KiB against a 28 KiB budget, because the budget is
  checked after a long line is appended, so it then collapses in the preview.
- A directory listing has no paging; a large one reaches the model as a count only.
- Tool arguments over 32 KiB end the whole run instead of returning an error the model
  can act on.
- An assistant answer may stream up to 512 KiB, but a message is capped at 128,000
  bytes, so a long answer cannot be stored.

## What Changes

- **A per-run tool-result budget from the model's context window**: 5% of the window at
  4 bytes a token, clamped to 32 KiB–256 KiB. A 1M-token model gets about 195 KiB
  (50k tokens), a 200k model 40 KiB, a 128k model the 32 KiB floor. The 256 KiB
  ceiling is DA's stored-preview bound (data-analyst
  `raise-tool-result-preview-budget`).
- **Oversized results fill the budget** instead of 4+4 rows: as many leading rows,
  fields, and characters as fit, with exact omitted counts.
- **Read pages are sized from the budget** and never overshoot it; a line over the
  per-line bound is cut and marked.
- **Directory listings page**: `offset`/`limit`, a stable order, the true total, and the
  next offset.
- **Exact-result ranges and skill pages scale with the budget.**
- **Oversized arguments are answered**: the call gets `invalid_arguments` with the bound,
  and the run continues. The bound rises from 32 KiB to 256 KiB.
- **Message text rises to 512 KiB**, matching the run's output budget; the attachment
  bound in context matches the 10 MiB upload bound.

## Impact

- Affected specs: `agent-context-budget` (new), `project-tool-contract`
- Affected code: `src-tauri/src/tools/{budget,invocation,dispatch,exact_result,registry}.rs`,
  `src-tauri/src/agent/{turn_driver,local_executor,tool_loop,tool_session,output}.rs`,
  `src-tauri/src/projects/{read,commands}.rs`, `src-tauri/src/skills/dispatch.rs`,
  `src-tauri/src/chat/{domain,service}.rs`, `src-tauri/src/clients/da/chat_api.rs`
- **Rollout (BREAKING across services)**: DA must deploy
  `raise-tool-result-preview-budget` before a desktop build with this change reaches
  users. Against an older DA, a preview over 32 KiB is refused when it is stored.
