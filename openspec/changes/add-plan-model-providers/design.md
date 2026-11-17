## Context

Desktop is the only run executor. `ProviderAdapter::stream` turns a bounded
`ContextMessage` list into one model turn and emits `ProviderStreamEvent`s. The
tool loop (`agent/tool_loop.rs`) owns tool policy, approvals, durable
invocation records, budgets and the DA event sequence. It calls `stream` again
after it runs the tools of a turn.

Both new providers fit that contract. Neither one lets a vendor program run
Desktop's tools. The tool loop stays in charge for both.

## Shared decisions

### Provider ids, selection and DA

Add `claude_code` and `chatgpt_plan` to `ProviderId`, the way `custom_openai`
was added. Every exhaustive `match` gets an arm. `agent/da_events.rs` maps both
ids by hand. `settingsCopy.ts` holds both ids in total `Record`s, so the labels
and the model-source badge (`MODEL_SOURCE_LABEL`, read by `ModelsSection.tsx`)
cannot omit them. `clients/da/mapping.rs` keeps its `_` arm, which rejects both
ids as provider names. The mapping guard does not cover model rows. The one DA
read, `list_model_rows` (`clients/da/chat_api.rs`), drops every catalog row whose
model id starts with `claude_code/` or `chatgpt_plan/`, so plan models exist only
as device-local rows. The key verifier (`providers/mod.rs`)
has no endpoint for either id and returns `ProviderUnavailable`, so key creation
refuses both. The Settings > Providers section lists only the four key providers
(`PROVIDER_IDS`).

Models are device-local rows. They are merged into the model catalog at read
time (`ModelCatalogCache::set_local_rows`), and the workspace refresh fills them.
Store `claude_code/<id>` or `chatgpt_plan/<id>` as the run model and the provider
id as the run provider. `select_model_by_id` (`clients/da/chat_api.rs`) accepts a
plan row only when its provider fields name the same runtime. A conversation
locked to a plan model whose row is absent gets a Settings hint from
`chat/service.rs`, not the tenant-catalog message. `PlanProviders::credential`
builds a `ResolvedCredential` for each provider, with no DA key lookup. Its
`secret` is empty for both. The ChatGPT adapter takes the current access token
from the session on each call, and the Claude adapter ignores `secret`. The web
client may show such a conversation but cannot continue it; check `xfalcon-web`
before release.

### Consent and data

Claude: the consent notice (`CLAUDE_RUNTIME_COPY`) says that the conversation and
tool results go to Anthropic under the user's own plan. It names the terms that
apply to each plan type: Pro and Max fall under Anthropic's Consumer Terms, and
Team, Enterprise and API use fall under the Commercial Terms. The switch is per
device and off by default, and it is stored in `plan-providers.json`. ChatGPT:
the sign-in is the consent step. After the first sign-in, the notice "You're
using your ChatGPT plan" appears once, with a "Got it" button (`CHATGPT_PLAN_COPY`). The "Manage usage"
link is on the ChatGPT card, not in the notice.

### Security surface

This adds three things the security docs forbid or do not mention: a child
process that is not an app helper (Claude), a listener on loopback (Claude's
declaration server and the sign-in callback), and one new fixed origin
(`auth.openai.com`). Amend `docs/security.md` for each. Add the minimum admitted
commands to `APP_COMMANDS`, `lib.rs`, `build.rs` and the window capability: seven
commands, so `APP_COMMANDS` holds 114. The plan providers add no plugin or core
permission. The default capability's four plugin permissions are
`opener:allow-open-url`, `opener:allow-default-urls` and the two event
permissions. The two window permissions for the header drag and zoom
(`core:window:allow-start-dragging`, `core:window:allow-internal-toggle-maximize`)
come from staging's window chrome and now sit in a macOS-only capability. The CSP
does not change, because all traffic runs in Rust.

## Claude (`desktop-claude-runtime`)

### One stateless `claude -p` per model turn

Each `stream` call starts one `claude` process and ends it when the turn ends.
A spike on `claude` 2.1.285 showed:

