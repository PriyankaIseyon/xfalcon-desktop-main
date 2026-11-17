# Runtime flows

Version 0.5.42 · reviewed 2026-10-15 · **Understanding** · audience: engineers changing the core

Each section traces one path end to end, naming the function at every hop so the
code can be opened alongside. Structure is in [architecture.md](architecture.md).
Commands and events are in [ipc-reference.md](ipc-reference.md). Environment
variables and files on disk are in [configuration.md](configuration.md). Tool
schemas, policy, and every enforced limit are in
[agent-runtime-reference.md](agent-runtime-reference.md). This file gives the
order of events and does not repeat those tables.

| §   | Flow                                        |
| --- | ------------------------------------------- |
| 1   | Startup                                     |
| 2   | Sign-in and renewal                         |
| 3   | A chat turn                                 |
| 4   | Editing, regenerating, retrying, cancelling |
| 5   | Crash recovery                              |
| 6   | Project trust, read, and mutation           |
| 7   | MCP                                         |
| 8   | Preview, canvas, reports, export            |
| 9   | Sign-out and local wipe                     |
| 10  | A tool call that waits for approval         |
| 11  | Attachments and uploads                     |
| 12  | Events and the renderer store               |
| 13  | Updates, the work barrier, and quitting     |

---

## 1. Startup

`src-tauri/src/main.rs` → `xfalcon_desktop_lib::run()` in `src-tauri/src/lib.rs`.

1. **`install_redacted_panic_hook()`** — before anything else. A panic afterwards
   appends location, thread, and backtrace to the daily log file in `logs/` and
   **discards the payload entirely** (CWE-532). The record always says
   `"xfalcon: internal failure"` (`security/panic_boundary.rs`). Until
   `init_logging` has told the hook where `logs/` is, the record goes to stderr
   only.
2. **`load_workspace_dotenv()`** — debug builds only; reads `.env` from the repo
   root. Compiled out of release builds.
3. **Single-instance plugin** — claimed _before_ the other plugins, so a second
   launch shows and focuses the existing `main` window instead of racing it.
   Order is asserted by
   `security::tests::runtime_claims_single_instance_before_initializing_other_plugins`.
4. **Plugins** — `opener`, `dialog`, `updater`.
5. **`register_uri_scheme_protocol(PREVIEW_URI_SCHEME, …)`** — the `xf-preview`
   origin. See § 8.
6. **Navigation guard** — a custom plugin whose `on_navigation` calls
   `security::capabilities::allows_navigation`. It permits the exact app origin
   (plus the exact dev-server origin when `tauri::is_dev()`) and the `xf-preview`
   origin, which WKWebView raises for every subframe.
7. **`on_page_load`** — registered next. It acts only for the `main` webview.
8. **`.setup()`** in order:
   - resolve `app_local_data_dir()`;
   - `init_logging()` — daily rotating file under `logs/`, 7 files retained,
     plus stderr; `XFALCON_LOG` tunes the filter, default `info`; the appender
     guard is deliberately leaked so buffered lines survive to process exit;
     it also points the panic hook at `logs/` and prunes legacy diagnostic logs;
   - manage `WorkBarrier` and `DesktopUpdateService`, and start the update
     cadence (`update_schedule::spawn`). See § 13;
   - resolve bundled `resources/tenant-config.json` if present and call
     `init_bundled_tenant_config` — a distributed build authenticates against
     its tenant with no `.env`;
   - construct two keyring stores: `ai.xfalcon.desktop` (session and local
     database keys) and `ai.xfalcon.desktop.provider-keys` (provider and MCP
     credentials). Each is wrapped in `CachedSecretStore`, so a device key costs
     one keychain read per launch. An unavailable vault fails startup — there is
     no plaintext fallback;
   - construct `ProjectsService` and `ApprovalsService`, build the
     `AccountCleanup` registry, then `AppRuntimeState` (which owns auth, chat,
     workspace, credentials, reports, and the model catalogue cache);
   - construct `TempCleanup` for the app-owned dataset-staging and upload-snapshot
     directories, manage it, and start its background worker; no cleanup pass
     runs during startup. The worker waits 30 minutes before its first pass, then
     runs every 30 minutes on a blocking worker thread. It claims only unleased
     UUID directories whose children are safe regular files, and removes them
     relative to an opened root handle. It does not touch a shared system-temp
     directory. Upload operations hold leases through active reads/transfers and
     retryable queue states; eager file cleanup is asynchronous and best effort.
     Cancel, queue removal, and account changes share the upload admission guard
     with start registration (`security/temp_cleanup.rs`, `uploads/service.rs`);
   - construct `McpServerRuntime` (the Settings surface's own MCP runtime),
     `UploadService`, dataset `Staging`, and `PreviewDocumentStore`;
   - construct `StartupMigration` and build the main window pointed at its
     `initial_page()` (`storage-migration.html`). The window is created hidden
     (`visible: false`, `create: false` in `tauri.conf.json`);
   - `register_drop_handler` — the dataset drop listener (§ 11);
   - `spawn_acknowledgement_timeout(…, ACKNOWLEDGEMENT_TIMEOUT, || exit(1))` —
     if the renderer never acknowledges the storage migration within 30 s, the
     process exits rather than sitting invisible forever.
9. **`on_page_load`**, when the main webview reports `Finished` —
   `StartupMigration::handle_loaded_url` decides. `Ok(true)` shows the window and
   marks ready; `Ok(false)` logs a warning and waits for the next page; `Err(_)`
   exits 1. A missing migration handle also exits 1 — startup fails closed.
10. **`invoke_handler`** — 115 commands, listed in
    [ipc-reference.md](ipc-reference.md).

After the app page loads, the renderer calls `app_bootstrap`, applies the
session it returns, and starts the background update checks (§ 13).

---

## 2. Sign-in and renewal

`auth_login` → `AuthService::begin_login`. Authorization Code with PKCE, public
client, no client secret.

1. **Config** — `AuthConfig::from_env`. A release build reads only the bundled
   `resources/tenant-config.json`, and only when its SHA-256 equals the digest
   `build.rs` compiled in. A debug build falls back to `XFALCON_DA_BASE_URL`,
   `XFALCON_CASDOOR_ISSUER`, `XFALCON_CASDOOR_CLIENT_ID`, and
   `XFALCON_CASDOOR_ORGANIZATION` (`auth/config.rs`). A second login while one
   runs fails with `AlreadyAuthenticating`.
2. **Discovery** — `auth::discovery::discover_auth` fetches DA's
   `/.well-known/oauth-protected-resource`, takes its first authorization server,
   and rejects it unless it equals the configured issuer (`IssuerNotAllowlisted`,
   reported as `IssuerMismatch`). It then fetches the issuer's
   `/.well-known/openid-configuration` and checks the issuer again. The
   authorization, token, and JWKS endpoints must be HTTPS and on the issuer's
   origin. The client follows no redirect.
3. **PKCE + state** — `auth::pkce` mints the verifier, challenge, state, and a
   nonce. State comparison is constant-time. The nonce goes into the
   authorization URL; no code in `auth/` reads it back.
4. **Loopback listener** — `auth::loopback` binds `127.0.0.1` on
   `XFALCON_OAUTH_LOOPBACK_PORT` (default **19847**), path `/callback`. Request
   reads are capped at 8 KiB, and a peer that is not loopback is ignored. A busy
   port fails with a message naming the override.
5. **System browser** — the authorization URL (`scope=openid profile email`, the
   `resource` and `organization` parameters, `S256`) opens in the OS browser,
   never in the app webview. The navigation guard would refuse it anyway.
6. **Callback** — the first well-formed request ends the wait. State must match
   exactly. The wait lasts at most 300 s (`LOGIN_TIMEOUT`).
7. **Token exchange** — `auth::oauth::exchange_authorization_code` posts to the
   discovered token endpoint.
8. **Verification** — `auth::token::verify_access_token` checks the access token
   as a signed JWT against the issuer's JWKS (issuer, audience = the DA resource,
   owner = the organization, time claims). Then `fetch_chat_identity` asks DA who
   the caller is, and `ensure_identity_matches_token` requires the two to agree.
   The account id is `<tenant_id>:<user_id>` (`canonical_account_id`). If the access
   token has no `nonce` claim, sign-in logs one info line.
9. **Session** — `AuthSessionStore::set_authenticated` keeps the access token,
   the refresh token, and the audience the token was minted for. All three live
   in process memory only. Nothing is written to disk or to the OS vault, so
   closing the app ends the session.
10. **Account scope** — the `auth_login` command binds the account to the local
    stores (`CurrentAccountScope::bind_native`). If that fails, it logs out and
    returns `account_bind_failed`. Then it starts the legacy provider-key
    migration.
11. **Projection** — the renderer receives a `SessionProjection`: display
    identity and an expiry, no tokens. A failure crosses as an `AuthLoginError`
    code; the full message goes to the log only.

