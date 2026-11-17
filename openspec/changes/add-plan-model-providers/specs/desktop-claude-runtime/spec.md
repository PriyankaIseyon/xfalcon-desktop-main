## ADDED Requirements

### Requirement: The Claude runtime is off until the user turns it on

Desktop SHALL keep the Claude runtime disabled on every device until the user
accepts a consent notice. The notice SHALL say that the conversation and tool
results go to Anthropic under the user's own plan and SHALL name the Anthropic
terms that apply to each plan type. Desktop SHALL store the choice per device in
`plan-providers.json`.

#### Scenario: First start

- **WHEN** a user opens Settings on a device that has never enabled the runtime
- **THEN** the runtime is off and no Claude model appears in the picker
- **AND** no `claude` process starts

#### Scenario: Consent

- **WHEN** the user enables the runtime and accepts the notice
- **THEN** Desktop stores the choice for this device
- **AND** shows Claude models only when the runtime is also ready

### Requirement: Desktop runs only the user's own unmodified Claude Code

Desktop SHALL start the `claude` program that the user installed. It SHALL NOT
bundle or install it, read or store its credentials, call the Anthropic API, or
set, clear or override any sign-in variable. It SHALL NOT start a Claude sign-in.
When the user is not signed in, Desktop SHALL show the command to run in a
terminal.

#### Scenario: Not signed in

- **WHEN** `claude auth status` reports `loggedIn: false`
- **THEN** Settings shows the sign-in command and no Claude model is offered
- **AND** Desktop reads no credential file and starts no sign-in

#### Scenario: Account fields stay private

- **WHEN** Desktop reads `claude auth status`
- **THEN** Desktop keeps only `loggedIn`, `authMethod`, `apiProvider` and `subscriptionType`, and the probe result carries only the plan and the sign-in method
- **AND** the email, organization id and paths appear in no event, log or screen

#### Scenario: An API key is set in the user's environment

- **WHEN** the user's environment carries `ANTHROPIC_API_KEY`
- **THEN** Desktop leaves it in place and shows the sign-in method that `claude` reports
- **AND** Desktop never sets or clears it

### Requirement: The tool loop stays in control

Desktop SHALL declare its tools to `claude` without letting `claude` execute
them. It SHALL return each tool request to the existing tool loop as a complete
`ToolCallRequested` event and SHALL stop the `claude` process before any tool
runs inside it. Tool policy, approvals, invocation records, budgets and DA
events SHALL be those of the existing loop.

#### Scenario: The model requests a tool

- **WHEN** a turn ends with one or more `tool_use` blocks
- **THEN** Desktop emits one `ToolCallRequested` per block, then `Stopped` with `ToolUse` and `Completed`
- **AND** the process tree is gone before the loop dispatches the first call
- **AND** the declaration server answers each held `tools/call` only after the process tree is gone, with an error-marked result that runs nothing

#### Scenario: Built-in tools

- **WHEN** a turn starts
- **THEN** the session lists only the declared Desktop tools
- **AND** `claude` has no file, shell or web tool

### Requirement: Each turn runs isolated

Desktop SHALL run each turn in a directory it owns that holds only the system
prompt file and its own MCP configuration, with no setting sources and no slash
commands. When the turn declares tools, Desktop SHALL bind the declaration server
to `127.0.0.1` with a random port and a random token per turn, and SHALL close it
when the turn ends.

#### Scenario: A project folder holds Claude hooks

- **WHEN** the user's project contains `.claude/settings.json` hooks or a `.mcp.json`
- **THEN** no hook runs and no extra server connects during a Desktop turn
- **AND** the `claude` process is started with `--strict-mcp-config`, `--setting-sources ""` and `--disable-slash-commands`

#### Scenario: A request without the token

- **WHEN** any process sends a request to the declaration server without the turn's token
- **THEN** the server answers 401 and exposes no tool

### Requirement: Claude models appear only when the runtime is ready

Desktop SHALL offer the Claude models only while the runtime is enabled, `claude`
is found, its version meets the verified floor, and the user is signed in. It
SHALL record `claude_code/<model id>` as the run model and `claude_code` as the
provider.

#### Scenario: Ready

- **WHEN** all four conditions hold
- **THEN** the picker lists the Claude models with a plain-text label
- **AND** a new run records the provider `claude_code`

#### Scenario: A locked conversation after the runtime goes away

- **WHEN** a conversation locked to a Claude model opens and its row is not offered (the runtime is off, `claude` is missing or outdated, or the user is signed out)
- **THEN** Desktop shows the Settings hint that the plan model is not available, and offers no silent fallback to another model

### Requirement: Turn directories and the device switch are removed

Desktop SHALL remove each turn's `turn-<uuid>` directory when the turn ends. It
SHALL remove the directories that a crash left under `claude-turns/` at start-up.
The account clear's device sweep SHALL also remove `claude-turns/` and
`plan-providers.json`.

#### Scenario: A turn ends