- `--tools ""` removes every built-in tool. `init.tools` listed only the tools
  Desktop declared.
- With `--system-prompt` replacing the default prompt, a full turn cost about
  1,000 prompt tokens of overhead, and three model calls took 5.7 seconds.
- Replaying structured history through `--input-format stream-json` does not
  work. `claude` ignored the replayed `tool_use` and `tool_result` blocks and
  reported "Tool result missing due to internal error". Desktop therefore
  renders the history as text in the prompt and sends it on stdin.

Command line, as `build_args` in `providers/claude_runtime/adapter.rs` builds it
(the flags were verified in `claude --help` 2.1.285):
`claude -p --output-format stream-json --verbose --include-partial-messages
--tools "" --strict-mcp-config --mcp-config <mcp.json>`, then
`--allowedTools "mcp__xf__*"` only when the turn declares tools, then
`--permission-mode dontAsk --no-session-persistence --setting-sources ""
--disable-slash-commands --system-prompt-file <system.txt> --model <id>`.
`--mcp-config` and `--system-prompt-file` take file paths. The prompt goes to
stdin, and the working directory is the turn directory.
Do not use `--bare`: bare mode never reads the subscription login.

Before rendering, CR, U+2028 and U+2029 each start a new line, so no content can
hide a line break. A line that would begin with a history marker, after whitespace,
control characters and invisible format characters (Unicode category Cf), has its
first `[` spaced. A person's text therefore cannot forge a history marker.

### Tools are declared, never executed, by a loopback server

A per-turn HTTP server on `127.0.0.1` with a random port and a random bearer
token (`declare_server.rs`) speaks the minimum MCP over Streamable HTTP:
`initialize`, the `notifications/initialized` notification (answered 202),
`ping`, `tools/list`, and `tools/call`. `tools/list` returns the declared tools
of the run. `tools/call` never runs anything. The server holds each `tools/call`
open until the process tree is stopped and the server shuts down. It then
answers with a result marked `isError: true` and the text "handled by xFalcon".
The server answers `GET` with 405, and it refuses a request without the token
with 401. It also refuses a `Host` other than `127.0.0.1:<port>` (403), any
`Origin` header (403), a path other than `/mcp` (404), and a body over 1 MiB
(413).

`claude` calls tools as `mcp__xf__<wire name>`. A per-turn table maps each
wire name back to its `ToolId`, so two tool ids that sanitize to one wire name
fail the turn instead of aliasing. The spike confirmed that `tools/call`
carries the arguments and `_meta["claudecode/toolUseId"]`.

### Stream mapping and the kill rule

`stream_event` lines carry the raw Anthropic streaming events. Map them like the
Anthropic adapter: text deltas to `TextDelta`, thinking to `ReasoningDelta`, and
usage to `Usage`. Emit `ToolCallRequested` for a `tool_use` block only when its
JSON is complete. When an `assistant` message holds one or more `tool_use`
blocks, emit every call, then `Stopped { ToolUse }` and `Completed`. The relay
then stops and the process tree is killed. The held `tools/call` is answered only
after that, so `claude` cannot start a second model call. Nothing ever executes
in `claude`.

A turn that ends with `end_turn` emits `Stopped { Natural }` and `Completed`.
The result line's `modelUsage` reports the context window and output limit of the
session. The stream mapper stores them (`session_limits`) only when that `result`
line arrives before the turn stops. The relay stops reading at the stop, so in
production the limits are never recorded, and no budget check uses them.

The tree is killed before the turn directory is removed. On Unix, `/bin/sh -c
'kill -s KILL -- "-$1"'` kills the process group that the child leads. On Windows,
`taskkill /PID <pid> /T /F` kills the tree. A guard kills the tree synchronously
when the turn's future is dropped before the normal stop, such as a cancelled
stream.

Two time bounds apply. The prompt write and the wait for the first output line
share 60 s. After the first line, each wait for the next line allows 10 minutes of
silence. Both expire as `RuntimeUnavailable`, which the person sees as "Claude did
not start or did not answer in time. In Settings → Models & connections, check that
Claude Code is installed, signed in and switched on, then try again." Neither
expiry is retried. A line longer than 8 MiB is an invalid response.

