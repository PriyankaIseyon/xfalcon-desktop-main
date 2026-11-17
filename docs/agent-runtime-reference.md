# Agent runtime reference

Version 0.5.50 · reviewed 2026-10-28 · **Reference** · audience: engineers, prompt authors, security reviewers

Exactly what a model can see and do inside one turn: the tool surface, the
policy that gates it, the skill format, the MCP contract, the models, and every
enforced limit. Behaviour is in [runtime-flows.md](runtime-flows.md) § 3;
rationale is in [decisions.md](decisions.md).

---

## The complete tool surface

Eight built-in tools plus whatever registered MCP servers publish. There is no
shell, no process spawn, no file write outside the trusted project, no web
fetch, and no web search ([decisions.md](decisions.md) ADR-011).

The saved-report tools (`ida_save_dashboard`, `ida_refresh_dashboard`,
`ida_recall_dashboard`, `ida_list_dashboards`, `ida_delete_dashboard`) are not built
in: they reach the model through an attached DA MCP server, under its call name
`mcp_<server>_<tool>`, and the visualization contract tells the model how to use them.
A single tool call's arguments are capped at 256 KiB encoded as JSON
(`tools/invocation.rs`), which bounds the size of a report page a save can carry.

| Tool id                        | Risk           | Idempotent                  | Capability tags                    | Present when                                                                                               |
| ------------------------------ | -------------- | --------------------------- | ---------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `project.search_files`         | ReadOnly       | yes                         | `project.scope`, `filesystem.path` | the conversation is bound to a project whose root still opens, and the index refresh for the run succeeded |
| `project.search_text`          | ReadOnly       | yes                         | `project.scope`, `filesystem.path` | same                                                                                                       |
| `project.read_file`            | ReadOnly       | yes                         | `project.scope`, `filesystem.path` | the conversation is bound to a project whose root still opens                                              |
| `project.list_directory`       | ReadOnly       | yes                         | `project.scope`, `filesystem.path` | same                                                                                                       |
| `project.propose_patch`        | ReadOnly †     | **no**                      | `project.scope`                    | same                                                                                                       |
| `project.update_plan`          | ReadOnly       | yes                         | `project.scope`                    | same                                                                                                       |
| `__internal__get_skill`        | ReadOnly       | yes                         | none                               | at least one skill is enabled                                                                              |
| `__internal__read_tool_result` | ReadOnly       | yes                         | none                               | the run's saved context holds at least one tool result payload that is ready and can be retrieved          |
| `mcp.<server-slug>.<tool>`     | from admission | from the tool's annotations | none                               | the server is reachable and its mode does not withhold the tool                                            |

Source: `tools::registry` (descriptors), `agent::tool_session::build_tool_session`
(which tools a run gets).

