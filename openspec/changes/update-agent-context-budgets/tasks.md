## 1. Budget

- [x] 1.1 `ToolResultBudget` from the context window; floor, ceiling, page and range
      sizes; unit tests on the derivation.
- [x] 1.2 Threaded through run assembly into the turn driver, the local executor's
      durable preview, the project dispatcher, the skill dispatcher, and the exact-result
      reader.

## 2. Results

- [x] 2.1 Budget-filling preview for arrays, objects, and strings, with exact omitted
      counts; tests at several budgets.
- [x] 2.2 `render_tool_outcome` and the durable preview use the run budget; DA-read
      validation uses the ceiling.
- [x] 2.3 Oversized arguments answer `invalid_arguments` with the bound; bound 256 KiB.

## 3. Project tools

- [x] 3.1 Read pages sized from the budget with no overshoot; per-line cut marked.
- [x] 3.2 Directory listing pages with `offset`/`limit`, stable order, `total`,
      `next_offset`.

## 4. Messages

- [x] 4.1 Message text 512 KiB, output budget 512 KiB, context attachment 10 MiB.

## 5. Validation

- [x] 5.1 `openspec validate update-agent-context-budgets --strict` passes.
- [x] 5.2 Rust suite, clippy, fmt; vitest suite; the contract snapshot test
      (2026-09-25, after the audit fixes: Rust 1893/1893, vitest 1900/1900).
- [ ] 5.3 On archive, replace the `read.rs:86 MAX_PARTIAL_READ_BYTES` row of
      `specs/project-tool-contract/spec.md` (not a requirement, so no delta carries it)
      with `tools/budget.rs` `ToolResultBudget::page_bytes`.

## Evidence

- `tools::budget::tests` (derivation, floor, ceiling, nested sizes, escaping).
- `tools::tests::invocation` (table and object fill at two budgets, complete when it fits).
- `agent::tests::turn_driver` (a larger budget shows more rows; oversized-call answer).
- `agent::tests::tool_loop::arguments_over_the_per_call_bound_are_answered_and_the_next_call_still_runs`.
- `tools::tests::dispatch` (listing pages in stable order; long-line pages fit; a line too long for a page is cut and skipped past).
- `tools::tests::step_summary` (paged and historical listing shapes).
- `chat::tests::domain::a_message_holds_any_answer_a_run_may_produce`.
- `clients::da::tests::native_context_wire::tool_preview_ceiling_matches_the_da_durable_payload_contract`.
- Full workspace suite 1839 passed, 0 failed; `cargo clippy --all-targets -D warnings` clean.