A failed turn is classified by the text of its `result` line, compared
case-insensitively. "not logged in" or "/login" is a sign-in requirement. "rate
limit" is retried. "usage limit" or "limit reached" is a plan usage limit, which is
never retried. Other text is an invalid response.

### Finding and starting `claude`

Search the process `PATH` first. The name is `claude.exe` on Windows and `claude`
elsewhere. Windows does not use `PATHEXT`. A `claude.cmd` shim is never spawned.
When one sits in a `PATH` directory, the package executable beside it,
`node_modules\@anthropic-ai\claude-code\bin\claude.exe`, is used instead. A quoted
Windows `PATH` entry is unquoted once. Only absolute entries are searched, and an
entry longer than 1,024 characters is skipped. The fixed fallbacks are: on Windows,
`%USERPROFILE%\.local\bin\claude.exe`, the same package executable under
`%APPDATA%\npm`, and `%LOCALAPPDATA%\Programs\claude\claude.exe`; on Linux and
macOS, `~/.local/bin/claude`, `~/.claude/local/claude`, `~/bin/claude`,
`~/.npm-global/bin/claude`, `~/.volta/bin/claude`, `~/.bun/bin/claude`,
`~/.asdf/shims/claude`, `/usr/local/bin/claude` and `/opt/local/bin/claude`. macOS
also probes `/opt/homebrew/bin/claude`. On Unix a file counts only with an execute
bit. When nothing is found on macOS, the login shell is asked for its `PATH`
with `-l -i`, between markers, within 10 s and 64 KiB, and that `PATH` is searched
the same way. A lookup that fails or times out, or whose output lacks both markers,
is a check failure ("Could not check Claude Code. Try again."), never "not
installed". Run `claude --version` (10 s limit) and refuse a version below the minimum,
2.1.295, with the message "Claude Code X is older than the supported 2.1.295.
Update it, then choose Check again." The minimum rose from 2.1.285 for Haiku 5.5
(added in 2.1.293), the `--tools` fix (2.1.295) and a startup fix (2.1.290). Lower
the floor only with evidence. Start the process with no console window on Windows, kill it as a tree,
and cap concurrent processes at two, process-wide. The probes (version and account
checks) are serialized by one mutex and take a permit from the same two-permit cap
as turns.

### Account state without reading credentials

Run `claude auth status --json`. Read `loggedIn`, `authMethod`, `apiProvider`
and `subscriptionType`. Never render or log the other fields (email, org id,
config path). Show the method that `claude` reports. Never set, clear or
override `ANTHROPIC_*` variables, `CLAUDE_CODE_OAUTH_TOKEN` or the config
directory, and never read `.credentials.json` or the keychain. When the user
is not signed in, Settings shows the command `claude auth login` to run in a
terminal. There is no sign-in button.

### Isolation

Each turn runs in a directory the app owns: `turn-<uuid>` under `claude-turns/`.
It holds only `system.txt` and `mcp.json`, and on Unix both are written with
owner-only permissions. Without `--bare`, `-p` loads hooks and `.mcp.json` from
the working directory with no trust prompt, so no project path is ever the
working directory. `--strict-mcp-config`, `--setting-sources ""` and
`--disable-slash-commands` stop user and project configuration from adding
servers, hooks or skills. Desktop's project tools stay Desktop's tools. The child
inherits the user's environment minus every variable whose name starts with
`XFALCON_` (case-insensitive). The app adds `NO_PROXY` and `no_proxy`, so the
loopback declaration server is reached directly.

### Models

Offer three models while the runtime is ready. The picker shows them as Opus,
Sonnet and Haiku. Their ids, `claude-opus-5-5`, `claude-sonnet-5-5` and
`claude-haiku-5-5`, are the ids in Anthropic's model table (`CLAUDE_MODELS`). The
run record carries them with the `claude_code/` prefix. The rows use fixed limits of
1,000,000 context tokens and 128,000 output tokens (`CLAUDE_CONTEXT_TOKENS` and
`CLAUDE_MAX_OUTPUT_TOKENS` in `adapter.rs`). The published table says the same,
and a Team plan session reported the same in the spike. The window the session
reports is read into the stream mapper, but no code uses it yet. Confirm on Pro
and Max before release.