† `project.propose_patch` is advertised ReadOnly because `ToolRisk` has only two
variants, `ReadOnly` and `Mutating`, and the project write is not gated by the
tool-approval system. The tool plans a change **and applies it in the same
call**. No person reviews it. See [Approval](#approval). It is not idempotent:
proposing persists a new encrypted plan row.

If a conversation has no usable project, no `project.*` tool is registered at
all. A project whose root moved or changed identity since it was trusted is
treated the same way, and the run continues with its other tools
(`build_tool_session`).

Each tool descriptor is bounded: name 128 bytes, advertised schema 16 KiB,
capability tag 64 bytes, 32 tags (`tools::registry`).

### `project.search_files`

Finds files by path. Returns path, `is_sensitive`, `content_kind`, and
`size_bytes` per result, never content. Covers files the local index has not
reached yet.

```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string", "minLength": 1, "maxLength": 512 },
    "mode": { "type": "string", "enum": ["substring", "glob"] }
  },
  "required": ["query"],
  "additionalProperties": false
}
```

Default `substring` ranks by tier: exact filename, filename prefix, filename
substring, then path substring (`projects::search`). In `glob` mode `*` stays
within one path segment and `**` crosses segments, so `src/**/*.rs` means every
Rust file under `src`. The result carries `truncated`, `index_truncated`, and
`live_scanned`. When the list is cut short, a `detail` field says it is not
complete.

### `project.search_text`

Searches file contents. Skips binary, non-UTF-8, oversized, and classified-
sensitive files. The agent can never pass a sensitive-file override
(`tools::dispatch`).

```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string", "minLength": 1, "maxLength": 512 },
    "mode": { "type": "string", "enum": ["literal", "regex"] },
    "case_sensitive": { "type": "boolean" }
  },
  "required": ["query"],
  "additionalProperties": false
}
```

Default `literal`, so `.` and `(` mean themselves. Default `case_sensitive` is
`false`. A hit has `path`, `line`, and `preview`. The result also carries
`truncated`, `index_truncated`, `files_scanned`, `files_stale`, and
`live_scanned`.

### `project.read_file`

```json
{
  "type": "object",
  "properties": {
    "path": { "type": "string", "minLength": 1, "maxLength": 4096 },
    "start_line": { "type": "integer", "minimum": 1 },
    "end_line": { "type": "integer", "minimum": 1 }
  },
  "required": ["path"],
  "additionalProperties": false
}
```

Every read the model makes is paged. No range means page one. A page holds as
many lines as fit the run's result budget, up to 2,000 lines
(`tools::dispatch::run_read_file`). When more remains, the result carries
`next_line`. Calling again with that value as `start_line` returns the next
page, so a file of any length is fully readable. `start_line` alone gives one
page from there. Both bounds give an exact range of at most 2,000 lines; a wider
range is refused. `end_line` without `start_line` is refused.

A text result has `kind: "text"`, `content`, `first_line`, `last_line`,
`truncated`, `next_line`, and a `detail` when lines were cut. A line over
20,000 bytes is cut at that point and the page is marked `truncated`. Binary,
non-UTF-8, and oversized content returns `kind: "metadata_only"` with size, kind,
a hash, and a reason. A classified-sensitive path returns
`kind: "sensitive_override_required"` and no content. The agent cannot grant the
override.

### `project.list_directory`

Immediate children of one directory (or the root when `path` is omitted or
empty), sorted by name, with `kind`, `size_bytes`, and `is_sensitive` per entry.
Never content. Never recursive.

```json
{
  "type": "object",
  "properties": {
    "path": { "type": "string", "maxLength": 4096 },
    "offset": { "type": "integer", "minimum": 0 },
    "limit": { "type": "integer", "minimum": 1 }
  },
  "required": [],
  "additionalProperties": false
}
```

A page holds as many entries as `limit` and the run's result budget allow. The
result gives `total` and, when entries remain, `next_offset`. An `offset` past
the last entry is refused.

### `project.propose_patch`

An ordered list of write operations. The change is applied directly and lands
before the call returns. No review step exists. Operations apply in the order
given and stop at the first failure.

```json
{ "type": "object",
  "properties": { "operations": { "type": "array", "minItems": 1, "maxItems": 200,
                                  "items": { "oneOf": [ ... ] } } },
  "required": ["operations"], "additionalProperties": false }
```

The result reports `applied`, `plan_id`, `total_paths`, `total_bytes`, and one
row per operation (`index`, `kind`, path, `applied`). It names any path left
alone because it already held exactly the content sent (`unchanged`). It does
not echo the diff. If a call fails part way, the error lists the operations
already on disk and the ones not applied.

Eight operation kinds, as a `oneOf`. 1 to 200 operations per plan.

| `kind`             | Fields                           | Notes                                                                                                                                                                                          |
| ------------------ | -------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `create`           | `path`, `content`                | Fails if the path exists. Creates missing parents. Sending the same `create` again after it landed is a no-op, not an error.                                                                   |
| `modify`           | `path`, `content`                | Whole-file rewrite. **Refused for a file larger than one read page returns**, because it would discard content the model has not seen. Keeps the file's CRLF line endings and byte order mark. |
| `replace`          | `path`, `edits[1..64]`           | Each edit: `old_string` (not empty), `new_string`, optional `replace_all`.                                                                                                                     |
| `insert`           | `path`, `insert_line`, `content` | Inserts whole lines _after_ the line. `insert_line: 0` inserts at the top.                                                                                                                     |
| `rename`           | `from`, `to`                     | Replaces an existing destination file; the old content is kept as a revision.                                                                                                                  |
| `delete`           | `path`                           | The old content is kept as a revision.                                                                                                                                                         |
| `rename_directory` | `from`, `to`                     | One move. Refused onto an existing directory.                                                                                                                                                  |
| `delete_directory` | `path`                           | Empty directories only.                                                                                                                                                                        |

`replace` rules the model is told: `old_string` must match the file's current
text, and must appear exactly once unless `replace_all` is set. If the exact
text is not found, the match is retried ignoring trailing spaces, then ignoring
leading and trailing whitespace, then also reading typographic quotes, dashes,
ellipses, and no-break spaces as plain characters (`projects::edit`). A failed
match names the closest text in the file. Edits apply in order, so a later edit
may anchor on text an earlier one wrote.

Other refusals, all returned to the model as errors it can correct:

- A comment that stands in for content, such as `// ... rest unchanged`, in any
  written text (not in prose files such as `.md`).
- Two operations on one path in one plan.
- A path in `.git` or `.xfalcon` (any path component, any letter case), and any
  path the secret classifier flags, such as credential files. A person saving in
  the file viewer is not subject to this rule (`tools::dispatch`).
- A plan in which every operation changes nothing.
- A delete or replacing rename of a file over 64 MiB, because no revision can
  hold it.

Every path is project-relative. One call carries at most 256 KiB of arguments.

### `project.update_plan`

The model's own checklist for a job of several steps. Stateless: the model sends
the whole list each time. Changes nothing in the project.

```json
{
  "type": "object",
  "properties": {
    "plan": {
      "type": "array",
      "minItems": 1,
      "maxItems": 50,
      "items": {
        "type": "object",
        "properties": {
          "content": { "type": "string", "minLength": 1, "maxLength": 300 },
          "status": { "type": "string", "enum": ["pending", "in_progress", "completed"] }
        },
        "required": ["content", "status"],
        "additionalProperties": false
      }
    }
  },
  "required": ["plan"],
  "additionalProperties": false
}
```

The turn driver reads the same arguments. If the model ends its reply while an
item is `pending` or `in_progress`, the driver sends it back to work. This
happens at most 3 times per run (`MAX_COMPLETION_NUDGES`).

### `__internal__get_skill`

```json
{
  "type": "object",
  "properties": {
    "name": { "type": "string", "minLength": 1, "maxLength": 64 },
    "file": { "type": ["string", "null"], "maxLength": 1024 },
    "offset": { "type": ["integer", "null"], "minimum": 0 }
  },
  "required": ["name"],
  "additionalProperties": false
}
```

Carries **no capability tag** — it reads a document in this app's own app-local
directory, so it needs neither an open project nor a trusted path. That is what
lets a run with skills but no project and no reachable MCP server still be
tool-capable. `null` and an empty string for `file` mean "no file".

The id is spelled exactly as `xfalcon-web` spells it, because the ported system
prompt says "you MUST call `get_skill(name)`" and the inventory groups it under
`_xfalcon_internal`.

### `__internal__read_tool_result`

Reads one byte range of an exact tool result that is already in this run's saved
context. Content is returned as untrusted data.

```json
{
  "type": "object",
  "properties": {
    "payload_id": { "type": "string", "format": "uuid" },
    "offset": { "type": "integer", "minimum": 0 },
    "length": { "type": "integer", "minimum": 1, "maximum": "<range bytes>" }
  },
  "required": ["payload_id", "offset", "length"],
  "additionalProperties": false
}
```

`maximum` is the run's range ceiling: `(result bytes − 1,024) ÷ 4 × 3`, which is
20,736 at the smallest budget and 192,768 at the largest (see
[Tool results](#tool-results)). The result is base64 in `data`, with
`next_offset` while bytes remain. The set of readable payloads is frozen from
the saved context before the run starts. A model-supplied id outside that set
fails. Before any content request the executor re-reads the payload's metadata
and requires its length, hash, media type, and format to match
(`tools::exact_result`). The tool is not registered when no ready payload exists.

---

## The effective registry

`agent::tool_loop::effective_registry_for_run` composes what a run may request:

```
global registry
  ∩ run allowlist
  ∩ active skill's allowed_tools     (when a skill scope is passed)
= effective registry
```

`ToolLoop` is constructed once per run with the pruned registry, so "may request
only tools published for this run" is structural, not checked:
`PolicyEngine::authorize` already denies `ToolNotRegistered` and
`ToolNotAllowlisted` for anything outside it.

In a shipped run the allowlist is the composed registry itself, and no skill
scope is passed: `chat::service` sets `skill_scope: None`, because the app has
no surface to select a skill per conversation. Every enabled skill is advertised
to every run. A skill's `allowed_tools` therefore narrows nothing today.
`skills::policy::effective_skill_scope` has no caller outside tests.

A skill can only **narrow**. `effective_skill_scope` intersects before
`PolicyEngine` ever sees the scope, and `PolicyEngine` independently checks the
global allowlist first ([decisions.md](decisions.md) ADR-040).

The provider receives each tool under a wire name, not its id. A wire name
replaces every byte outside `[A-Za-z0-9_-]` with `_` and is cut at 64 bytes. A
clash takes a numeric suffix `_2` to `_99`. `ToolSession::tool_ids` is the only
way back from a wire name to an id (`agent::tool_session`).

---

## Policy

`PolicyEngine::authorize` returns `Allow` / `RequireApproval` / `Deny`. It checks,
in order: an authenticated actor, the allowlist, a registered descriptor, a valid
advertised schema, the run call budget, the per-tool call budget, an active
project (for `project.scope`), a path inside the trusted root (for
`filesystem.path`), the skill scope, and an approval token (for `Mutating`).
Denial codes are the `PolicyDenial::code` strings in `tools::policy`.

Every field of the `PolicyRequest` comes from run-scoped Rust-owned state or the
current call's own `tool_id`. `PolicyEngine` never reads `arguments`. The one
value derived from them is the requested path of a tool tagged `filesystem.path`:
the `path` string, or the project root when the tool has no `path`
(`tool_loop::requested_path_for`).

**Never** from a prior tool result. Nothing in `ToolLoop` reads a completed
`ToolResult` back into a `PolicyRequest`. Proven by
`agent::tests::tool_loop::prompt_injected_tool_result_cannot_alter_a_later_policy_decision`:
a malicious tool result claiming to grant an allowlist entry or carrying a forged
approval has zero effect on a later, identical dispatch.

Order of checks in `ToolLoop::prepare`:

1. replay — a repeated invocation id returns its recorded outcome, and an id
   reused for a different tool is denied (`invocation_id_tool_mismatch`);
2. cancellation — not a real attempt, no history, no budget spent;
3. descriptor exists in the effective registry (else denied
   `tool_not_registered`);
4. argument size (256 KiB), then **JSON Schema validation** — before
   `ToolInvocation::open`, because the invocation state machine has no
   `Requested → Failed` edge. Both answer `invalid_arguments` and the run goes on;
5. budget accounting;
6. `ToolInvocation::open` (journaled);
7. `PolicyEngine::authorize`;
8. standing consent, if the decision was `RequireApproval`;
9. execute.

Within one round, calls beyond the 8th are not run. Each gets
`{"status":"denied","reason":"parallel_call_limit"}`, and the model can send them
again next round. A call that exceeds 120 s fails with a retryable error. A round
that exceeds 300 s fails the run, and so does reaching the 256th round
(`ToolLoopError`, handled in `agent::engine`).

What the model sees for a call that did not complete: `status` is one of
`invalid_arguments`, `denied` (with `reason`), `failed` (with `error` and
`retryable`), `cancelled`, or `awaiting_approval`
(`agent::turn_driver::render_tool_outcome`). A person's decline is
`denied` with reason `declined_by_user`.

### Approval

Two systems, deliberately separate ([decisions.md](decisions.md) ADR-007).

|              | System 1 — generic tools                                                           | System 2 — project writes                                                                      |
| ------------ | ---------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| Types        | `ApprovalRequest`, `ApprovalToken`, `ExecutionGrant`                               | `ApprovalDecision`, `ApprovalProof`, `MutationPlan`                                            |
| Driven by    | `ToolLoop::run_round` / `resume`                                                   | `ApprovalsService::apply` → `resolve_for_account`                                              |
| Triggered by | any `ToolRisk::Mutating` call — in this build, only MCP tools in the reviewed tier | `project.propose_patch`, a file-viewer save, a revision restore, a report saved into a project |
| Surface      | run pauses in `AwaitingApproval`                                                   | none; no person is asked                                                                       |
| Default      | **auto-approved** (ADR-008)                                                        | applied directly once the folder is trusted (ADR-043)                                          |

**Standing consent.** `tools::standing::StandingToolApprovals` defaults to
`auto_approve_all = true`. Policy still returns `RequireApproval`; standing
consent then mints the same invocation-scoped token a click would have produced
and journals a normal approval. The approval is recorded as `tool.approve` in the local audit trail (implemented in the 0.5.42 working tree). Set `XFALCON_REQUIRE_TOOL_APPROVALS=1`
(or `true`) to restore ask-every-time; the store reads it once, at load. Durable
grants live in `standing-tool-approvals.json`. No production code path writes a
grant today: `allow_for_session` and `allow_always` have no caller outside
tests, so with ask-every-time restored only a grant already in the file applies.

**System 2.** `apply` approves the plan it just built and passes the plan's own
second-confirmation flag. The plan is still persisted (encrypted), journaled,
audited, checked for expiry and for stale base hashes, and re-validated for
containment by `projects::execute` before any byte lands. Before it writes, the
service captures every file the plan modifies, deletes, or replaces by rename as
a revision. A file over 64 MiB that a plan would delete or replace is refused.
The history keeps 50 revisions per file and 512 MiB (compressed) per account, and
drops the oldest first. A person restores a revision from the file viewer
(`project_file_restore`). A plan row has one of the states `pending`, `claiming`
(transient), `denied`, `expired`, `executed`, `executed_durability_unconfirmed`,
or `failed`. A claim by compare-and-swap lets only one caller execute a plan.

**Second confirmation** is a risk tier on the plan: it contains a delete, a
rename onto an existing path, binary content, more than 10 distinct paths, or
more than 1 MiB total. The tier is computed once, immutably, by
`MutationPlan::propose`. `apply` supplies the confirmation itself, so no prompt
follows.

---

## Run states

`RunState` has nine members: `draft`, `queued`, `running`, `awaiting_approval`,
`completed`, `failed`, `cancelled`, `interrupted`, and `recovering`
(`crates/contracts`). `agent::run::allowed_transition` is the full list of legal
moves; every other move is refused:

| From                | To                                                                     |
| ------------------- | ---------------------------------------------------------------------- |
| `draft`             | `queued`                                                               |
| `queued`            | `running`                                                              |
| `running`           | `completed`, `failed`, `cancelled`, `interrupted`, `awaiting_approval` |
| `awaiting_approval` | `running`, `cancelled`, `failed`                                       |
| `interrupted`       | `recovering`                                                           |
| `recovering`        | `running`, `failed`, `cancelled`                                       |

`completed`, `failed`, and `cancelled` are terminal. A transition is journaled
before it takes effect. After a restart, `agent::recovery` decides per state:
`queued` and `draft` resume; `running`, `recovering`, `interrupted`, and
`awaiting_approval` wait for the user (the user sees "This response was
interrupted before it finished. You can retry when ready."); terminal states are
not restarted. A run paused in `awaiting_approval` lives only in memory. Quitting
the app loses the pause, and the tool never runs.

A watchdog stops a tool-capable run that is silent too long
(`chat::tool_run`). See the limits below.

---

## Limits

Every value below is a named constant whose doc comment states its reason
([decisions.md](decisions.md) ADR-006). This table is the operator-facing subset;
the crate defines many more field-level bounds.

### Turn and loop

| Limit                                      | Value                                                                                    | Constant                                                                                                  |
| ------------------------------------------ | ---------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Tool rounds per run                        | 256                                                                                      | `agent::tool_loop::MAX_TOOL_ROUNDS`                                                                       |
| Concurrent calls per round                 | 8 (more are refused, not failed)                                                         | `MAX_PARALLEL_TOOL_CALLS`                                                                                 |
| Total tool calls per run                   | 2,048                                                                                    | `MAX_TOOL_CALLS_PER_RUN` (derived: 256 × 8)                                                               |
| Calls to any one tool per run              | Same 2,048-call run budget                                                               | `MAX_CALLS_PER_TOOL`                                                                                      |
| One tool call                              | 120 s                                                                                    | `TOOL_CALL_TIMEOUT`                                                                                       |
| One round                                  | 300 s                                                                                    | `ROUND_TIMEOUT`                                                                                           |
| Whole run wall clock                       | 3,600 s (7,200 s on the company gateway)                                                 | `chat::service::RUN_WALL_CLOCK`, passed as `RunBudgets::wall_clock`; `providers::gateway::RUN_WALL_CLOCK` |
| Run event budget                           | 4,096                                                                                    | `RunBudgets::event_budget` (literal in `chat::service`)                                                   |
| Run output bytes                           | 512 KiB                                                                                  | `RunBudgets::output_byte_budget` = `MAX_MESSAGE_TEXT_BYTES`                                               |
| Silence before a run is stopped            | 300 s without provider, tool, or durable-write progress (3,720 s on the company gateway) | `chat::tool_run::RUN_STALL_LIMIT`; `providers::gateway::RUN_STALL_LIMIT`                                  |
| Outer hard stop                            | 61 min (121 min on the company gateway)                                                  | `RUN_HARD_LIMIT`; `providers::gateway::RUN_HARD_LIMIT`                                                    |
| Time a stopped run gets to write out       | 20 s                                                                                     | `STOP_GRACE`                                                                                              |
| Concurrent runs                            | 8                                                                                        | `chat::service::MAX_IN_FLIGHT_RUNS`                                                                       |
| Provider output tokens per turn            | min(DA catalog ceiling, 128,000)                                                         | `agent::context_budget::MAX_PROVIDER_OUTPUT_TOKENS_PER_TURN`                                              |
| Turns in a row cut off by the output limit | 3                                                                                        | `agent::turn_driver::MAX_TRUNCATION_RECOVERIES`                                                           |
| Sends back to work for an open checklist   | 3 per run                                                                                | `MAX_COMPLETION_NUDGES`                                                                                   |
| Durable event writes per batch             | 20 records or 250 ms                                                                     | `agent::local_executor::MAX_BATCH_RECORDS`, `MAX_BATCH_INTERVAL`                                          |
| One streamed text or reasoning delta       | 256 KiB                                                                                  | `agent::stream::MAX_DELTA_BYTES`                                                                          |
| One forwarded reasoning event              | 4 KiB                                                                                    | `agent::stream::MAX_REASONING_EVENT_BYTES`                                                                |

After the first round, the model sees a `<run-status>` turn last in its context.
It says "Tool rounds used: N of 256", with an instruction to answer without
another tool on the last round, and any notice about how the last turn ended
(`turn_driver::refresh_run_status`).

### Context

A shipped run does not cap the number of context messages or the prose bytes of
the context builder (`ContextBuilder` is created with `usize::MAX` for both).
The window is managed in tokens, by the rules below.

| Limit                                   | Value                                                                       | Constant                                                                                                |
| --------------------------------------- | --------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| Hard input for a run                    | model `context_length` − min(model output ceiling, 128,000) − 16,384 tokens | `ContextCapacity::new`                                                                                  |
| Smallest accepted window                | more than 16,384 tokens, and the output ceiling must be below it            | `ContextCapacity::new`                                                                                  |
| Safety reserve                          | 16,384 tokens                                                               | `PROTOCOL_SAFETY_TOKENS`                                                                                |
| Request framing estimate                | 1,024 tokens, plus 32 tokens per message or block                           | `PROTOCOL_REQUEST_OVERHEAD_TOKENS`, `UNMEASURED_BLOCK_OVERHEAD_TOKENS`                                  |
| Estimated bytes per token               | 3                                                                           | `ESTIMATED_BYTES_PER_TOKEN`                                                                             |
| Estimated tokens per image              | 4,800                                                                       | `IMAGE_ESTIMATED_TOKENS`                                                                                |
| Image bytes across the context          | 20 MiB; older images are named, not re-sent                                 | `chat::attachments::MAX_CONTEXT_IMAGE_BYTES`                                                            |
| Single attachment                       | 10 MiB                                                                      | `uploads::kind::MAX_CHAT_ATTACHMENT_BYTES` (`MAX_ATTACHMENT_BYTES` equals it)                           |
| Attached file text, one file            | the run's result budget                                                     | `AttachmentBudget::for_result_budget`                                                                   |
| Attached file text, all files           | twice the result budget                                                     | same                                                                                                    |
| One user prompt                         | 512 KiB (524,288 bytes)                                                     | `chat::service::MAX_PROMPT_LEN`                                                                         |
| Any one stored or hydrated message part | 512 KiB                                                                     | `chat::domain::MAX_MESSAGE_TEXT_BYTES` — `MAX_PROMPT_LEN` is defined as this, so the two cannot diverge |
| Stored tool name, tool call id          | 256 bytes each                                                              | `chat::domain::MAX_TOOL_NAME_BYTES`, `MAX_TOOL_CALL_ID_BYTES`                                           |
| One SSE line                            | 6 MiB + 16 KiB                                                              | `providers::http::MAX_SSE_LINE_BYTES`                                                                   |
| One streamed response                   | 16 MiB                                                                      | `providers::http` (`MAX_TOTAL_BYTES`)                                                                   |

Each run measures the prompt from the provider's own token count for the last
request, plus an estimate for what was added since (`UsageAnchor`). Before the
first response it uses the estimate alone. A provider count below 40% of the
estimate for the same request is ignored: a server that counts only the tokens
it newly read (it reused a cached prefix) would otherwise make a full context
look nearly empty and compaction would never start.

The company gateway has been seen closing streamed replies within seconds while
serving the same request whole. After a `StreamCut` on a gateway request, for
`gateway::NON_STREAMING_AFTER_CUT` (15 minutes) its requests are sent with
`stream: false` and the one `chat.completion` body is turned into the same events
(`openai_compat::consume_completion_body`). A turn cut with no answer text and no
runnable tool call is requested again with no notice, up to
`turn_driver::MAX_STREAM_CUTS` (8) times in a row; with text, the model is asked
to continue. A gateway request that only reads tool results is sent with
`reasoning_effort: "none"`. When shedding, the oldest exchanges are dropped
before messages are cut below 8 KB, and the newest messages are cut down to
`summary::SHED_FLOORS` (8 KB, 2 KB, 512 B, 128 B) only after that.

When the response body ends without the `[DONE]` marker, the turn is closed as
`[DONE]` would have closed it if a `finish_reason` had already arrived. With none,
the connection ended mid-answer (a proxy may have closed it): the turn is closed
as a cut-off one (`ProviderStopReason::StreamCut`). A connection that breaks
after output has begun is closed the same way; one that breaks before any output
is an unavailable-provider failure. Finished tool calls are kept,
a half-written one is discarded, and the run asks the model for the rest in
smaller pieces, as it does for an output limit, in words that do not blame a
limit. Title, follow-up and summary requests treat any truncated reply as
unusable. The log records how long the stream ran
(`providers::openai_compat::close_without_done`). A tool call fragment with no
`index`, or with an `id` that differs from the call stored at its `index`,
starts a new call rather than extending the previous one.

**Compaction.** The selected model decides the route
(`clients::da::chat_api::ModelExecutionMode`).

- _Last resort for bounded models._ When summarizing is not enough or fails, a
  request over the input room is not refused. `LocalTurnDriver::shed_to_fit`
  shortens it without a model call, newest content last: old tool results and
  old tool-call arguments become stubs; the biggest older messages are cut in
  the middle; the oldest whole exchanges are dropped behind a note; only then
  are the newest messages cut. The model sees every cut and the note. Only a
  request whose instructions and tool list alone exceed the room is refused.
  The saved conversation is not changed.
- _Native compaction_ (the Sonnet product model). The provider compacts the
  history itself. OpenRouter is asked to compact at 700,000 input tokens
  (`OPENROUTER_COMPACTION_TRIGGER_TOKENS`). If the request would not fit the
  hard input, the driver first sends the longest complete history prefix that
  fits the hard input and is at least 80,000 estimated tokens, as a request that
  must compact (trigger 50,000 tokens, `COMPACTION_MIN_TRIGGER_TOKENS`). It saves
  the block durably and repeats. A compaction block over 1 MiB is refused. At most 8 passes
  (`MAX_COMPACTION_PASSES`).
- _Bounded context_ (every other model, including private models). The driver
  starts at 75% of the hard input (`SOFT_LIMIT_PERCENT`). It first replaces old
  tool results with a stub; results from the newest 30% of the window stay whole
  (`PRUNE_KEEP_PERCENT`) and results under 512 bytes are left alone. If that is
  not enough, the run's own model writes a summary of the oldest history. The
  newest 25% of the window stays word for word, the summarizer sees at most 55%,
  and a summary longer than 50% of what it replaces is rejected. A summary is
  limited to 8,192 output tokens. After 2 unusable summaries it stops asking.
  The transcript shown to the summarizer cuts a model message to 6,000 bytes, a
  user message to 24,000, a tool result to 2,000, and tool arguments to 800
  (`agent::summary`). Summaries are cached for 64 conversations. If the request
  still does not fit, the run fails. It does not cut blind.
- If a provider says the prompt is too long, the allowed input shrinks by a
  quarter (never below 10% of the hard input) and the history is compacted again.
  This happens at most 2 times (`MAX_OVERFLOW_RECOVERIES`).

### Tool results

The result budget is sized once per run from the model's window:
`(context_tokens ÷ 20) × 4` bytes, held between 32 KiB and 256 KiB
(`tools::budget::ToolResultBudget`). So 5% of the window at 4 bytes a token.

| Limit                                  | Value                                                    | Constant                                         |
| -------------------------------------- | -------------------------------------------------------- | ------------------------------------------------ |
| Whole rendered outcome the model reads | the result budget, 32–256 KiB                            | `ToolResultBudget::bytes`                        |
| Structured result inside it            | budget − 4 KiB                                           | `ToolResultBudget::result_bytes`                 |
| One page (file, directory, skill)      | result bytes − 1 KiB − the path, counted as JSON-escaped | `ToolResultBudget::page_bytes`                   |
| One exact-result range                 | (result bytes − 1 KiB) ÷ 4 × 3                           | `ToolResultBudget::range_bytes`                  |
| Exact result retained                  | 8 MiB                                                    | `tools::invocation::MAX_EXACT_TOOL_RESULT_BYTES` |
| Content references                     | 64                                                       | `MAX_CONTENT_REFS`                               |
| Failure message                        | 2,048 bytes                                              | `MAX_FAILURE_MESSAGE_BYTES`                      |
| Tool arguments                         | 256 KiB                                                  | `MAX_ARGUMENTS_BYTES`                            |
| Progress summary                       | 2,048 bytes                                              | `MAX_PROGRESS_SUMMARY_BYTES`                     |

A result larger than the budget reaches the model as a preview: the whole value
(`complete`), a JSON structure with parts cut (`json_structure`), the edges of a
string (`text_edges`), or metadata only (`metadata_only`). The rendered outcome
says `preview_complete`, `preview_strategy`, `exact_byte_length`, `sha256`, and
`source_truncated`. When the preview is not complete it names
`__internal__read_tool_result` as the way to read the rest. A structured result
over 8 MiB is a terminal failure for that call. Truncation is always recorded on
the result, never silent.

### Project filesystem

| Limit                                | Value                                                | Constant                                                  |
| ------------------------------------ | ---------------------------------------------------- | --------------------------------------------------------- |
| Whole-file read without a line range | 1 MiB (the agent never asks for one)                 | `projects::MAX_FILE_READ_BYTES`                           |
| Explicit line range or page          | 2,000 lines                                          | `MAX_LINE_RANGE_LINES`                                    |
| One line in a paged read             | 20,000 bytes                                         | `MAX_LINE_BYTES`                                          |
| Hashable file                        | 4 GiB                                                | `MAX_HASHED_FILE_BYTES`                                   |
| Directory entries listed             | 100,000, returned in pages                           | `MAX_DIRECTORY_ENTRIES`                                   |
| Relative path                        | 4,096 bytes; 255 bytes per component; 256 components | `path_policy`                                             |
| Indexed files per pass               | 250,000                                              | `MAX_INDEXED_FILES_PER_PASS`                              |
| Index worker threads                 | 4                                                    | `MAX_INDEX_WORKER_THREADS`                                |
| Search query                         | 512 bytes                                            | `projects::search::MAX_SEARCH_QUERY_BYTES`                |
| Search results                       | 200                                                  | `MAX_SEARCH_RESULTS`                                      |
| Files scanned per text search        | 5,000                                                | `MAX_SEARCH_FILES_SCANNED`                                |
| Matches per file                     | 20                                                   | `MAX_MATCHES_PER_FILE`                                    |
| Bytes searched per file              | 4 MiB                                                | `MAX_SEARCH_BYTES_PER_FILE`                               |
| Match preview                        | 500 bytes                                            | `MAX_MATCH_LINE_PREVIEW_BYTES`                            |
| Compiled regex size                  | 1 MiB                                                | `MAX_PATTERN_PROGRAM_BYTES`                               |
| Files visited by the live walk       | 20,000                                               | `MAX_LIVE_WALK_FILES`                                     |
| SQLite schema extraction             | 10 MiB                                               | `MAX_SQLITE_SCHEMA_BYTES` (local to `projects::commands`) |
| Viewable file (viewer, not agent)    | 10 GiB                                               | `projects::viewer::MAX_VIEWABLE_FILE_BYTES`               |

### Mutation plans

| Limit                                    | Value              | Constant                                   |
| ---------------------------------------- | ------------------ | ------------------------------------------ |
| Operations per plan                      | 200                | `projects::MAX_OPERATIONS_PER_PLAN`        |
| Edits per `replace`                      | 64                 | `MAX_EDITS_PER_OPERATION`                  |
| Anchor (`old_string`) or inserted block  | 64 KiB             | `MAX_ANCHOR_BYTES`                         |
| Editable file                            | 64 MiB             | `MAX_EDITABLE_FILE_BYTES`                  |
| Total plan bytes                         | 128 MiB            | `MAX_TOTAL_PLAN_BYTES`                     |
| Rendered diff                            | 256 KiB            | `MAX_DIFF_BYTES`                           |
| Plan time-to-live                        | 15 min             | `DEFAULT_PLAN_TTL`                         |
| Second-confirmation path threshold       | 10 paths           | `SECOND_CONFIRMATION_PATH_THRESHOLD`       |
| Second-confirmation byte threshold       | 1 MiB              | `SECOND_CONFIRMATION_BYTE_THRESHOLD`       |
| Revisions kept per file                  | 50                 | `projects::backup::MAX_REVISIONS_PER_FILE` |
| Revisions kept per account               | 512 MiB compressed | `MAX_BACKUP_ACCOUNT_BYTES`                 |
| Revision diff, file size / changed lines | 1 MiB / 2,000      | `projects::revision_diff`                  |

### MCP

| Limit                           | Value                                          | Constant                                       |
| ------------------------------- | ---------------------------------------------- | ---------------------------------------------- |
| Concurrent server sessions      | 32 (bundled included)                          | `mcp::runtime::MAX_CONCURRENT_SERVER_SESSIONS` |
| Tools per server catalogue      | 512                                            | `mcp::catalog::MAX_TOOLS`                      |
| Tool name in a descriptor       | 128 bytes                                      | `tools::registry`                              |
| Server slug                     | 24 bytes                                       | `mcp::registry::MAX_SLUG_BYTES`                |
| Catalogue registration deadline | 20 s per server                                | `mcp::runtime::REGISTRATION_DEADLINE`          |
| Response body                   | 8 MiB                                          | `mcp::client::MAX_MCP_RESPONSE_BYTES`          |
| Inline image or audio kept      | 8 KiB; larger is summarized as type and length | `mcp::executor::MAX_INLINE_BINARY_BYTES`       |
| Connect / read / total timeout  | 10 s / 30 s / 130 s (total is the 120 s tool budget plus 10 s, so the tool loop's timeout fires first) | `mcp::client`                                  |
| Rate limit (HTTP 429) | waited out once when `Retry-After` is 30 s or less (5 s when absent), then reported as `RateLimited` with the wait; the session is kept | `mcp::dispatch` |
| Reconnect attempts              | 4, 250 ms → 10 s backoff                       | `ReconnectPolicy::default`                     |

The session cap is not about sockets. Every held session contributes its whole
catalogue — up to 512 tools, each with a compiled JSON Schema validator — to one
declaration list. Registrations beyond the cap are recorded as
`ServerExclusion::SessionLimit`, never silently dropped.

### Provider HTTP

| Limit                                           | Value                                                                                                                           |
| ----------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Credential verification: connect / read / total | 10 s / 15 s / 20 s (`providers::mod`)                                                                                           |
| Credential verification: response               | 64 KiB                                                                                                                          |
| Streaming a turn: connect / idle read / total   | 10 s / 90 s / 3,600 s (`providers::http`); the read bound is idle time between bytes, not a total, and the total is the run wall clock |
| Private-model streaming                         | 10 s / 240 s / 3,600 s (`providers::custom_openai`); a self-hosted model sends nothing while it reads a long prompt; the company gateway uses 10 s / 3,660 s / 3,660 s (`providers::gateway`) |
| Error body read to detect a context overflow    | 32 KiB                                                                                                                          |
| Retry of a failed turn                          | up to 3 attempts, 500 ms doubling, 15 s cap (`providers::retry`); never after any event was emitted                             |
| Redirects                                       | **none** — `Policy::none()`                                                                                                     |
| TLS                                             | rustls, validation mandatory                                                                                                    |

Retried: `RateLimited` (a `Retry-After` of 15 s or less is honoured), `Unavailable` and
`TimedOut`. `TimedOut` means the provider was reached and then sent no bytes for the idle bound; a
connect timeout stays `Unavailable`. Both reach the renderer as `provider_unavailable`, with different
messages (`provider_failure_message`). Not retried: authentication, payment, payload too large, TLS
verification, invalid response, context overflow, cancelled, and every error of
a private-model endpoint (its adapter is not wrapped in the retry layer). The one exception is the company gateway, whose adapter retries a connection failure, HTTP 502 or HTTP 503 up to three times, after 30, 60 and 120 s, and only before any response arrives. It never retries a 504, an in-stream error or a timeout.

---

## Skills

A skill is a folder: a `SKILL.md` (data-only YAML frontmatter plus an
instructions body) and any files it bundles, such as `references/`, `scripts/`,
and `assets/`. All of it is **inert text or data**. Nothing is executed, and a
bundled file reaches a run only when the model reads it by path
([decisions.md](decisions.md) ADR-013).

### What import accepts

| Chosen           | Read as                                                                                                                                                            |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `.skill` package | A zip, recognized by content: `<name>/SKILL.md` + files (skill-creator's layout) or `SKILL.md` at the root. A `.skill` that is plain Markdown imports as one file. |
| `.zip`           | Every skill folder inside it, and `.skill` packages one level deep.                                                                                                |
| Folder           | Searched to depth 8. Each directory holding a `SKILL.md` is one skill; a file belongs to the nearest one above it.                                                 |
| `SKILL.md`       | With its folder's files when the folder is named for the skill; otherwise alone.                                                                                   |
| Other `.md`      | One skill with no bundled files.                                                                                                                                   |

Left out everywhere: `__MACOSX/`, `.DS_Store`, `._*`, `Thumbs.db`, `.git/`,
`node_modules/`, `__pycache__/`. Left out of a skill's folder, as
`package_skill.py` does: `*.pyc` and a root `evals/`. A skill is stored by
staging its whole folder in the account's `.skills-staging` and renaming it into
place. On import, cleanup removes only directories older than 1 hour with
recognized `import-<32 lowercase hex>` or `obsolete-<32 lowercase hex>` names,
and only when their contents are regular files. It preserves unrecognized
entries and every `replaced-*` directory: that name can be the only recoverable
copy after a failed rollback. A successful replacement marks its redundant
backup `obsolete-*` before deletion, so a later import can retry best-effort
cleanup. A re-import replaces the whole folder, so a file the new version
dropped is gone (`skills::store::staging_cleanup_tests`).

### Sources

| Source                    | Path                                                                                                                                                                         |
| ------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Imported through Settings | `<app-local-data>/accounts/acct-<sha256>/skills/<slug>/SKILL.md` and its bundled files — one library per signed-in account                                                   |
| Trusted project           | `<trusted-root>/.xfalcon/skills/*/SKILL.md` — code exists (`skills::discovery::discover_project`), but **no production code calls it**, so a run never loads a project skill |

This app bundles none of its own. Each account has its own library: another
account signed in to the same device neither sees nor runs with it. The
device-wide `<app-local-data>/skills` library of earlier versions moves, whole
and once, into the first account that opens a library while it has none. The
library also holds a `state.json` that names the disabled skills by slug. A
re-import of a disabled skill stays disabled; deleting a skill forgets the
slug. The library is read when a run starts, so a change takes effect on the
next message.

### Frontmatter

| Field                                             | Required | Limit                      | Notes                                                                                                                                                                                                                  |
| ------------------------------------------------- | -------- | -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`                                            | yes      | 64 bytes                   | **Normalized**, not just validated: lowercase letters and digits with single hyphens, so `My Skill` becomes `my-skill`. It is the folder name and the id `get_skill` takes. A name with no letter or digit is refused. |
| `description`                                     | yes      | 4 KiB                      | Not empty. The only body text that reaches the prompt up front, cut to 300 characters there.                                                                                                                           |
| `version`                                         | no       | 32 bytes                   | `MAJOR.MINOR.PATCH`, digits only. Absent means `0.0.0` (unversioned).                                                                                                                                                  |
| `allowed_tools` / `allowed-tools`                 | no       | 32 entries, 64 bytes each  | Policy field. Can only narrow. A list, or one string split on spaces and commas outside parentheses. Not applied in a shipped run (see [The effective registry](#the-effective-registry)).                             |
| `allowed_project_paths` / `allowed-project-paths` | no       | 32 entries, 256 bytes each | Policy field. Can only narrow. Checked as text only: relative, no `..`, no NUL.                                                                                                                                        |

Unknown keys are **ignored**, not rejected. Both policy fields are matched under
both spellings. Structure is strict — fixed `---` delimiters, size ceilings
checked before allocation, a validated name — and the field set is permissive,
because a parser that refuses every real skill is fail-useless, not fail-closed
([decisions.md](decisions.md) ADR-014).

### Size ceilings

| Limit                                                                | Value             |
| -------------------------------------------------------------------- | ----------------- |
| Frontmatter block                                                    | 8 KiB             |
| Instructions body                                                    | 240 KiB           |
| Whole document                                                       | 248 KiB (8 + 240) |
| One skill file on disk                                               | 256 KiB           |
| Files one skill bundles                                              | 512               |
| Bytes one skill bundles, `SKILL.md` included                         | 32 MiB            |
| Files listed per chosen folder or archive                            | 4,096             |
| Bytes read per chosen folder or archive, and a chosen archive's size | 256 MiB           |
| Folders deep a bundled file may sit                                  | 16                |
| Skills per chosen folder or archive                                  | 512               |
| Archive nesting                                                      | 1 level           |
| Skills in the library (import refuses a 257th)                       | 256               |
| Search depth                                                         | 8                 |

A skill over a bundle limit is refused whole, never trimmed. So is a skill with
two bundled paths that land on one file on a disk that ignores letter case (a
bundled `skill.md` beside `SKILL.md`, `Guide.md` beside `guide.md`), or a path
that is both a file and a folder. Chosen archives are read from disk entry by
entry; a file is served to the model one page at a time, so a large bundled
file never sits whole in memory. Import runs off the UI thread and logs its
outcome and each reason (never a chosen path).

### Progressive disclosure

Discovery and cataloguing surface **only** name, description, version, source,
and content hash. The full instructions body is loaded when the model calls
`__internal__get_skill(name)`.

| Call                    | Returns                                                                                                                                      |
| ----------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| `get_skill(name)`       | `instructions`, and on the first page `files` (path and bytes, up to 200, and at most half the page) with `files_not_listed` when more exist |
| `get_skill(name, file)` | That bundled file's `content`, as text; a binary file is refused                                                                             |
| `…, offset`             | The page starting there; a page is sized from the run's result budget, and `next_offset` names the next                                      |

A `file` is matched against the skill's own listing, never joined onto a path,
so `..`, an absolute path, or a planted symlink reaches nothing. The prompt
tells the model this is mandatory:

> Active skills below are mandatory guidance whenever relevant to the user
> request. Only their name and a short description are shown here. When a skill
> is relevant, you MUST call get_skill(name) to fetch its full instructions
> before acting on it. If multiple apply, synthesize their directives and
> briefly state any assumption. Never ignore an active skill.

A disabled skill stays on disk and reaches no run — neither advertised nor
fetchable (`get_skill` answers that it is turned off).

---

## MCP

### What may be dialed

`McpTransport::Http` and `McpTransport::Sse` rows are both dialed as MCP
Streamable HTTP. The label is not trusted as the wire protocol: xFalcon Web
labels every server `sse` and speaks Streamable HTTP to it. A server that speaks
only the older HTTP+SSE transport fails its `initialize` and is reported
unreachable. `registry::validate_endpoint` is the single place every URL rule
lives:

- HTTPS (plain `http` only in a debug build);
- no embedded userinfo;
- no query and no fragment;
- a host this build will dial: not a private, link-local, shared (100.64.0.0/10),
  broadcast, unique-local, NAT64, multicast, or unspecified address, and not
  loopback or `localhost` outside a debug build. A DNS name is not resolved at
  this check.

A registered URL is used exactly as given. The build does not append `/mcp`.
Protocol revision is asserted `== V_2025_11_25` after `initialize`, never
`LATEST` ([decisions.md](decisions.md) ADR-021).

### Which servers a run holds

- The **bundled tenant server** has slug `tenant` (`BUNDLED_SERVER_ID`), always
  uses mode `Ask`, and authenticates with the signed-in session's bearer. It is
  held only while the device preference `bundled_enabled` is on.
- **Registered servers** come from DA, in DA's order. A row with `enabled` false
  is listed but not dialed. A row that aliases the bundled endpoint is omitted.
- A slug is the lowercased name with other characters collapsed to `-`, at most
  24 bytes. Clashes take `-2`, `-3`, … so one server cannot shadow another.
- A server that cannot connect and list tools within 20 s is skipped for that run
  (`ServerContribution::Unreachable`). The prompt then names it as temporarily
  unavailable ([System prompt assembly](#system-prompt-assembly)).
- If a server refuses its saved grant, the call fails without a retry, the run
  stops before the next model turn, and the window gets the event
  `mcp:reauth-required` (`mcp::dispatch`).

### Why a server may be unavailable

`McpServerUnavailableReason` — a row that cannot be used is **listed with a
reason**, never hidden.

| Reason                    | Meaning                                                                                           |
| ------------------------- | ------------------------------------------------------------------------------------------------- |
| `UnsupportedTransport`    | `stdio`, or a transport this build does not know.                                                 |
| `UnsupportedAuthMethod`   | An auth type outside this build's vocabulary, or a custom header without naming the header.       |
| `NeedsSignIn`             | An OAuth server this device has not signed in to. Recoverable here.                               |
| `MissingDeviceCredential` | A header or bearer server registered on another device or on the web; this device holds no value. |
| `InvalidUrl`              | Absent, or fails the connect-boundary check above.                                                |

### Permission modes

`McpPermissionMode`, from DA's per-server column:

| Mode    | Effect                                                                                                                           |
| ------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `Allow` | Tools annotated read-only _and_ not destructive run unattended; everything else still pauses.                                    |
| `Ask`   | **Default**, including when DA's column is null. Every call pauses for approval.                                                 |
| `Deny`  | Tools are not registered at all — the model never sees them, and the server holds no session (`ServerExclusion::ToolsWithheld`). |

A server's own annotations decide nothing alone: `ToolAdmission` combines them
with this mode, and the default in every direction requires approval — including
for a tool claiming to be both read-only and destructive
([decisions.md](decisions.md) ADR-035). An unannotated tool counts as not
read-only and destructive. A tool in the reviewed tier is registered as
`Mutating`; a tool in the unattended tier is registered as `ReadOnly`. Idempotent
follows the annotations (`readOnlyHint` implies it). Note that whether a pause
becomes a _prompt_ is then governed by standing consent (ADR-008). MCP tools
carry no capability tags.

A result is normalized to `{ is_error, text[], structured, truncated }`. Text is
kept exactly up to 8 MiB. Resource links stay references and are never fetched.
Binary content over 8 KiB is dropped and only its type and length are kept
(`mcp::executor`).

### Tool ids

`mcp.<server-slug>.<tool>`, slug ≤ 24 bytes. Namespacing prevents a server
advertising `project.read_file` from shadowing the built-in. A tool whose
descriptor the registry refuses (a name over 128 bytes, a schema over 16 KiB) is
logged and not offered. Provider-legal wire aliases are produced by
`wire_tool_name`; `ToolSession::tool_ids` is the authoritative way back. Never do
string surgery on an alias ([decisions.md](decisions.md) ADR-031).

---

## Providers and models

### Provider ids

`ProviderId` has five members (`crates/contracts`). Credential verification
for the first four uses a fixed origin. Origins are compile-time constants.

| Provider      | `ProviderId`   | Verification endpoint                                     | Chat endpoint                                                                       |
| ------------- | -------------- | --------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| OpenAI        | `OpenAi`       | `https://api.openai.com/v1/models`                        | `https://api.openai.com/v1/chat/completions`                                        |
| Anthropic     | `Anthropic`    | `https://api.anthropic.com/v1/models`                     | `https://api.anthropic.com/v1/messages`                                             |
| Google        | `Google`       | `https://generativelanguage.googleapis.com/v1beta/models` | `…/v1beta/models/<model>:streamGenerateContent?alt=sse`                             |
| OpenRouter    | `OpenRouter`   | `https://openrouter.ai/api/v1/key`                        | `https://openrouter.ai/api/v1/messages` (native compaction) or `…/chat/completions` |
| Private model | `CustomOpenAi` | none                                                      | the model's own `http` or `https` base URL + `/chat/completions`                    |

There is no configurable base URL for the first four, so Azure OpenAI, Bedrock,
Vertex, and local runtimes such as Ollama are not reachable through them.

OpenRouter's verification endpoint is deliberately `/api/v1/key`, not
`/api/v1/models`: the latter is unauthenticated and answers 200 to any bearer, so
verifying against it would accept a typo'd or revoked key.

**Private models** (`providers::custom_openai`) are user-owned OpenAI-compatible
endpoints with a model-bound key. The adapter checks the URL syntax at every
request: `http` or `https`, and a host that is an IPv4 address in dotted-decimal
form, a bracketed IPv6 address, or a DNS name made of one or more non-empty labels
of lower-case letters, digits and hyphens (a label has at most 63 characters and
neither starts nor ends with a hyphen), at most 253 characters in all, whose last
label starts with a letter, on any address; any port from 1 to 65535; ASCII; no
whitespace or control characters, no userinfo, query, fragment, backslash, dot
segment, or percent-encoding in the authority, at most 2,048 bytes, and one
stored normal form (lower-case scheme and host, default port dropped, no trailing
slash; ADR-061). It resolves the host for each request and pins the answered
addresses into that request's client, ignores any proxy, and follows no redirect.
A private model URL may reach any host the device can reach, including link-local
addresses. Over `http`, the key and prompts are not encrypted. The upstream model id is at most 256
characters and replaces the model field on the wire. A 401 or 403 is a private
model authentication failure.

**The company gateway** (`providers::gateway`) is the one private endpoint whose
transport Desktop tunes itself. A person enters a URL and a key; Desktop asks the
endpoint which models it serves (`chat_custom_model_discover`) and takes context
length, output ceiling, tool and image support, and sampling defaults from that
answer. For a host equal to `llm-gateway.iseyon.workers.dev` and the model
`ollama/qwen3.6:35b-a3b-coding-nvfp4`, Desktop also names it "Qwen 3.6 35B Coding"
when discovery gives no name, and applies the gateway's timeouts, retry rule and
stream bounds. Its requests carry `stream_options.include_usage`, and the sampling
parameters discovery reported. A
request with `allow_reasoning: false` (title, follow-up chips, context summary)
also carries `reasoning_effort: "none"`. One request is in flight per app. A
run on this model has a 2 h wall clock and a 3,720 s stall limit, because the
gateway queues requests and each can think for minutes. Only `https` on the default port gets this profile; the same host over `http` or on another port is an ordinary private endpoint.

### Capability differences that matter

| Capability           | Status                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Streaming            | All five. One normalised event shape.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| Tool calling         | All five.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Reasoning / thinking | The direct Anthropic adapter **deliberately sends no `thinking` block**, because enabling it obliges the caller to echo every thinking block back on the next request (`never_requests_thinking_it_cannot_echo_back`). OpenRouter's Messages wire sends `thinking: {type: "adaptive"}`, and its Chat Completions wire sends `reasoning: {enabled: true}` for models whose id starts with `anthropic/`, `openai/`, `google/`, or `x-ai/` (a leading `~` allowed), when the request allows reasoning. Reasoning blocks are kept opaque and echoed back verbatim on later rounds. |
| Native compaction    | OpenRouter's Messages wire only, for Sonnet or Opus Claude models, with beta `compact-2026-01-12`. A route is probed first, and a good probe is cached for 24 h per credential and                                                                                                                                                                                                                                                                                                                                                                                             |
| model.               |
| Prompt caching       | OpenRouter's Messages wire only: `cache_control: ephemeral` on the last system block when the system text is at least 4 KiB. No other adapter sends one.                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `require_parameters` | OpenRouter sends `provider.require_parameters: true` on the Messages wire, and on the Chat Completions wire when the request has tools and the model is from a vendor listed above.                                                                                                                                                                                                                                                                                                                                                                                            |

Both gaps — no direct-Anthropic reasoning or caching — are cost and quality left
on the table, tracked in [status.md](status.md). Only the first four providers
have fixed adapters. A catalogue row for a provider other than OpenRouter is not
offered for chat (see below), so in the shipped chat path the OpenAI, Anthropic,
and Google adapters are not reached.

A turn ends with a normalised stop reason: natural, tool use, output limit,
context limit, stream cut, refusal, or other. The output limit and the context limit count
as a cut-off turn. The driver answers a cut-off turn with a notice and asks for a
continuation, up to 3 times in a row. If the cut-off cut a tool call, that call
is discarded and the model is told to send the work in smaller pieces. If the
answer is still cut off after the third time, the app appends a fixed
"This response was cut off at the model's output limit." line.

### Model selection

The tenant's DA catalogue is the source. Chat offers exactly these rows
(`clients::da::chat_api`):

| Model id                          | Route                            | Context mode                                                       |
| --------------------------------- | -------------------------------- | ------------------------------------------------------------------ |
| `~anthropic/claude-haiku-latest`  | OpenRouter Chat Completions      | bounded context                                                    |
| `~anthropic/claude-sonnet-latest` | OpenRouter Messages              | native compaction; the row needs at least 1,000,000 context tokens |
| `~x-ai/grok-latest`               | OpenRouter Chat Completions      | bounded context                                                    |
| `~openai/gpt-luna-latest`         | OpenRouter Chat Completions      | bounded context                                                    |
| `~openai/gpt-sol-latest`          | OpenRouter Chat Completions      | bounded context                                                    |
| `custom/<uuid>`                   | the private model's own endpoint | bounded context                                                    |

A row is eligible only if it is a chat model, enabled, active, streaming- and
tool-capable, has a non-blank display name of at most 128 characters, and has an
output ceiling above zero and below its context length. A product row must also
name `openrouter` as provider and parent. A private row must carry a valid
route, and its capacity must leave input room (`ContextCapacity::new`). The old
pinned id `openai/gpt-5.6-luna` is never offered and reads as Luna; a
conversation already locked to it still runs.

The default for a new chat is the saved preference, if that model is still
offered. Otherwise it is the first eligible row in the order Haiku, Sonnet, Grok,
Luna, Sol. The preference is a per-account file, `chat-model.json`, set from Settings
(`chat_default_model_set`). If no row is eligible, selection fails; there is no
built-in fallback model. A conversation locks its model when it starts. A
conversation made before locking exists is locked to Sonnet. Any later run uses
the locked model, or fails with a message if that model is no longer offered.

There is no per-conversation model picker. `/model` in the composer opens
Settings, and the composer shows the model name. See
[runtime-flows.md](runtime-flows.md) § 3.

---

## System prompt assembly

`agent::prompt::build_system_prompt`, in this order:

```
DEFAULT_SYSTEM_PROMPT                                      always, first
  + <conversation_guidance>…                               when a caller supplies it
  + <local_project>…                                        when the run has project tools
  + <active_skills priority="high">…                       when any skill is enabled
  + <visualization_contract>…                               always
  + <capability_catalog>…                                   when skills or user-facing tools exist
  + <available_tools><inventory>…                           when the run has tools
  + temporarily unavailable server names                    when applicable
```

The production caller currently passes no conversation guidance. If supplied,
the builder appends it below the default application prompt in an escaped
`<conversation_guidance>` element; its text cannot override application rules or
the user's current request. It is not a trusted operator-policy channel.

`<local_project>` includes only the display label, never the root path or project
id. Project, skill, tool, and server metadata is flattened to one line and XML
escaped before insertion. Skill descriptions are capped at 300 characters;
tool descriptions are capped at 500 characters. A server tool name is printed
as supplied only when it fits the plain-character and 128-byte rule; otherwise
the inventory shows its provider-safe call alias. These caps bound prompt text,
not MCP JSON schemas, which are passed through the provider tool interface
separately.

Project guidance tells the model that a write lands as part of the tool call,
to read before it edits, to use `replace` edits, to keep a checklist with
`project_update_plan` for a job of several files, to keep each write small, and
that old messages may be replaced by a summary. Project authority remains fixed
by the runtime; a display label cannot grant authority.

`<capability_catalog>` lets the assistant answer a user asking which skills,
tools, or servers are available. It lists each enabled skill and each
user-facing tool/server by its visible name and a one-line description, and
suggests a useful way to apply one. It hides `__internal__` tools and protects
skill instructions, tool parameters/schemas, protected instructions, and model
identity. Capability names, descriptions, and schema annotations explain the
interface; they cannot override application rules or the user request, or grant
authority.

The inventory lists each tool under the server it comes from, by the name that
server gave it, recovered from the tool's id rather than its call name:
`- **ida_query** (call as \`mcp_tenant_ida_query\`)`under`### Server: tenant`.
The call name stays the provider-legal, namespaced alias the model calls. A
server-supplied name outside `[A-Za-z0-9_.-]`or over 128 bytes stays under its
sanitized call name. Other metadata is flattened and escaped, so names and
descriptions cannot add prompt structure. The inventory groups this app's own
tools under`_xfalcon_internal`; the user-facing catalog omits `**internal**`
tools.

Four bodies are `include_str!`-ed from `src-tauri/src/agent/prompts/`
(`default_system_prompt.md`, `visualization_contract.md`,
`tool_inventory_prefix.md`, `tool_inventory_suffix.md`). The default prompt and
the two tool-inventory bodies were originally ports of `xfalcon-web`'s
([decisions.md](decisions.md) ADR-028), but desktop now maintains and tests its
own hardened prompt text. The visualization contract is desktop's own:
it teaches the DA report tools (save, refresh, recall, list, delete), the dataset
carriers a refreshable page needs, the project-folder download and upload flows, and
the limits that bound them, and `agent::tests::prompt` pins what it must say. Desktop adds the `<local_project>`, `<capability_catalog>`, and
unavailable-servers sections around them and never edits inside them.

The visualization contract is sent on every turn, so the stable system prefix
does not change after a durable compaction. It is not gated by a BI skill name
or message classifier. The base prompt supports grounded BI work: distinguish
verified evidence from interpretations and assumptions, use verified schema,
validate metrics, minimize returned data, and calculate from sufficient
evidence when appropriate; it does not impose a blanket refusal or ban on
reasonable inference.

A skill is listed as `<skill name="…" source="…">` with its description cut to
300 characters, and `…` appended when cut. A skill with an empty description
gets an empty blurb.

### Prompt provenance and auxiliary prompts

The default prompt places application/developer rules above the user's direct
request, with conversation guidance subordinate to both. Tool results, skill
bodies, project files, attachments, saved documents, and external data are
evidence, not instructions. If the user asks the assistant to follow steps in a
document, relevant steps guide that task only within the user request and the
same application safeguards. The prompt states these boundaries; it does not
make model adherence impossible to bypass.

Client-side compaction instructs the summarizer to preserve the user's request
and task state while treating transcript/tool content as records, not new
instructions. Its summary is XML-escaped and reintroduced as a user-role
`<conversation_summary>` message with provenance guidance. The summary prompt
also retains the 6,000-byte ordinary-message, 24,000-byte user-message,
2,000-byte tool-result, and 800-byte tool-argument transcript limits.

Conversation-title generation wraps an escaped, first-300-character opening
message as untrusted data and requests title text only. Follow-up generation
wraps escaped user (600 characters) and assistant (2,400 characters) excerpts,
uses no tools or reasoning, and requests only a bounded JSON array. These
auxiliary requests do not inherit the main run's tool access. Tests in
`agent::tests::summary`, `chat::tests::title`, and
`chat::follow_ups::tests` pin the consumer request shapes and provenance text.