**Renewal.** `auth::renewal::ensure_fresh_session` runs before every path that
spends the bearer: chat, workspace status, uploads, reports, and the MCP runtime
set. It acts inside a 60 s leeway window before expiry. A process-wide lock
makes concurrent callers share one refresh. The renewed token is verified like
the first one. The renewed bearer keeps its MCP audience. A refused refresh token
(`invalid_grant`) clears the session; any other failure leaves it in place.

**Generation guard.** `AuthSessionStore` keeps a generation counter, and every
change of who is signed in bumps it. A renewal writes only while the generation is
unchanged (`set_authenticated_if_generation`). So a refresh that finishes after
sign-out writes nothing. A renewed token for a different subject is refused, and
that clears the session. The session-end hook, which clears the audit identity,
runs on `clear()` and when a session replaces an authenticated one. It does not run
on a renewal. See [audit-log.md](audit-log.md#identity-binding).

**Cancel and sign-out.** `auth_cancel` cancels a waiting login and clears the
session. `auth_logout` does the same and then clears in-memory upload, staging,
and model-catalogue state and unbinds the account. It deletes no local data. See
§ 9. A sign-in cancelled after the loopback callback, or during discovery, never
completes. Known and accepted: two concurrent `auth_login` calls can supersede each
other. An `invalid_grant` or identity-mismatch clear can remove a newer session in a
very narrow race.

### Inactivity sign-out

The hook is `src/app/useIdleSignOut.ts`, wired in `src/app/AppShell.tsx`.
The timer limit is a compile-time constant, not a setting (see
[configuration.md](configuration.md#not-configurable-on-purpose)).

1. **Input.** `pointerdown`, `pointermove`, `keydown`, `wheel` and `touchstart` on the
   window record the last real input. Updates are throttled to one per second.
2. **Checks.** A check runs every 60 seconds, and also on window focus, `online` and
   `visibilitychange`. Elapsed time comes from the wall clock, not from counted ticks.
   A laptop that slept therefore signs out at the first check after it wakes.
3. **Postponement.** A running chat run, or an edit or regenerate in progress, postpones
   the sign-out and never cuts it short. An open dialog postpones it only until
   `IDLE_DIALOG_HOLD_MS` (24 hours) after the last real input. After that, an open
   dialog no longer holds it.
4. **Sign-out.** At `IDLE_SIGN_OUT_MS` (12 hours) without input, the hook calls the
   existing `auth_logout` command, the same one as the account menu. The audit
   behaviour of `auth_logout` applies (§ 2 and [audit-log.md](audit-log.md)).
5. **Failure.** If the logout rejects, or the session stays authenticated, an error
   toast shows: "Could not sign you out after 12 hours of inactivity. Close xFalcon or
   sign out from the account menu." The hook retries after `IDLE_RETRY_MS` (5 minutes).
6. **Success.** The sign-in screen shows the info notice "Signed out after 12 hours of
   inactivity. Sign in to continue." It is held in memory by
   `src/features/auth/signedOutNotice.ts`, cleared at the next sign-in, and not saved.

The frontend boundary test lists the hook as an audited timer exception: it uses
`setInterval` only, with no network, no storage and no native calls of its own. The
limit maps to company policy AUTH-04 (automatic session timeout after inactivity).
The policy gives no duration, so 12 hours is a product choice.

**Dev bypass.** `XFALCON_DEV_SKIP_AUTH=1` synthesises an authenticated session
with a `dev-user` / `dev-tenant` identity. Double-gated: `#[cfg(debug_assertions)]`
means the branch does not exist in a release binary, and the variable means it is
inactive in a normal debug run.

---

## 3. A chat turn

The main path. The renderer first makes sure a conversation exists
(`chat_conversation_create` on the first send, or the reserved one — § 11). Then
it draws the sent message, sets `awaitingRun`, and calls `chat_message_send` →
`ChatService::message_send` → `message_send_inner`.

### Admission

1. The command takes a `WorkBarrier` admission, parses up to 64 attachment ids,
   and calls the service (§ 13).
2. Trim. A message with no text and no attachments fails. A message with
   attachments and no text is stored as `Attached files.`. A message longer than
   `MAX_PROMPT_LEN` fails with "Message is too long." `MAX_PROMPT_LEN` equals
   `chat::domain::MAX_MESSAGE_TEXT_BYTES` (512 KiB), so the composer refuses
   exactly what the domain would.
3. One send at a time: a second send while one is in this step fails with "Wait
   for the current message to finish sending."
4. `authenticated_fresh()` — renew if needed. Distinct refusals for _not signed
   in_, _not configured_, and _session expired_.
5. Reject if `MAX_IN_FLIGHT_RUNS` (8) runs are already registered. Parked runs
   count.
6. Open a `DaChatSession` against the tenant DA base URL.

### Model selection

The tenant catalogue comes from `clients::da::model_catalog::ModelCatalogCache`.
`workspace_status` refreshes it on app load and publishes the result. A send
calls `read_through`: it uses the snapshot held for the account and fetches once
only when none is held. A refusal is not held, and neither is an empty list.

`ChatService` then chooses the model:

- For a conversation that exists, `locked_conversation_model` reads the model
  stored on the DA conversation row and re-validates it against the catalogue. A
  conversation with no stored model is locked to Sonnet
  (`set_conversation_model`).
- For a new conversation, `preferred_model` takes the saved per-account default
  (`chat-model.json`, set by `chat_default_model_set`) when the catalogue still
  offers it. Otherwise `select_model` takes the first eligible row in the order
  Haiku, Sonnet, Grok, Luna, Sol. There is no built-in fallback. Plan models are not in that list, so the fallback never picks one. With no eligible row,
  the send fails with "No supported chat model is available."

There is no per-chat model picker. The model for a conversation is fixed when
the conversation is created. Eligibility rules, the offered ids, and the
execution modes are in [agent-runtime-reference.md](agent-runtime-reference.md)
§ Model selection.

`inference_credential` then resolves the provider key for the model. It runs
**before** the user message and the run row are written, so a missing key is
reported at once. `CredentialRuntime::resolve_for_inference` picks the account's
enabled default key for the model's provider (a key DA owns, or a legacy
device-sealed key that it migrates). The secret is held in a `Zeroizing<String>`
for the run. A private model re-reads its catalogue row and binds the exact key
and endpoint on every call.

### Conversation and rows

The renderer creates the conversation on the first send, never on "New chat"
(`chat_conversation_create` → `create_conversation_row`). That call:

- binds the conversation to the **active** project, if any
  (`ProjectsService::bind_conversation`). The binding is fixed from then on
  (ADR-029 in [decisions.md](decisions.md));
- creates the DA row with the preferred model and a title derived locally from
  the opening message (`chat::title::derive_title`, 32 chars), and records the
  title source as `Derived`.

If `message_send_inner` gets no conversation id and finds no active conversation,
it creates the row itself, with no project binding. The shipped renderer does not
take this path.

Then `message_send_inner`:

1. Refuses the send if this conversation already has a registered run.
2. Claims the one-time title upgrade. `spawn_title_upgrade` later asks the run's
   model for a 3 to 6 word title (30 s limit) and replaces the derived title
   unless the person renamed the chat. A user rename always wins.
3. For an edit or a regeneration, cuts the conversation first (§ 4).
4. Writes the user message to DA with a fresh idempotency key, so a transport
   retry cannot write it twice. DA reports how many attachments it linked. If
   that count differs from the ids sent, the send fails and no run starts.
5. Creates the DA run (`create_run`).
6. Registers the run in `run_cancels` **before** any further wait, so a sidebar
   refresh does not finalize it as a zombie.
7. Reconciles the conversation and emits `snapshot_replaced`, then emits the
   conversation-scoped `run_state_changed(Queued)`. That event binds the run to
   the view before the first delta can arrive.
8. Spawns the run (`spawn_local_run`) and the title upgrade, and returns
   `ChatSubmitResult::submitted(conversation_id, run_id)`.

After a successful send with attachments, the command forgets those ids in the
upload queue.

Follow-up chips use a separate command. `chat_follow_ups` asks the
conversation's own model for up to three short next messages (6 s limit, replies
over 4 KiB refused) and returns them without writing anything.

### Run assembly

`spawn_local_run` runs in a task. Each waiting step before the loop races the
run's cancellation token (`prepare_run_step`, cancel checked first). A cancel
there emits the cancelled conclusion, cancels the DA run, and runs the normal
teardown (`finish_run`). In order:

1. `McpServerRuntime::runtime_set` — renews the session, then returns the cached
   set of MCP servers or builds a new one (§ 7).
2. `read_context_snapshot` — DA's authoritative context for the conversation.
   The 200-row history endpoint is never an inference fallback.
   `chat::history::context_from_snapshot` turns the rows into context messages.
   A tool call keeps its place only when its answer exists. A call cut off from
   its answer is answered in place with "No result was produced for this call."
3. `ContextCapacity::new(context_length, max_output_tokens)` and the per-run
   tool-result budget (`ToolResultBudget::for_context_tokens`). Both come from
   the model's catalogue row.
4. `chat::attachments::attach_files` — puts each turn's files back on the turn
   they came with (§ 11).
5. `Run::new` with `RunBudgets` (wall clock `RUN_WALL_CLOCK`, 60 min).
6. The enabled skills of the run's own account (`SkillStore::enabled`) and the
   exact tool-result payloads the snapshot admits.
7. `agent::tool_session::build_tool_session`.
8. `agent::prompt::build_system_prompt` (below), then `LocalTurnDriver::new`.
9. `ToolRun::new(...).drive(Vec::new())`.

### Tool session

`build_tool_session` composes what this run may use:

- the conversation's **bound** trusted project contributes the six `project.*`
  tools. The project must still open (its root identity is re-checked). The index
  is refreshed first. If the refresh fails, only the two search tools are
  withheld;
- `__internal__get_skill` is added whenever any skill is enabled;
- `__internal__read_tool_result` is added when the saved context holds at least
  one ready tool-result payload;
- every reachable MCP server contributes its catalogue, namespaced
  `mcp.<server-slug>.<tool>`. A tool the server marks for its own MCP Apps view
  only (`_meta.ui.visibility` without `model`, such as DA's
  `ida_upload_dashboard_thumbnail`) is left out: desktop hosts no such view.

Dotted ids are aliased to provider-legal wire names (`^[a-zA-Z0-9_-]{1,64}$`) by
`wire_tool_name`; `ToolSession::tool_ids` is the authoritative way back. An
unreachable MCP server degrades to the tools that _are_ reachable, and its name
goes into the prompt. A run with nothing registrable is still an ordinary run:
the provider gets no tool declarations, and the same turn driver handles
compaction and recovery. There is no separate tool-free path.

### The loop

`ChatToolRun::drive` wraps `agent::engine::run_with_tool_loop` in a watchdog
(`chat/tool_run.rs`). The run state machine:

```text
Draft ──> Queued ──> Running ──┬─> Completed
                               ├─> Failed
                               ├─> Cancelled
                               └─> AwaitingApproval ──┬─> Running
                                                      ├─> Cancelled
                                                      └─> Failed
```

`Interrupted` and `Recovering` also exist. A live run never enters them; see § 5.
The full transition table is in
[agent-runtime-reference.md](agent-runtime-reference.md) § Run states.

Every transition is appended to the run's journal file under
`local-projections/run-journal/` (`RunJournalStore`, through
`PersistingRunJournal`) **before** it is acted on. The write fails the
transition if the disk rejects it. After the disk accepts it, the in-memory
journal emits `xfalcon://chat-run-event`. Each iteration:

1. Cancellation check. Then `run.ensure_within_deadline()`. A missed deadline
   transitions to `Failed` first and only then returns the error. Then a check
   for a refused MCP sign-in (`authorization_refused`), which also fails the
   run.
2. `turn_driver.next_turn(previous_outcomes, &cancel)` streams one provider turn
   (see Streaming below). The turn is raced against the run deadline
   (`drain_on_deadline`): at the deadline the engine cancels and waits for the
   step to drain, so no in-flight write is dropped.
3. `ModelTurn::Final` → `Completed`, return.
4. `ModelTurn::ToolCalls(calls)` → `tool_loop.run_round(...)`, also raced against
   the deadline.
   - If the round **paused**, the driver is still handed every outcome from that
     round (`record_outcomes`) before the run transitions to `AwaitingApproval`
     and returns. An assistant turn with unanswered tool calls is rejected
     outright by OpenAI and Anthropic on the next request, so the answers must
     survive the pause.
   - Otherwise the outcomes seed the next `next_turn`.
5. Any `ToolLoopError` (round cap, round timeout — both reachable through
   ordinary model behaviour) transitions to `Failed` before returning. Calls
   beyond the parallel limit are refused, not failed: they get
   `{"status":"denied","reason":"parallel_call_limit"}` and the run goes on.

When `run_with_tool_loop` returns an error other than a cancel, `ToolRun`
calls the driver's `abandon`, which writes the terminal DA record so the DA run
is never left non-terminal. For a cancel it flushes the finished tool results
instead, and the cancel path owns the terminal write.

**Watchdog.** `ToolRun::drive` stops a run that makes no progress. It polls every
2 s. A run with no provider, tool, or durable-write progress for 300 s
(`RUN_STALL_LIMIT`), or alive past 61 min
(`RUN_HARD_LIMIT`), is cancelled and given 20 s (`STOP_GRACE`) to save what it
finished. Then it is dropped, the person is told to retry, and the run goes to
`Failed`. A cancel that does not unwind within three grace periods is cut the
same way.

Bounds (8 parallel calls, 256 rounds, 120 s per call, 300 s per round, and the
rest) are in [agent-runtime-reference.md](agent-runtime-reference.md) § Limits.

### One tool call

`ToolLoop::prepare`, in order:

1. Replay. A repeated invocation id returns its recorded outcome; an id reused
   for a different tool is denied (`invocation_id_tool_mismatch`). A repeat of an
   id still awaiting approval reports `AwaitingApproval` again.
2. Cancellation → not a real attempt; no invocation history, no budget spent.
3. Descriptor lookup in the **effective registry** — the global registry pruned
   to the run's allowlist, further narrowed by an active skill's scope. A miss is
   denied `tool_not_registered` and is not journaled.
4. Argument size, then **schema validation** against the tool's advertised JSON
   Schema, deliberately _before_ `ToolInvocation::open`, because the invocation
   state machine has no `Requested → Failed` edge. Both answer
   `invalid_arguments`, and the run goes on.
5. Budget accounting.
6. `ToolInvocation::open` — journaled.
7. `PolicyEngine::authorize`. Every field of the `PolicyRequest` comes from
   run-scoped Rust-owned state or the current call's own `tool_id`. Never from
   `arguments`. Never from a prior tool result. `agent::tests::tool_loop::`
   `prompt_injected_tool_result_cannot_alter_a_later_policy_decision` proves it.
8. Decision:
   - `Allow` → approve, start, execute.
   - `RequireApproval` → if standing consent covers this tool for this account,
     mint the same invocation-scoped token a click would have produced, journal
     and audit a normal approval, and proceed. Otherwise park the invocation in
     `pending_approval` and return `AwaitingApproval` (§ 10). **The product
     default is auto-approve for all tools**; `XFALCON_REQUIRE_TOOL_APPROVALS=1`
     restores ask-every-time. See ADR-008 in [decisions.md](decisions.md).
   - `Deny` → journal the denial with a stable reason code.
9. The calls that reached `Running` execute concurrently, each under its own
   timeout, and the round has an outer timeout. Results are then finalized one
   by one. Execution routes by tool id through `CompositeToolExecutor` to the
   exact-result executor, the skill dispatcher, the MCP dispatcher (ids starting
   `mcp.`), or the project dispatcher.
10. The turn driver renders each outcome as a JSON body for the model
    (`render_tool_outcome`) with a `status` of `completed`, `invalid_arguments`,
    `denied`, `failed`, `cancelled`, or `awaiting_approval`. A completed result is
    cut to a preview that fits the run's result budget. The preview says whether
    it is complete and names `__internal__read_tool_result` when it is not.
    Truncation is recorded on the result, never silent.

### Streaming one provider turn

`LocalTurnDriver::next_turn` is the model side of one round.

1. Fold the previous round's outcomes into the context, answer any dispatched
   call that has no outcome, and write the finished round to DA
   (`flush_pending_round`, see Persistence). A failed write fails the run.
2. A native-compaction run probes its route once before its first request.
3. Fit the request to the model's window (Context fit, below).
4. Set the `<run-status>` turn: rounds used so far, plus any one-shot notice
   (an output-limit continuation, an unfinished checklist).
5. `agent::local_executor::stream_turn` starts the adapter on its own task and
   drains its events through a channel. For each event it assigns the next
   sequence number and does two things:
   - **Durable mirror.** Text and thinking deltas, `run.start`, and
     `run.complete` become DA run-event records. They go out in batches of 20
     records or every 250 ms.
   - **Presentation.** The event enters the coalescing sink. The sink forwards
     buffered text every 16 ms or at 4 KiB. A paint timer flushes it when the
     provider goes quiet. The sink emits `xfalcon://chat-inference-event`.
6. The turn ends in a normalised stop reason. A refusal and a context-limit stop
   with no tool call fail the run with a fixed message.
7. A turn cut off by the output limit is not final. The driver keeps the prose,
   sets a notice, and asks again, at most 3 times in a row. A tool call cut off
   mid-write is discarded and the model is told to send smaller pieces. If a cut
   tool call repeats past the limit, the run fails with a message that says so.
   Otherwise, after the third time, the answer ends with a fixed "cut off" line.
8. If the model ends its reply while its own `project.update_plan` checklist has
   open items, the driver sends it back to work, at most 3 times per run.
9. A turn with no tool call finalizes the DA run as `completed` with the turn's
   own prose as the message. A turn with tool calls hands them to the loop.

The provider adapter for the model (`adapter_for_selected_model`) is one of the
four first-party adapters wrapped in a retry layer, or the guarded single-attempt
adapter for a private model. A turn that has emitted nothing may be retried on a
rate limit or an overload. A turn that has emitted anything is never retried. Timeouts and
other provider limits are in
[agent-runtime-reference.md](agent-runtime-reference.md) § Limits.

### Context fit

The context builder admits every message with no cap of its own. The window is
managed in tokens by the turn driver, before each request and in this order. The
numbers are in [agent-runtime-reference.md](agent-runtime-reference.md) § Context.

- **Native compaction** (the Sonnet product model). If the request does not fit
  the hard input, the driver sends the longest complete history prefix that fits
  as a request that must compact. It emits `context_compaction` (active), saves
  the returned block to DA (`activate_compaction`), and repeats, at most 8
  passes. The provider may also compact on its own at its trigger, which the
  driver saves the same way and then asks again. A conflicting save reloads the
  snapshot.
- **Bounded context** (every other model, private models included). From 75% of
  the hard input, the driver first replaces old tool results with a stub. If that
  is not enough, the run's own model writes a summary of the oldest history. After
  2 unusable summaries it stops asking. A request that still does not fit fails
  the run. It is never cut blind.
- **Provider overflow.** If the provider says the prompt is too long, the allowed
  input shrinks and the history is compacted again, at most 2 times.

System messages and the stable prefix are never compacted away. A run that
compacted keeps working with the saved block on its next message, because the
block lives in DA's context snapshot.

### System prompt

`agent::prompt::build_system_prompt` assembles, in order:

```text
DEFAULT_SYSTEM_PROMPT                                  always, first
  + <conversation_guidance>…                           only if a caller supplies it
  + <local_project>…                                    when the run has project tools
  + <active_skills priority="high">…                    when any skill is enabled
  + <visualization_contract>…                           always
  + <capability_catalog>…                               when skills or user-facing tools exist
  + <available_tools><inventory>…                       when the run has tools
  + temporarily unavailable server names                when applicable
```

Conversation guidance is appended after the application prompt and escaped as
text. It is subordinate to application rules and the current user request; the
production caller currently supplies `None`. Project names, skill names/sources/
descriptions, tool/server metadata, and unavailable-server names are flattened
to one line and escaped before they enter prompt structure. Descriptions are
bounded; malformed tool names use the safe call alias. Their names and one-line
descriptions remain available for user-facing capability listings. Tool results,
skill bodies, attachments, project files, and saved documents remain evidence,
not authority. If the user asks to follow a document, its relevant steps are task
guidance only within the user's request and application rules.

The desktop keeps its own prompt text and tests the assembled contract; current
fragments are no longer byte-for-byte copies of the reference web prompt. The
visualization contract is always included. It teaches the DA report tools and the
project tools this app has, and `agent::tests::prompt` pins what it must say. The
sections are described in
[agent-runtime-reference.md](agent-runtime-reference.md) § System prompt assembly.

### Persistence

Four writes keep a run durable.

- **Run events** to DA, as above.
- **Tool rounds.** `flush_pending_round` writes each finished round to the
  conversation: the assistant message that requested the calls, a tool-call row
  per call, and one tool-result message per call. Each completed result is also
  stored on DA as an exact payload (prepare, upload, finalize), so a later run
  can read ranges of it with `__internal__read_tool_result`. A round that ends a
  run (cancel, round cap, deadline) is flushed by the driver's `abandon` or the
  cancel path.
- **The terminal record.** `finalize_completed` or `finalize_failed` writes
  `completed` or `error` to the DA run. A competing terminal write already on DA
  (typically a user cancel) is treated as a cancel, not a failure.
- **The local projection.** After the run ends, `finish_run` reconciles the
  conversation from DA into the encrypted local projection
  (`chat_projection.sqlite3`, AES-256-GCM, per-row associated data binding
  account, conversation, entity kind, entity id, and sequence). It tries up to
  three times (0, 200, and 500 ms) and emits `snapshot_replaced`. Then it
  deletes the run journal if the run is terminal and releases the run's slot.

The run's ending reaches the renderer through `emit_run_conclusion`: `completed`
for a finished or cancelled run, and `failed` with a message that names what ran
out and what to do for any other error (`run_failure_report`). The turn driver
and the watchdog emit their own `failed` events, so those cases add none.

Opening a conversation is two commands, and the renderer starts both together.
`chat_conversation_preview` serves the sealed local snapshot with no network, so a
known transcript draws at once. `chat_conversation_open` reconciles with DA and
emits the result. Only when DA is unreachable does it fall back to the local copy,
marked `Stale`. ADR-005 in [decisions.md](decisions.md) gives the reason. Sending
needs DA.

---

## 4. Editing, regenerating, retrying, cancelling

| Intent               | Command                   | Behaviour                                                                                                                                                                                                                                                                                                                                                                                                                              |
| -------------------- | ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Edit a sent message  | `chat_message_edit`       | The target must be a user message. A run this process drives in that conversation blocks the edit with the normal send guard. The service cancels on DA any other in-flight run of the conversation, cuts the conversation at that message (`truncate_conversation`), reconciles, deletes the journals of runs the cut removed, then runs the normal send path with the new text (`TurnRevision::Replace`). No attachments are resent. |
| Regenerate an answer | `chat_message_regenerate` | Finds the user message before the target, cuts at the message after it, and runs the stored prompt again (`TurnRevision::Rerun`). No new user row is written.                                                                                                                                                                                                                                                                          |
| Retry a failed run   | `chat_run_retry`          | Loads the run from DA. A run that is already `completed` only reconciles the view and reports settled, so no answer is written twice. Otherwise it sends the run's input again through `message_send`, which writes it as a new user message and starts a new run. Attachments are not resent.                                                                                                                                         |
| Cancel               | `chat_run_cancel`         | See below.                                                                                                                                                                                                                                                                                                                                                                                                                             |

Edit and regenerate run optimistically in the renderer: `beginTurnRevision`
removes the cut turns from the view at once and keeps a restore point, which
`rollbackTurnRevision` replays if the command fails. Send, edit, regenerate, and
retry take a `WorkBarrier` admission. Cancel does not.

**Cancel.** `ChatService::run_cancel` has two paths.

- This process drives the run. It cancels the run's token. If the run is parked
  on an approval, it takes the run out of the parked map and abandons it, which
  **declines every pending invocation first**, then writes the terminal DA
  record. Otherwise the two state machines diverge and those invocations stay
  `AwaitingApproval` forever. Then it calls `cancel_run` on DA.
- This process holds no handle. It loads the run from DA, cancels it there unless
  it is already terminal, and reopens the conversation to refresh the view.

A running cancel unwinds like this: the engine sees the token at its next
check (the top of the loop, or after the turn or round returns), moves the run
to `Cancelled`, flushes the finished tool results, and `emit_run_conclusion`
sends `completed` with a cancelled finish reason. The provider stream and running
tools receive the token (a child token for the stream), so they stop without a
poll. Run-assembly steps race the token directly (§ 3).

---

## 5. Crash recovery

Nothing resumes a run silently. Recovery happens in `ChatService::workspace_bootstrap`
(the `chat_workspace_bootstrap` command), which the renderer calls on load and on
every sidebar refresh. For each of the 8 most recent conversations
(`STARTUP_RECOVERY_CONVERSATION_LIMIT`), plus any older one whose local copy
holds an in-flight run, it reconciles with DA, 8 at a time, and then looks at
every run that is still in flight:

1. **Live run.** A run this process drives (registered in `run_cancels` or
   parked) is skipped. It is streaming right now.
2. **Foreign run.** A run with no entry in this device's run journal was started
   by another client, such as xFalcon Web. It is left alone. Cancelling it would
   stop a live answer on the source of truth.
3. **Abandoned run.** For any other in-flight run,
   `chat::cold_start_recovery::interrupted_run_patch_from_journal` reads the
   persisted transition log. It applies `agent::recovery::decide_from_run_state`
   to the last state in the log (`replay_policy_for_journal`). When the decision
   is anything other than `Fail`, it patches the projection run to `Interrupted`,
   so a retry cannot duplicate assistant output.
4. The bootstrap remembers that run for this boot (`recovered_runs`) and shows it
   as the conversation's interrupted run, with the notice "This response was
   interrupted before it finished. You can retry when ready." Only the open
   conversation gets the notice.
5. `finalize_abandoned_runs` then cancels the abandoned run on DA and patches the
   local run to `Cancelled`, so the run does not raise the banner again on the
   next launch.

The person retries with `chat_run_retry` (§ 4).

| Scenario                              | Decision    |
| ------------------------------------- | ----------- |
| Queued, never started                 | `Resume`    |
| Running, awaiting provider            | `AwaitUser` |
| Provider terminal, DA state uncertain | `AwaitUser` |
| DA committed, local pending           | `Resume`    |
| User cancelled                        | `Fail`      |

By run state (`decide_from_run_state`): `Draft` and `Queued` → `Resume`;
`Running`, `Recovering`, `Interrupted`, and `AwaitingApproval` → `AwaitUser`;
`Completed`, `Failed`, and `Cancelled` → `Fail`. In the shipped code a `Resume`
decision also produces only the `Interrupted` patch above. No code restarts a
run from its journal.

A run paused on an approval lives only in memory. Quitting loses the pause, and
the tool never runs. The journal still ends in `AwaitingApproval`, which maps to
`AwaitUser` (pinned by `chat::tests::tool_run::a_run_paused_for_approval_is_recovered_as_needing_the_user`).

A cancel during run assembly normally cancels the run on DA and tears it down. If
that DA write fails, the run is not reconciled. Instead the journal gets an
`Interrupted` transition and the person sees a failure that asks them to reopen
the conversation, so the next bootstrap finds evidence to recover from.

---

## 6. Project trust, read, and mutation

### Trust

Rust opens the folder picker. `project_pick_folder` runs the native dialog and
returns an opaque grant id (`src-tauri/src/path_grants.rs`). The renderer
passes that id to `project_create`, and Rust resolves it. The renderer never names a
path. Trust is decided in Rust by what `ProjectsService::create` does with the
resolved folder: `TrustedRoot::establish`
canonicalises the directory and records its stable platform identity (volume
serial + file index on Windows via `GetFileInformationByHandle`; device + inode
elsewhere). `TrustStore::create` seals the row into `projects.sqlite3` (in
`project-data/`) under the `xfalcon-project-trust-v1` associated-data domain. The
new project becomes the active project.

Every list or activate **revalidates the root fresh**. A cached "still valid"
flag is never trusted. `project_find_by_folder` compares platform identities, not
path text, and neither grants trust nor changes the active project.
`project_deactivate` clears the active project and keeps trust and bindings.
`project_revoke` removes the trust record and cancels in-flight project work.

### Containment

All content access goes through `TrustedRoot::resolve_existing` / `resolve_new`,
which are `cap-std` handle-relative. `Dir::open_ambient_dir` turns the trusted
root into a handle every later open resolves against, so a symlink, junction, or
reparse point planted inside the root — even one that appears mid-resolution,
defeating a naive canonicalise-then-`starts_with` check — cannot resolve outside
it. `..`, absolute-path injection, and platform-unsafe path forms are rejected.
No other module reaches project content by any other route.

### Index and search

`ProjectsService::refresh_index` runs at the start of every run that has project
tools. It is serialized per project. The parallel walker from the `ignore` crate
(ripgrep's traversal engine) _proposes_ candidate relative paths; every candidate is independently
re-validated through `path_policy::RelativeProjectPath` and `TrustedRoot` before
any content, size, or hash is recorded. Propose, then re-validate. Records are
sealed under the `xfalcon-project-index-v1` domain in `project-index.sqlite3`.
`sensitive.rs` compiles the built-in secret-file deny rules and is consulted by
both read and search _before_ any content crosses the boundary. The classifier
can take extra configured globs, but no production code supplies any.

### Mutation

```text
model calls project.propose_patch
  └─ tools::dispatch::run_propose_patch
       · refuse paths in .git or .xfalcon and anything the secret classifier flags
       · refuse a whole-file rewrite of a file bigger than one read page
       · keep the file's line endings on a whole-file rewrite
  └─ ApprovalsService::propose_reporting_for_account
       └─ MutationPlan::propose_reporting
            · resolve every operation through TrustedRoot
            · check every operation and report all problems together
            · blake3 hash each base and proposed file
            · build the unified diff, compute the risk tier
            · content-address the plan id over every hash
       └─ the plan is stored encrypted as pending
  └─ ApprovalsService::apply   (no review step: trusting the folder is the consent)
       └─ ApprovalsService::resolve_for_account
            · claim the plan row by compare-and-swap, so one caller wins
            · expiry check
            · approvals::policy::require_sufficient_confirmation
                — the ONLY production constructor of ApprovalProof
                — pub(in crate::approvals): compiler-enforced
            · if every operation's result is already on disk (a crash after the
              write, before the status update), mark the plan executed
            · capture_overwritten_files
                — a revision of every modified, deleted, or rename-replaced
                  file; one over 64 MiB fails the plan
            · projects::writer::execute
                — re-validate containment and root identity
                — re-check every base hash; a conflict fails without writing
                — stage a same-directory temp file for every write
                — apply the operations in order, each one atomic
                — fsync each file; on Unix, also the parent directory
            · set the row to executed, executed_durability_unconfirmed, or failed
```

A plan of several files has no cross-file transaction. The writer checks and
stages every operation before it applies any. If an operation fails during the
apply phase, the ones already applied stay applied, the rest are not applied,
and the error names how many completed. The tool result lists what landed.

The risk tier (a delete, a rename onto an existing path, binary content, more
than ten distinct paths, or more than 1 MiB total) is computed once, immutably,
by `MutationPlan::propose`, and recorded with the plan. It no longer gates a
person: `apply` passes the plan's own confirmation flag, so an agent patch
applies within the same tool call. The revision of every overwritten or deleted
file is how an unwanted change is undone. The folder trust sheet says so before a
folder is trusted.

A person's own edit in the file viewer takes `project_save_file`, through
`ApprovalsService::save` and the same `apply` path: planned, journaled, audited,
backed up, and containment-checked. A report saved into a project (§ 8) and a
revision restore take the same path.

`project_file_revisions`, `project_file_revision_diff`, and `project_file_restore`
expose the revisions (`project-data/shadow_backups.sqlite3`). A restore is itself
a save, so it keeps the file it replaces as a new revision.

### Deleted folders

On macOS and Linux, the open root handle shows when a trusted folder was
unlinked, and the project is marked deleted. Windows never confirms a deletion
this way: a pending delete is released and the project and its chats stay.
`project_list` first runs
`ChatService::cleanup_deleted_projects`. It deletes the project's bound
conversations on DA and locally, then revokes the project. Failures keep the
trust and bindings so the next list retries. See
[project-deletion-audit.md](project-deletion-audit.md).

---

## 7. MCP

```text
server_resolver::resolve            bundled tenant endpoint: <DA base URL>/mcp
registry::classify                  DA's shared mcp_servers rows → dialable
registry::validate_endpoint         https · no userinfo · no query/fragment
                                    · no private/loopback host outside debug
auth / oauth                        bind this device's credential or grant
client::connect_with_reconnect      initialize, then assert the negotiated
                                    revision == V_2025_11_25 exactly
catalog                             discover and validate the tool catalogue
runtime::McpRuntimeSet              one session per server per app, shared
                                    across runs, reconnected on demand
```

`McpServerRuntime::runtime_set` builds the set a run uses. It reuses the cached
set while the dialable rows, the device's grants, and the bundled switch are
unchanged. It keeps the bundled session across a registry change. The bundled
tenant server is held only while the device preference `bundled_enabled` is on.
A registration deadline of 20 s per server applies when the run gathers tools.

`stdio` and `other` transports name no remote endpoint and are deliberately not
dialed. A row registered elsewhere with one of those is **listed with an
`McpServerUnavailableReason`, not hidden** — the person can see why it is
unusable here.

**Hardening `rmcp` does not provide.** The stock reqwest transport buffers
response bodies with no size bound. This build supplies a bounded
`StreamableHttpClient`: no redirects, 10 s connect / 30 s read / 130 s total, a
`Content-Length` pre-check _and_ a live streaming-chunk accumulation cap at
8 MiB — never trusting `Content-Length` alone.

**Risk.** MCP tools carry no risk classification this app defines, only the
protocol's optional annotation block. Those annotations never decide anything on
their own: `ToolAdmission` combines them with the server's DA-registered
`permission_mode` (`allow` / `ask` / `deny`, default `ask` including when DA's
column is null), and the default in every direction is the one that requires
approval — including for a tool claiming to be both read-only and destructive.
Whether that approval becomes a prompt depends on standing consent (§ 10).

**OAuth.** Per server _and_ per device. `mcp_server_sign_in` runs this order
(`mcp/oauth/flow.rs`):

1. Discover the server's own protected-resource metadata and refuse it if it names
   another resource.
2. Take the authorization server from that metadata only.
3. Bind the loopback listener on `loopback_port + 1` (default 19848).
4. Pick a client identity, in this order: this device's own client for the server
   (typed just now or stored earlier), the client a previous grant for the same
   issuer used, the packaged fallback (`XFALCON_MCP_CLIENT_ID` or the tenant
   config's `mcp_client_id`, offered only to the tenant's own issuer), then
   dynamic registration when the authorization server offers it. Without any of
   them, sign-in fails with `client_registration_unavailable`, and the renderer
   asks for client details.
5. Open the system browser, wait for the callback (300 s), and exchange the code
   with PKCE and the `resource` indicator.
6. Seal the grant into this device's store. It is never written to the shared
   registry.

When a grant key is configured, the grant goes to DA instead, as AES-256-GCM
ciphertext, so a sign-in on either product signs in on both. The key comes from
`XFALCON_MCP_GRANT_KEK`, or else from `PROVIDER_KEYS_ENCRYPTION_KEY`, the name
`xfalcon-web` uses (`mcp/oauth/envelope.rs`).

**Re-auth mid-run.** When a call finds the server's credential refused,
`McpToolDispatcher::on_refusal` forgets the credential, then emits
`mcp:reauth-required` once per server per run, so the person is asked while the
run is still in front of them. The bundled server has no DA row, so its
credential is forgotten and nothing is emitted. The call fails without a retry
and sets `authorization_refused`. The engine then fails the run before the next
model turn: "MCP server sign-in expired. Reconnect it, then retry." The
renderer's `McpReauthPrompt` is a modal. Reconnect runs `mcp_server_sign_in`, then
verifies with `mcp_server_test` before it closes.

---

## 8. Preview, canvas, reports, export

**Preview.** `preview_document_register(kind, content)` stores an HTML or SVG
document in a bounded, process-lifetime, in-memory store and returns a
**content-addressed** id — identical content re-registers to the same id, so
re-renders are free. A document from the renderer may hold up to 2 MiB; a
project file registered by `project_preview_register` may hold up to 32 MiB. The
store holds 256 MiB in all and evicts the least recently served first.
`previewDocumentSrc` resolves the platform-correct URL
(`http://xf-preview.localhost/<id>` on Windows, `xf-preview://localhost/<id>`
elsewhere). The scheme handler serves the document with a per-kind CSP:

- HTML — `default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; img-src data: blob:; font-src xf-preview://localhost http://xf-preview.localhost data:; connect-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors http://tauri.localhost tauri://localhost http://127.0.0.1:1420 http://localhost:1420`
- SVG — same, but `script-src 'none'`

Inline script is the point for HTML: previewed documents are self-contained by
product contract. Every network avenue is dead, so a script cannot exfiltrate.
The preview frame is sandboxed without `allow-same-origin`, so the document runs
on an opaque origin, which is why `font-src` names the preview origin and not
`'self'`. See ADR-010 in [decisions.md](decisions.md).

A route `/fonts/<file>` serves the three bundled product faces so the injected
zero-specificity base stylesheet can reach them. An id that was evicted answers 404. The handler injects the base style at display time and never changes the
stored bytes.

**Canvas.** A transcript card offers to expand into the side panel
(`features/canvas/`): a virtualised `DataGrid` with sort, filter, and CSV export
with formula neutralisation; a hand-drawn `ChartCanvas` with five chart types
(bar, line, area, scatter, pie); a full-width `HtmlCanvas` document frame. The
canvas keeps the source of each HTML tab, because a preview id can be evicted and
the frame cannot report that. Numeric columns are classified once, over the whole
column, by the card that produced the table — a grid left to guess would sort
"100" before "50".

**Reports.** A saved report is a DA report kit, and DA is the only place one is
created, changed, or deleted. The assistant makes those calls with DA's MCP tools
(`ida_save_dashboard`, `ida_refresh_dashboard`, `ida_recall_dashboard`,
`ida_list_dashboards`, `ida_delete_dashboard`); the visualization contract in the
system prompt tells it which tool serves which request and what arguments DA
validates. Desktop has no command that uploads a report.

The Reports destination reads and manages kits over DA's REST API
(`/api/v1/user/report-kits`) from Rust, with the user's bearer, and keeps no copy.
Every operation needs a signed-in session, and a list or open whose answer arrives
after the account changed is refused rather than handed to whoever is signed in now:

- `report_list` drains DA's pages (100 per page, at most 200 pages, within 120 s) with
  `include_hidden=true`, and returns every kind (dashboard, workbook, presentation) with a
  `hidden` flag. A page that comes back empty while DA promises more, a
  count or total that keeps changing across three passes, or a cap that runs out is an
  error: a failed read is an error state with Retry, never a shortened or stale list.
  It also returns `project_origins`: for each report this device knows was created in a
  project chat, the project's id and current name. DA keeps no such fact for a report an
  assistant saves, so the link is device-local (below). A local store that cannot be read
  costs the links, not the library: the list is returned without them and the failure is
  logged.
- **Project link.** In a run whose conversation is bound to a project, the tool executor
  watches the answer to `ida_save_dashboard`. A successful save that created a kit (no
  `kit_id` argument, and DA's structured `kit_id`, or for a workbook or deck a
  `# ✅ … Saved` answer naming exactly one kit id) writes
  `(account, kit id) -> (project, conversation)` into the project trust database
  (`report_kit_projects`, schema version 3, cascading with the project). The first link
  wins, a redefine never moves a report, and a regular chat carries no recorder. Deleting a
  report drops its link. A report saved before this, or on another device, reads as having
  no project.
- `report_open(reportId, page)` fetches the assembled, self-contained page from DA,
  registers it in the preview store, and returns a preview id, the page name, the
  page names, and when DA stored each dataset behind the page. A page over 2 MiB is
  refused as `too_large_to_open`. It is for dashboards: a workbook or deck has no page,
  and the viewer never opens one.
- **A saved report in the chat.** To show a saved report in a chat, the assistant calls
  the bundled server's `ida_view_dashboard` (`mcp_tenant_ida_view_dashboard`), which web
  renders through DA's MCP Apps viewer. Desktop hosts no MCP Apps view, so under that
  call's tool activity the transcript draws the report itself: the call's `kit_id` and
  `page` arguments go to `report_open`, and the page shows in the same sandboxed preview
  frame as a chat artifact, with Open in Reports. The id the page comes back under must
  match. Each call keeps the document it opened for the session (16 at most, cleared on
  sign-out), so a turn scrolled away and back does not open the report again. Another
  server's viewer is not drawn.
- `report_thumbnail(reportId)` reads the card image DA stored for a dashboard
  (`GET /thumbnail`, PNG or WebP, at most 200 KiB, signature checked) and returns it as
  base64. A card asks only when the list gave a `thumbnail_updated_at_ms`, keeps the image
  per version in memory (160 at most, cleared on sign-out), and shows it with `<img>`. A
  report DA holds no image for is drawn live from `report_open`, as before.
- **Making a thumbnail** (the owner's dashboards only). `report_thumbnail_capture_open`
  fetches the first page and puts DA's own capture pair first in its `<head>` (DA's pinned,
  patched `html-to-image` and `thumbnail-capture.js`, vendored verbatim under
  `src-tauri/src/reports/capture_assets` with a digest test) with a fresh nonce. The renderer
  loads it in a sandboxed 1024×640 frame that is on screen but transparent, posts the
  nonce and a `MessageChannel` port once it loads, and takes the one PNG the script sends
  back (20 s at most). `report_thumbnail_save` checks the image (standard base64, a PNG
  signature, 200 KiB at most) and `PUT`s it to `/thumbnail`, replacing any image; DA
  answers a reader who is not the owner with 404. Captures run one at a time. Three
  places start one: the viewer, once per session, after an owner's dashboard with no
  image has loaded; the card's Create/Update preview action; and Mine's Create missing
  previews, which works through the owner's dashboards with no image and can be stopped.
- `report_rename`, `report_set_pinned`, `report_set_hidden`, and `report_set_scope` patch
  DA. A rename always sends the description, so a cleared one is cleared; each of the
  others sends only its own field. A pin and a hide are per reader. Sharing is the
  owner's: `user` is private, `global` is the organization, and the surface asks before
  either change.
- `report_delete` deletes on DA. Rename and delete are owner-only; DA enforces it.
- `report_refresh` asks DA to re-run the kit's stored SQL and returns one outcome
  per dataset. It runs with a 180 s request timeout. The device sends no SQL and no
  rows. A report the assistant refreshed in chat shows its new data on the next open.
- `report_export(reportId, page)` fetches what it will write first, so a failure never
  opens a dialog for nothing. A dashboard's assembled page is written as one `.html` file.
  A workbook or deck is downloaded after the reader has chosen a place: DA answers
  `GET /file` with a redirect, and the backend follows that one hop to an `https` address
  without sending the bearer, streams the body into a temporary file beside the
  destination (at most 256 MiB), and replaces the destination only when every byte
  arrived. The account is checked again after each wait.
- `report_save_library_to_project` fetches the assembled page and writes it as
  `reports/<name>.html` through the same function the chat card's project save uses, so
  the name, the duplicate rule, the 64 MiB ceiling, and the approvals write are the same.
  The copy is a snapshot as of the last refresh and keeps no link to the report.
- The viewer's Schedule a refresh and Share via email are previews that call nothing:
  their dialogs say "Demo mode · No changes are applied". DA has no endpoint for either
  ([status.md](status.md)).

**Project reports.** `report_save_to_project` takes the same identity and no
project id: the project is the conversation's own binding. It re-reads the
document the same way and refuses a conversation with no project
(`not_in_project`) or a document over 64 MiB. It looks in `reports/` at the
project root for a file with the same bytes (`duplicate`, unless a copy was
asked for) and picks the first unused name `reports/<name>.html`, then
`<name> (2).html` and on. It writes one `Create` through
`ApprovalsService::save_for_account`, the path of § 6, so the write is planned,
journaled, audited, and containment-checked. `Create` never overwrites.

**Export.** `document_export_save(suggested_name, contents, format)` and
`document_export_pdf(suggested_name)`. **There is no path parameter, by design.**
The native save dialog opens in privileged Rust, so every destination originates
from the OS after a person named the file; repeated calls cannot accumulate into
a silent write. The suggested name is reduced to a plain base component, the
format's extension is re-asserted over whatever was typed, and the document size
is capped at 8 MiB. The file is written to a temp file beside the target and
renamed over it. The renderer holds no dialog permission:
`dialog:allow-open` was removed, so save, message and ask are not granted either. On Windows, PDF generation calls WebView2 `PrintToPdf`
directly rather than `window.print()`, which blocks the renderer on a preview UI
that can fail to appear. Other platforms answer `PRINT_VIA_DIALOG`, and the
renderer falls back to the native print panel.

---

## 9. Sign-out and local wipe

The two are different commands.

`auth_logout` (§ 2) ends the in-memory session and unbinds the account. It also
clears the in-memory upload queue (cancelling transfers), the staged datasets,
and the model catalogue snapshot. It deletes nothing on disk. In the renderer,
`setAuthSession` resets the chat, reports, datasets, and canvas stores whenever
the signed-in account changes. A run still executing is not cancelled by this command.

The account clear is not a separate command. It is the first step of Reset xFalcon
(below). `clear_account_state` in `src-tauri/src/app/commands.rs` clears upload and
staging state and path grants first. Then it drives
`security::account_cleanup::AccountCleanup`, an idempotent Rust-owned state
machine over a `FixedAccountResources` registry. It pins the current account, so
no other account can bind during the wipe, and runs one handler per resource in a
fixed order:

1. `ActiveWork` — the `active-work` directory;
2. `SessionVault` — the MCP credential directory first, then the in-memory vault,
   then the account's keys in both keyrings. Sealed rows go before the keys that
   open them;
3. `Journal` — the event journal;
4. `LocalDatabase` — `local.sqlite3` and its sidecars;
5. `LocalProjections` — the `local-projections` directory (the chat projection,
   and the reports projection that earlier builds wrote);
6. `ProjectData` — cancels project work, then removes `project-data/` (trust,
   index, revisions, plans);
7. `ArtifactCache` — the legacy `reports` directory that earlier builds wrote;
8. `Export` — the legacy `export` directory;
9. `DecryptedWorkingFiles` — the `decrypted-working-files` directory.

`ProjectData` and `DecryptedWorkingFiles` are registered only when project
cancellation is available.

`UploadTemporary` has a place in the order and no handler in the registry. After
every handler succeeded, the now-empty account directory is removed and the
account is unbound. Any failure is surfaced as a retryable partial result
instead of success.

Local wipe is local. Cloud deletion happens only through operations present in
the pinned DA contract, and the UI must not promise account-wide or backup
erasure it cannot perform.

### Reset xFalcon

Settings > Data & reset > **Reset xFalcon** runs `app_reset`, which
`src-tauri/src/app/commands.rs` defines. It is the only in-app erase command. The
person must type `RESET` in the dialog first. The webview check is a guard, not the
boundary. The boundary is the `allow-app-reset` capability. The steps run in this
order.

1. The command starts the reset helper with
   `device_purge::spawn_reset_helper`. The helper is a detached copy of the same
   executable, run with `--purge-local-data --after-parent-exit`, plus
   `--relaunch` in an installed build. Its stdin is a pipe that the app holds open
   for its whole life. If the spawn fails, the command records the audit event
   `data.erase` with outcome `failure` and detail `not_started`, and returns an
   error. Nothing is erased. A second call while one is under way returns "A reset is
   already under way. xFalcon will close and reopen on its own." and records no
   audit event.
2. The command records `data.erase` with resource type `local_data`, resource id
   `device` and outcome `allowed`.
3. It runs the account clear (`clear_account_state`): upload and staging state,
   path grants, and `AccountCleanup`, the handlers listed above. The result is
   ignored, because the purge supersedes it. The account clear writes no audit
   event. The clear is bounded at 10 seconds (`RESET_CLEAR_TIMEOUT`). On a timeout
   the app logs a warning and continues.
4. After 400 ms the app calls `exit(0)`. This happens even if the clear failed or
   timed out, because the exit lets the helper start its purge.
5. The helper waits for stdin to reach end of file, for up to 120 seconds. End of
   file comes when the app exits. On timeout it exits `3`, deletes nothing and does
   not relaunch.
6. The helper runs the full purge up to 20 times, 500 ms apart, until nothing
   remains and the credential store reports no error. That is at most about 10
   seconds of waiting. The purge deletes the credential store entries and every fixed path
   for the current OS, the same as `--purge-local-data` run by hand.
7. If `--relaunch` is set, the helper starts the app again, whatever the purge
   found. On macOS it uses `open -n <bundle>.app` when the executable is in
   `<bundle>.app/Contents/MacOS/`, and runs the executable directly otherwise.
   On Windows it starts a detached process.

The helper's stdout and stderr are discarded. The helper's exit codes are `0`
(clean), `1` (refused, nothing deleted), `2` (something remains) and `3` (the app
did not exit). The app never reads them. See
[data-lifecycle.md](data-lifecycle.md) for the full table.

**UNVERIFIED** on Windows and macOS: the hand-off from the exiting app to the
helper, the end-of-file trigger, the relaunch, and the real credential-store
deletion.

---

## 10. A tool call that waits for approval

Only a `Mutating` tool can wait, and by default nothing does (§ 3, standing
consent). With ask-every-time on (`XFALCON_REQUIRE_TOOL_APPROVALS=1`), the flow
is:

1. `ToolLoop::prepare` finds that policy returned `RequireApproval` and no
   standing grant covers the tool. It journals `approval_required` and keeps the
   invocation in `pending_approval`.
2. The round returns `paused`. The engine hands all other outcomes to the turn
   driver, moves the run to `AwaitingApproval`, and returns.
3. `spawn_local_run` sees `Ok(None)` and **parks** the whole `ToolRun` — run,
   journals, turn driver, tool loop — in `ParkedToolRuns`
   (`chat/tool_run.rs`). It stays registered in `run_cancels`, so it counts
   against the 8-run cap. Rebuilding any of those pieces on resume would be
   wrong: a new loop has no pending invocation, and a new driver has lost the
   model's context.
4. The renderer already received `approval_required` on
   `xfalcon://tool-invocation-event`. `ToolApprovalPrompt` shows the pending
   invocations of the active run and calls `chat_tool_approval_resolve(run_id,
invocation_id, approve)`.
5. `ChatService::resolve_tool_approval` checks that the run belongs to the
   current account and takes it out of the parked map. If a cancel arrived, it
   tears the run down instead. Otherwise `ToolRun::resolve_approval` either mints
   an `ApprovalToken` scoped to that exact invocation and tool and calls
   `ToolLoop::resume` (which authorizes again from scratch and then executes), or
   declines. A decline tells the model `denied` with `declined_by_user`, so it
   does not ask for the same call again.
6. While other invocations of the same round still wait, the run goes back into
   the map. Only when every one has a decision does a spawned task call
   `drive(resolved_outcomes)`. The engine moves the run from `AwaitingApproval`
   to `Running` and seeds the next turn with those outcomes. The run can pause
   again.
7. `chat_tool_approval_resolve` is not behind the work barrier.

If an MCP refusal or the run deadline arrives while a round has pending
invocations, the engine declines all of them before it fails the run.

---

## 11. Attachments and uploads

**Choosing a file.** `upload_choose` opens the native picker **in Rust** and
copies the chosen file into an app-owned snapshot under the app-local data
directory. The renderer never names a path. `upload_paste` takes bytes and a
file name from a renderer paste, or, with neither argument, reads native file
references from the clipboard. It needs the window focused. Both take an owned
`WorkBarrier` admission that lasts as long as the transfer.

With no conversation open, the picker or clipboard is read first. Only a chosen
file reserves a conversation to hold it (`conversation_reserve`). The renderer
keeps its id as `reservedConversationId`. Sending opens the reserved conversation
as the chat. Leaving the screen cancels its uploads and deletes it.

**Admission.** A file must be 10 MiB or less, must be a type in the attachment
table (`uploads/kind.rs`, classified by extension), and must match its extension
by content (`verify_chat_attachment_content`). At most 3 transfers are active at
once (`MAX_CONCURRENT_UPLOADS`).

**Transfer.** `UploadService::start` → `start_chat_attachment`. The queue state
moves `Selected` → `Preparing` (DA presigns) → `Uploading` (one PUT to object
storage, with a client that sends no `Authorization` header and follows no
redirect) → `Finalizing` (DA records it) → `Ready`. It can also end `Failed` or
`Cancelled`. The `Resigning` state belongs to the multipart path that datasets
use; a chat attachment does not enter it. Every change emits
`xfalcon://upload-queue`. A `Ready` upload carries the DA attachment
id as its `resource_id`, and the snapshot copy is deleted. Queue entries hold a
temporary-file lease while active, retryable, or otherwise capable of reading the
snapshot. Success, cancellation, queue removal, and account clear release the
appropriate leases; the active-operation lease prevents cleanup during a transfer.
Eager cleanup is best effort, while the delayed background cleaner handles only
recognized unleased staging directories.

**Send.** `chat_message_send` takes those ids. `message_send_inner` asks DA to
link them to the new user message and requires DA's `attachments_linked` count
to match.

**Into the model.** At run assembly, `attach_files` lists the conversation's
attachments from DA and puts each file back on the user turn it came with, newest
turn first:

- an image becomes the turn's image part;
- text, CSV, and TSV become text, and a workbook becomes one block per sheet,
  inside a delimited block that the preamble tells the model is data, not
  instructions;
- a file that cannot be read becomes a note saying so.

Text is bounded per file by the run's result budget and in total by twice that.
Images are bounded across the context, and older images are named rather than
re-sent. A file the allowance cannot hold is not even downloaded. If the message
being answered was sent with files and none come back, the run fails with "The
files attached to this message could not be loaded. Try again." A message sent
without files still runs when the list is unreadable.

**Through tool rounds.** After each tool round the driver reloads the context from
DA, and DA's rows hold text only. The driver keeps the attachments and the file
text it saw for each saved user message (`attachment_memory`,
`file_text_memory`), and the thinking blocks of each tool turn
(`reasoning_memory`). It puts them back after every reload, so the model keeps
seeing the file for the rest of the run.

**Datasets** share the transfer runtime and nothing else. `dataset_pick` (or a
drop on the window, `datasets::drop`) validates and **stages** a file in Rust and
returns a staged id; the webview never receives the path. A drop takes up to the
staging cap of files and reports one `xfalcon://dataset-dropped` outcome per
file. `dataset_upload_start` takes a work-barrier admission, optionally applies
the cleanup recipe to produce a CSV, and enqueues the file for the same
presign, multipart transfer, and finalize steps.

---

## 12. Events and the renderer store

Rust emits five chat-related channels (names and payloads are in
[ipc-reference.md](ipc-reference.md) § Events):

| Channel                           | Emitted when                                                                                                                                                |
| --------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `xfalcon://chat-projection-event` | A reconcile replaces a snapshot (`snapshot_replaced`); a conversation is removed; a run is announced for a conversation (`run_state_changed`).              |
| `xfalcon://chat-run-event`        | The run journal accepted a transition (`state_changed`).                                                                                                    |
| `xfalcon://chat-inference-event`  | The turn streams: `started`, coalesced `text_delta` and `reasoning_summary`, `context_compaction`, and the terminal `completed` or `failed`.                |
| `xfalcon://tool-invocation-event` | The invocation journal accepted a step (`requested`, `approval_required`, `approved`, `denied`, `started`, `progress`, `completed`, `failed`, `cancelled`). |
| `xfalcon://upload-queue`          | The upload queue changed (§ 11).                                                                                                                            |

The event bridge (`startChatEventBridge` in `src/features/chat/chatStore.ts`)
listens to the four chat channels. The store admits events by run, because every
channel is a broadcast:

1. **Binding.** The run for the conversation on screen is bound in two ways. The
   send result calls `adoptActiveRun` (state `queued`). The projection event
   `run_state_changed` for the open conversation does the same, and it arrives
   before the first delta.
2. **Admission.** Run and inference events for any other run are dropped.
   `requested` tool events are admitted by run id. Later events of an invocation
   are admitted by invocation id. Text and reasoning each keep their own sequence
   watermark, so a late or repeated delta is ignored.
3. **Live state.** Text accumulates in `streamingText`. The first `requested`
   event of a round moves the prose streamed so far into `roundProse`, so the
   round's narration sits among its tool rows. Reasoning is kept as segments
   placed among the tool rows and capped at 32 Ki characters, dropping the oldest
   text first. `context_compaction` sets `compactingRunId`.
4. **Commit cadence.** Text and reasoning updates commit state at once but notify
   React at most once per animation frame. Terminal events notify immediately.
5. **Hand-off.** On `completed`, the streamed text moves to `settlingText` so the
   answer does not blank while `finish_run` reconciles. On `failed`, partial text
   is dropped and the event's message becomes the visible error. The durable
   snapshot that follows retires `streamingText`, `settlingText`, `activity`, and
   `roundProse` together. Reasoning is kept, because the snapshot has none, until
   the reader leaves the conversation or a new run starts.
6. **Run state.** `state_changed` sets `runState` and the recovery notice for
   `interrupted` and `failed`. `completed` and `cancelled` clear the notice and
   the error. `isBoundRunPending` decides the composer's send or stop control.

`snapshot_replaced` for a conversation that is not on screen only updates the
sidebar title. A snapshot older than what the view holds is ignored.

---

## 13. Updates, the work barrier, and quitting

**Check.** `update_schedule::spawn` emits `xfalcon://update-check-requested`
every 4 hours. It sleeps first, so the first emission comes 4 hours after
launch. The renderer also checks once at startup (`startBackgroundUpdateChecks`
in `features/settings/updateStore.ts`). A check calls `update_check`
(`DesktopUpdateService::check`):

1. If an update was installed and the app is waiting to restart, it refuses.
2. It always asks the updater plugin (30 s limit), even when a package is
   already staged, so a release published after staging is never missed. It
   checks that the artifact URL is HTTPS on an allowed host with no credentials,
   prunes staged files that the manifest no longer names, and reports the
   available version, its release notes (cut at 32 Ki characters), and whether
   the package it names is already staged. A newer release has a different
   signature, so it is not staged and the older package is removed.

The renderer keeps checking while an update is staged. If the manifest names a
newer version, the renderer downloads it and replaces the staged one. Only a
restart error on screen pauses the background check; "Check again" resumes it.
Release notes are cut to 32 Ki characters before crossing IPC. While an update is
installing or ready to restart, Settings renders notes as passive Markdown using
the shared sanitized GFM/highlight pipeline. Source and copy actions retain the
original notes. HTML/SVG fences stay code; they do not register assistant preview
documents. Unsafe links are blocked by the shared link component, and remote
images are shown as omitted text.

**Download.** The renderer calls `update_install` when a version is available
and not staged. Despite its name, the command only **downloads and stages**:
`download_bounded` streams the package with a redirect policy that allows at most
5 hops and only to the allowed hosts, a 15 s connect limit, a 15 min total
limit, and a 512 MiB cap on bytes. It emits `xfalcon://update-download-progress`
about every 250 ms or at each percent. It then verifies the minisign signature
against the public key compiled into `desktop_update.rs` and saves the package
under the app cache directory `desktop-updates/` (written to a `.part` file, then
renamed). Ordinary work continues meanwhile.

**Restart.** `update_restart` is the step that installs. The renderer refuses it
while a run, a send, or a transfer is active (`updateWorkIsActive`). Before it
calls `update_restart` it runs `update_check` once more. If the manifest names a
version other than the staged one, the renderer downloads that version and does
not install, so one restart installs the latest release and never an older one
first. If the manifest cannot be reached, the verified staged package is applied.
The backend repeats the check:

1. Read the staged package and verify its signature again, before taking the
   barrier.
2. `guard_install` takes the barrier's write lock. That waits for every admission
   in progress, then sets the `blocked` flag. If `ChatService::has_active_work`
   (a send in flight or any registered run, parked ones included) or
   `UploadService::has_active_work` is true, it unblocks and returns "Finish the
   active run or transfer before restarting to update."
3. Otherwise it installs on a blocking task. Windows launches its installer and
   exits inside `install()`. On macOS, `preflight_macos_install` first requires
   a writable `.app` on the same volume as the temp directory, then the bundle is
   replaced and the app relaunches.
4. After a successful install the block stays on for the rest of the process.
   If the install fails, the block is lifted.

**The work barrier.** `WorkBarrier` is a read-write lock plus a `blocked` flag
(`work_barrier.rs`). New work takes a read admission. While the barrier is
blocked, admission fails with "An application update is being installed. Restart
xFalcon before starting new work." The commands that take an admission are
`chat_message_send`, `chat_message_edit`, `chat_message_regenerate`,
`chat_run_retry`, `upload_choose`, `upload_paste`, `upload_retry`, and
`dataset_upload_start`. An upload or dataset admission is held for the whole
transfer. A chat admission covers only the command call, because the run
continues in its own task. That is why the restart check also asks
`ChatService::has_active_work`. Cancel, approval resolution, and reads are not
behind the barrier.

**Quitting.** No code in `src-tauri/src` handles a window close or an exit
request. The only exits it causes are `exit(1)` on a failed startup (§ 1) and
`restart` after an update (`desktop_update.rs`). Anything held in memory is lost when the process ends:
the session including the refresh token, parked approvals, the preview store,
staged datasets, the upload queue, and MCP sessions. What stays on disk is the
encrypted projection, the run journals, the project stores, and the update cache.
A run that was streaming leaves an in-flight journal, and § 5 handles it at the
next launch.