## ChatGPT plan (`desktop-chatgpt-plan`)

### Sign-in, as OpenAI documents it

This follows OpenAI's "Sign in with ChatGPT" token-sharing track, the same flow
t3code runs. t3code then passes the token to `codex app-server`. Desktop sends
the same `POST /v1/responses` call itself, so no `codex` is needed and the tool
loop keeps control.

- Host id: `urn:uuid:<v4>`, generated once per account folder and kept in that
  folder's `meta.json`. It is opaque and is sent only to OpenAI's authorize endpoint.
- New registration: open `https://auth.openai.com/api/accounts/authorize` in the
  system browser with `client_id=dynamic_agent_client`,
  `agent_name_hint=xFalcon Desktop`, `ext_agent_host_id`, `response_type=code`,
  `redirect_uri=http://127.0.0.1:<port>/auth/callback`,
  `scope=openid profile email offline_access resource.invoke
chatgpt.tokens.use.direct`, `resource=https://api.openai.com/v1`, `state`,
  `nonce`, and a PKCE S256 challenge. State, nonce and verifier are fresh per
  attempt. The loopback listener is bound before the browser opens, and it accepts the first
  valid callback. A request whose `state` is wrong, on the listener's own host and
  path, is answered 400 and counts toward a limit of 20 such requests. A host or path
  mismatch, or a request that cannot be read, does not count. An `accept` error from a
  client that reset first (`ECONNABORTED` on macOS and the BSDs) is tolerated, and 20
  such errors in a row end the attempt. The reply page says "Sign-in code received",
  or "Sign-in did not complete" for a failed one.
- Returning account: use the saved issued `client_id`, add `id_token_hint` and
  `login_hint`, and omit `agent_name_hint`. If plan usage was not granted, add `prompt=consent`. Reject a callback that names another
  client id.
- Exchange the code at `https://auth.openai.com/api/accounts/oauth/token`
  (form, no client secret). Verify the ID token against the JWKS in the
  discovery document: signature, issuer, audience (the issued client id),
  expiry (with 5 s of leeway) and nonce. Use `sub` as the identity. Only RS256 and ES256 are accepted. An issued-at time more
  than 5 minutes ahead of the clock is refused. A token with several audiences must
  also name the client in `azp`. Each JWKS key is read on its own, so a key this code
  cannot read is dropped alone, and a key is used only when its `use` is absent or
  `sig`. Require the granted scope
  `chatgpt.tokens.use.direct`; without it, keep the sign-in, disable plan usage,
  run no inference, and say so.

On macOS the browser opens with `/usr/bin/open -- <url>`. The launch counts as
failed unless `open` exits successfully within 5 s. Cancelling a sign-in after its
callback writes nothing.

### Tokens

Each issued client id has one sealed file, `session-<digest>.bin`, in the
account's `chatgpt-plan/` folder. The file holds the ID token, access token,
refresh token, subject, e-mail and scopes. It is sealed with AES-256-GCM under the
account's existing key from the `SecretStore`, so the plan adds no keychain
namespace. Non-secret state (host id, active client id, notice state, and a
profile per client id with its subject and e-mail) is in `meta.json` in the same
folder. Access tokens last one hour and refresh tokens 30 days, and each refresh
rotates the refresh token. A token with less than 60 s of life is refreshed
first, one refresh at a time. The new access token, expiry, scopes and rotated
refresh token are written in one record before the new access token is used. A
token with at least 20 s of life may wait for the `earliest_refresh_at` that the
server gives. Below 20 s the refresh runs whatever the server asked. If that save
fails, the rotated pair is the only valid one, so it is kept in memory and written
by the next save attempt.
Terminal codes are `invalid_grant`, `invalid_refresh_token`, `token_expired`,
`refresh_token_expired`, `refresh_token_invalidated` and `refresh_token_reused`.
A terminal code deletes the sealed file for that client id. The profile stays in
`meta.json`, so the next sign-in starts from it with a login hint, and the status
asks for a new sign-in. A refresh that OpenAI refuses as `invalid_client` also drops
the profile, because the registration is gone, and the next sign-in registers anew.
A network or 5xx failure keeps the sign-in, and the current access token is used
only while more than 5 s of life remain. Sign-out, under the refresh lock, deletes
every sealed session file in the folder, not only the active one, and clears the
active client id. Only the active sign-in's refresh token is then posted to the
revocation endpoint from the discovery document. A 5xx or network failure is retried
after 500 ms and again after 1 s. If revocation is unconfirmed, tell the user to
disconnect the app in ChatGPT Settings. Never log tokens, or authorization URLs
that carry `id_token_hint`.

