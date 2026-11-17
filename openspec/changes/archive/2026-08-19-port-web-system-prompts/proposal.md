# Port Web System Prompts

## Why

Asking desktop to "build a dashboard" returns SQL. Asking xfalcon-web the
same thing returns a dashboard.

Desktop sends **no system prompt at all**. Every `system_prompt` field on
the wire is `None` (`chat/service.rs`, `clients/da/chat_api.rs`) and
nothing is seeded into the run's context — `seed_messages` is the
conversation history and nothing else. The model runs with no persona, no
SQL contract, no response format, and no output contract. It answers with
SQL because SQL is the obvious reading of "build me a dashboard of X" when
nothing has said what a dashboard is in this product.

Web's answer is four prompt artifacts assembled in one place
(`RunEngine.execute`, `src/lib/server/run-engine.ts` ~L3094-3180), and the
one that matters most for this defect —
`buildInChatChartContractPrompt`'s `<visualization_contract>` — is a
hard-coded block in that file, not a skill. Porting skills alone would not
have fixed it.

## What Changes

- **Four prompt bodies copied byte-for-byte** into
  `src-tauri/src/agent/prompts/`, embedded with `include_str!`:
  `DEFAULT_SYSTEM_PROMPT` (7,974 chars),
  `<visualization_contract>` (5,472), the `<available_tools>` frame
  (3,809 + 32). Only the line endings differ — web's files are CRLF, these
  are LF. They were extracted by evaluating web's own builder functions, so
  every escape resolves exactly as it does in production rather than being
  transcribed by hand.
- **`agent::prompt` assembles them in web's order**: base (or the
  conversation's own prompt, which REPLACES the default rather than
  appending to it) → `<active_skills>` → `<visualization_contract>` →
  `<available_tools>`.
- **The BI gate is web's `isVisualizationRequest`**, both regexes ported
  verbatim: a loaded skill whose name matches the BI pattern makes every
  turn a BI turn; otherwise the message decides. Deterministic — same
  input, same route.
- **The tool inventory is generated the same way**: grouped by server name
  (everything before the first `.`), servers sorted, tools sorted within a
  server, `(no description)` for a tool that declares none.
- **The assembled prompt is prepended to the run's context** as a
  `ContextRole::System` message, in both the tool-session path and the
  no-tools path. It is built after the tool session resolves, because its
  inventory is a function of the tools the run actually got.

## Capabilities

### Modified Capabilities

- `chat-run-activity`: a run carries a system prompt, assembled from
  xfalcon-web's.

## Impact

- New: `src-tauri/src/agent/prompt.rs`, `src-tauri/src/agent/prompts/*.md`,
  `src-tauri/src/agent/tests/prompt.rs`.
- `src-tauri/src/chat/service.rs`: `with_system_prompt` at the seed seam.
- No wire contract, storage, or frontend change.

## Not in this change

**Skills.** Web advertises skills by name and description and lets the
model pull the body with a `get_skill` tool. Desktop has neither that tool
nor web's skill set: it bundles three of its own (`charting`,
`data-analysis`, `reporting`) written against different tool ids, and
`skill_scope` is `None` for every chat run. So `<active_skills>` never
renders today and the port is faithful to a tenant with no skills loaded.

The consequence to know about: `<visualization_contract>` tells the model
the exact v2 report-definition schema lives in the `ida-dashboard` skill,
fetched via `get_skill`. Desktop can serve neither. The model still gets
the contract's own `<how_to_build>`, `<output_form>`, `<html_rules>` and
`<chart_type_selection>` inline, which is what turns "build a dashboard"
into a dashboard — but not the exact v2 schema. Closing that means porting
web's skill catalogue and a `get_skill` tool, and it is its own change.

**Side-channel prompts.** Title generation and follow-up suggestions have
their own prompts and their own small models in web. Desktop titles
conversations by truncating the first message (`useChat.conversationTitleFrom`)
and has no follow-up surface, so there is nothing to inject them into yet.
