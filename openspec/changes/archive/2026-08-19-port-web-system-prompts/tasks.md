# Tasks

## 1. Copy the prompt bodies

- [x] 1.1 Extract by EVALUATING web's own builders (not transcribing), so
      every escape resolves as it does in production.
- [x] 1.2 `agent/prompts/default_system_prompt.md` (7,974 chars),
      `visualization_contract.md` (5,472), `tool_inventory_prefix.md` (3,809),
      `tool_inventory_suffix.md` (32). LF, not web's CRLF.

## 2. Assemble them

- [x] 2.1 `agent/prompt.rs`: `build_system_prompt` in web's order — base →
      skills → visualization contract → tool inventory.
- [x] 2.2 `is_visualization_request` with both of web's regexes verbatim.
- [x] 2.3 Tool inventory grouped by server, servers and tools sorted,
      `(no description)` fallback, internal-tool server name.
- [x] 2.4 A conversation prompt replaces the default; blank falls back.

## 3. Wire it into the run

- [x] 3.1 `with_system_prompt` prepends a `ContextRole::System` message to
      the seed context.
- [x] 3.2 Applied on BOTH paths — the tool-session run and the no-tools
      run — with the inventory built from the tools the run actually resolved.

## 4. Verification

- [x] 4.1 `agent/tests/prompt.rs` — size and section pins, the gate's
      positive and negative cases, ordering, inventory shape.
- [x] 4.2 `cargo test --lib` green (1045 passed).
- [x] 4.3 Live: "build a dashboard" returns a dashboard, not SQL.