Reads split into two kinds. `Unreadable` is a keychain or file read failure: the
data is kept, and the status reads as a check failure. `Corrupt` is a decrypt or
parse failure. A damaged `meta.json` is an error and is never replaced silently.
On Unix a save writes a temporary file, syncs it, renames it into place, and syncs
the folder.

### The Responses adapter

`POST https://api.openai.com/v1/responses` with `Authorization: Bearer <access
token>`, `store: false`, `stream: true`, and the whole history in `input`. The
system text is the first input item, a `developer` message, because the route
refuses `role: system` and the `instructions` field. Omit the fields OpenAI lists
as unsupported (`background`, `conversation`, `max_output_tokens`,
`max_tool_calls`, `metadata`, `moderation`, `multi_agent`, `prompt`,
`prompt_cache_retention`, `safety_identifier`, `temperature`, `top_logprobs`,
`top_p`, `truncation`, `user`, `instructions`) and `previous_response_id`. Send
only function tools, with `tool_choice` set to `auto`; never hosted tools. Never
call ChatGPT `backend-api` endpoints.

Map the stream to provider events and process it through `response.completed`. A
refusal streams as text: `response.refusal.delta` text is shown, and
`response.refusal.done` adds nothing.
Treat a stream that ends without `response.completed` as a failure, reported as
unavailable. Map `response.failed` by `error.code`:

- `subscription_sharing_usage_limit_exceeded` (429): stop plan requests, show
  "Usage limit reached" with "Manage usage", and give no estimate or reset time.
  It is not retried.
- `subscription_sharing_usage_unavailable` and `subscription_sharing_user_unavailable`
  (503), and a direct 503: keep credentials and retry with bounded backoff. The
  retry makes three attempts at most, and only before any output.
- `subscription_sharing_user_not_eligible` (403), `subscription_sharing_route_not_supported`,
  `chatpass_v2_scope_not_authorized` and `chatpass_v2_invalid_authorization_context`:
  explain; do not retry.
- `subscription_sharing_unsupported_capability` (400): fail as an invalid response.
  The `error.param` is not read, and the same body is never resent.
- `subscription_sharing_invalid_user` (401): map to a sign-in requirement. The
  adapter does not refresh and retry on this code. The sealed record is removed, the
  profile stays, and the status shows sign-in required (`mark_sign_in_required`). A
  plain HTTP 401 has the same effect. The refresh-first step in the
  original design is not built.
- Never switch to another billing path on any of these.

The request and stream shapes follow OpenAI's open-source Codex client. The unit
tests use inline payloads written in those shapes. The tree holds no recorded
Codex fixture. Live inference needs a plan with sharing enabled; unit tests do not.

### Models

List models with `GET https://api.openai.com/v1/models` and the access token,
keep entries with `visibility == "list"`, and keep the server's order. Offer only
slugs whose limits are documented: `gpt-6-luna`, shown as Luna, and `gpt-6.1-sol`,
shown as Sol. Both have 1,050,000 context tokens and 128,000 output tokens, per
OpenAI's model table (`DOCUMENTED_MODELS`). The display names are fixed by the
desktop, whatever the listing calls them. `gpt-6-astra` is not offered. The request
builder refuses any slug outside that table before a body exists. Hide any other
slug until its limits are documented. The workspace refresh reloads the
list for the signed-in account, and sign-out clears that account's ChatGPT rows.