- **WHEN** a turn ends by completion, cancellation or failure
- **THEN** its `turn-<uuid>` directory is removed after the `claude` process is stopped

#### Scenario: A crash left a turn directory

- **WHEN** Desktop starts and finds `turn-*` directories under `claude-turns/`
- **THEN** it removes them before any turn starts

#### Scenario: Reset xFalcon

- **WHEN** the user runs Reset xFalcon
- **THEN** the account clear's device sweep removes `claude-turns/` and `plan-providers.json` after the account is gone

### Requirement: A silent `claude` ends the turn and is never retried

Desktop SHALL allow 60 seconds for the prompt write and the first output line
together, and 10 minutes of silence between later lines. When either bound expires,
Desktop SHALL stop the process tree and report the runtime as unavailable. It SHALL
NOT retry the turn.

#### Scenario: No first line

- **WHEN** `claude` writes no output line within 60 seconds of the turn starting
- **THEN** Desktop stops the process tree and reports `RuntimeUnavailable`
- **AND** the turn is not retried

#### Scenario: Silence after output

- **WHEN** `claude` writes no line for 10 minutes after its last line
- **THEN** Desktop stops the process tree before it removes the turn directory

### Requirement: Existing providers are unchanged

With the runtime off, the model catalog, IPC surface, provider dispatch and
inference output of every existing provider SHALL be identical to the behavior
before this change.

#### Scenario: Runtime off

- **WHEN** the runtime is disabled
- **THEN** the existing catalog, security and provider tests pass unchanged
- **AND** no listener opens and no process starts

## Addendum 2026-10-22: reasoning effort and per-chat lock

The requirements below extend this change. They are dated 2026-10-22 and do not
replace the requirements above.

### Requirement: The reasoning level is set per device and defaults to high

Desktop SHALL keep one reasoning level for the device in `plan-providers.json`, under
the key `claude_effort`. The levels SHALL be `low`, `medium`, `high`, `xhigh` and
`max`, and the default SHALL be `high`. Desktop SHALL NOT offer `ultracode`. A missing
level, or one that is unknown, of the wrong type or null, SHALL read as `high` and
SHALL NOT turn the runtime off. A missing, damaged, oversized or newer file SHALL
still read as everything off.

#### Scenario: A missing level

- **WHEN** `plan-providers.json` holds no `claude_effort` key
- **THEN** the level is `high`

#### Scenario: An unreadable level

- **WHEN** `claude_effort` is an unknown string, a value of the wrong type, or null
- **THEN** the level is `high`
- **AND** the effort does not turn the runtime off

#### Scenario: A damaged settings file

- **WHEN** `plan-providers.json` is damaged, oversized or a newer version
- **THEN** every switch reads as off

### Requirement: Every Claude run passes the chat's reasoning level

Desktop SHALL pass `--effort <level>` as the last two arguments of every `claude`
turn. Probes for the version and the sign-in status SHALL pass no effort. Desktop
SHALL withhold `CLAUDE_CODE_EFFORT_LEVEL` from the child process.

#### Scenario: A turn starts

- **WHEN** a Claude turn starts
- **THEN** its arguments end with `--effort <level>`, where the level is the chat's level

#### Scenario: A probe runs

- **WHEN** Desktop runs `claude --version` or `claude auth status --json`
- **THEN** no `--effort` argument is passed

#### Scenario: The user sets the variable

- **WHEN** `CLAUDE_CODE_EFFORT_LEVEL` is set in the user's environment
- **THEN** Desktop does not pass it to the `claude` process

### Requirement: Each Claude chat keeps the level it started with

On the first Claude turn of a conversation, Desktop SHALL store the current setting for
that conversation in the account's `claude-chat-effort.json`. Every later turn of that
conversation SHALL use the stored level, whatever the setting is now. Only chat runs
SHALL take the lock. Titles and follow-up suggestions SHALL NOT run on a plan model.

The file SHALL be version 1, with a `chats` array of `{id, effort}` entries, oldest
first. It SHALL hold at most 2,048 entries and drop the oldest first. Reads SHALL be
capped at 256 KiB. A damaged or oversized file SHALL count as empty and SHALL be
replaced. An entry with an unknown level SHALL be ignored. The file SHALL hold
conversation ids and levels only.

#### Scenario: A new chat

- **WHEN** the first Claude turn of a conversation runs with the setting at `xhigh`
- **THEN** the conversation's entry records `xhigh`
- **AND** every later turn of that conversation passes `xhigh`, even after the setting changes

#### Scenario: An existing chat after the update

- **WHEN** a Claude chat that predates this change runs its first turn after the update
- **THEN** the chat locks at the setting in force on that turn

#### Scenario: A damaged file

- **WHEN** `claude-chat-effort.json` is damaged or larger than 256 KiB
- **THEN** Desktop treats it as empty and replaces it

#### Scenario: The account clear

- **WHEN** the account clear runs, as the first step of Reset xFalcon
- **THEN** `claude-chat-effort.json` is removed