### Required wording

Follow OpenAI's UI/UX guidelines: the button reads "Continue with ChatGPT"; the
Settings section is titled "Use your ChatGPT plan"; the first sign-in shows
"You're using your ChatGPT plan" with a "Got it" button, once; the composer
shows "Using ChatGPT plan" while a ChatGPT model is selected; "Manage usage" links
to `https://chatgpt.com/settings/usage`. The ChatGPT logo must come from
OpenAI's approved formats. Ship text only until Desktop has the asset. Say that
a Plus plan's five-hour limit is shared across every app that uses the plan. The
usage-limit text differs by provider: ChatGPT's points to "Manage usage" in Settings,
and Claude's sends the person to Claude.

### Frontend behaviour

The ChatGPT card shows `status.error` in every state. The Claude consent text says
nothing about training. The ChatGPT sign-in and sign-out actions report success,
and the section refreshes the workspace only after a success. Focus is moved
explicitly after a change, because WKWebView does not focus a button on click. The
loading text is a `role="status"` region. The welcome notice says "Chats that use a
ChatGPT model run on your ChatGPT plan, separate from xFalcon's own charges." The
composer note "Using ChatGPT plan" stays visible while a ChatGPT model is selected.

## Legal posture

### Anthropic

Source: Anthropic's legal and compliance page, the Agent SDK overview, the
Consumer and Commercial Terms, and the Agent SDK help article of 2026-10-07.

- Run only the unmodified `claude` the user installed. Never bundle or install
  it. Do not remove or restrict any of its sign-in methods.
- Never offer Claude.ai sign-in, read or store tokens, or route requests through
  plan credentials. Sign-in completes in Anthropic's own flow.
- One sign-in per person. No shared sign-in, no paying for or reselling usage.
- iSeyon must have accepted Anthropic's Commercial Terms.
- Label the feature in plain text. No logos. No "Claude Code" in a feature name.
  State that it is not affiliated with Anthropic.
- Open tension: the Agent SDK page says third parties cannot offer Claude
  sign-in or rate limits "unless previously approved", and the help article says
  subscription limits apply to third-party apps "for now". Get written
  confirmation from Anthropic sales before release. Have counsel read the
  Commercial Terms clause on competing products.

### OpenAI

Source: OpenAI's Sign in with ChatGPT pages (token sharing for open-source and
locally hosted apps, sign-in, models and inference, preview limitations,
accounts and sessions, errors, UI/UX guidelines) and the Codex app-server
documentation.

- The token-sharing documents cover open-source and locally hosted apps. A paid
  or remotely hosted app files OpenAI's interest form
  (`https://openai.com/form/sign-in-with-chatgpt-interest/`). Submit it before
  release.
- Use the dynamic registration flow only. Do not read `~/.codex/auth.json`, do
  not start `codex`, and do not use ChatGPT `backend-api` endpoints.
- A valid ID token alone does not authorize plan usage. Check the granted scope.
- Keep the plan separate from xFalcon's own charges in every label.

## Risks and verification

- Vendors can change terms or enforce without notice. Disclose it in the consent
  text. Each provider stays off until its release gate is met.
- A Claude turn costs a process start. Measure it on the Windows mirror.
- Killing the tree can leave a grandchild on Windows. Test with a job object or
  `taskkill /T` and confirm no orphan after cancel.
- macOS execution is unverified until the macOS CI gate runs. Test the
  Dock-launch `PATH` case and the keychain prompt for the account key the plan uses.
- A rendered-text history is less exact than native tool blocks. Compare answer
  quality against the same prompts on the Anthropic adapter.
- `claude` flags can change between versions. Pin the verified version floor and
  surface the version in Settings.
- The ChatGPT stream shapes are tested from inline payloads, not a live plan. Run one
  live turn on a plan with sharing enabled before release.
- OpenAI refresh tokens rotate. A crash between receiving and saving a new
  refresh token would lose the session. Write the record before using the new
  access token, and fall back to a fresh sign-in.
- macOS has never been compiled or run on the development host. The macOS CI gate
  is the first build and run there.
- A `claude` turn that is running when the app quits may finish its turn. Desktop
  has no exit hook that stops it.
- Keychain reads inside the session layer still run on async workers. A keychain
  prompt blocks one worker, not the app. The welcome acknowledgement runs on the
  blocking pool.
- Open release gates: Anthropic's written confirmation; OpenAI's interest form and
  the approved ChatGPT sign-in logo; and one live ChatGPT turn on a plan with sharing
  enabled.
- The unit of `earliest_refresh_at` is unverified against OpenAI's documents.

## Addendum 2026-10-22: reasoning effort, per-chat lock and ChatGPT headers

This addendum adds three changes made after the design above. It supersedes the
count in Security surface: the plan providers add eight commands, so
`APP_COMMANDS` holds 115. The rest of the design above still stands.

### Reasoning effort

Claude runs at a reasoning level chosen in Settings. The levels are `low`,
`medium`, `high`, `xhigh` and `max`, the values the installed `claude` program
takes as `--effort`. The default is `high`, because this app favours accuracy.
Claude's own default for these models is `medium`. `ultracode` is accepted by
`claude` but is not offered, because it switches on Claude Code's own workflow
mode. The setting is the `claude_effort` key in `plan-providers.json`, beside
`claude_enabled`. The file stays at version 1 and at 4 KiB. A missing level reads
as `high`. An unreadable level (an unknown string, a wrong type or null) also
reads as `high`, and it never turns Claude off. A missing, damaged, oversized or
newer file still reads as everything off.

Every `claude` run gets `--effort <level>` as its last two arguments. Probes (the
version check and the sign-in status) pass no effort. The app withholds
`CLAUDE_CODE_EFFORT_LEVEL` from the child process, next to the `XFALCON_*`
variables, because `claude` ranks that variable with `--effort`.

Settings > Claude shows a "Reasoning effort" select, with Low, Medium, High
(recommended), Extra high (xhigh) and Max, whenever Claude is on. The status view
`ClaudeRuntimeStatusView` carries `effort` in every state, including `disabled`.
The command `claude_runtime_set_effort` (input `ClaudeEffortInput`) saves the level
in `plan-providers.json`. It works while Claude is off, does not clear the cached
probe, and does not re-probe.

### Per-chat lock

The first Claude turn of a conversation writes the current setting to
`accounts/<account-dir>/claude-chat-effort.json`. The file is version 1, with a
`chats` array of `{id, effort}` entries, oldest first. It holds at most 2,048
entries, and the oldest is dropped first. Reads are capped at 256 KiB. A damaged
or oversized file counts as empty and is replaced. An entry with an unknown level
is ignored. Every later turn of that conversation passes the stored level,
whatever the setting is now.

Titles and follow-up suggestions are never generated on a plan model, so only chat
runs take the lock. Existing Claude chats lock at the setting in force on their
first turn after the update. The file holds conversation ids and levels only: no
secret, no content, and nothing logged. The account clear removes it, because
`security/account_cleanup.rs` names it in the account-root file list.

### ChatGPT conversation headers

`POST https://api.openai.com/v1/responses` now carries `session-id`,
`thread-id` and `x-client-request-id`, each set to the conversation id. The id is
an opaque UUID, and the app accepts it only if it is 1 to 64 ASCII letters, digits,
`-` or `_`. The headers are sent so that OpenAI can route its prompt cache for the conversation.
OpenAI's Codex client and opencode send the same headers to this route. The request
body is unchanged. `prompt_cache_key` is deliberately not sent, because OpenAI's
published pages do not mention it and nothing confirms that the route accepts it.

What leaves the device grows by one opaque identifier per ChatGPT plan request, and
no conversation content. The effect on cache hits and plan usage has not been
measured live. That needs a signed-in ChatGPT plan that OpenAI has approved for this
app, so the effect is **UNVERIFIED**.
