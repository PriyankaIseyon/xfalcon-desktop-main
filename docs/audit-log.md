# Security audit log

Version 0.5.42 · reviewed 2026-10-15 · **Reference** · audience: security reviewers, auditors, support

The desktop app keeps a local security audit log on each device. This page
describes what the log records, where it lives, how its integrity check works,
how long it is kept, and how to read it. It also gives the steps for adding a
new recorded action. Scope is the desktop app only. The DA server, the MCP
server, Casdoor and the web app are separate systems and are not covered here.

> Status: this document describes the v0.5.42 source implementation and its open limits. It is not a certification, an attestation or an
> audit opinion. No independent review of the log has been recorded. Related
> control mapping and assurance status are in [soc2-control-mapping.md](soc2-control-mapping.md)
> and [assurance-plan.md](assurance-plan.md).

## What it is and why it exists

The audit log is a per-device record of security-relevant actions taken in the
desktop app. It answers questions such as: who signed in on this device, which
tool calls ran, which provider keys were changed, and when local data was
erased.

Its properties:

- **Local.** Events are written to a folder on the device. Nothing is sent off
  the device.
- **Tamper-evident.** Each record carries a keyed hash that links it to the
  record before it. Edits, reordering and mid-log removal are detected by
  `verify`, without DA and without the user's data.
- **Bounded.** Old segments are pruned by age and by total size.
- **Content-free.** Records hold identifiers, actions and outcomes. They never
  hold prompts, file contents, file names, emails, URLs or credentials.

It is a forensic record for one device. It is not a central SIEM feed.

**Not shipped centrally.** A transport to DA's audit endpoint exists in code
(`src-tauri/src/clients/da/audit.rs`, with `src-tauri/src/audit/da_sink.rs`
above it). The app opens `LocalAuditStore` under app-local `audit/` and does not wire
that transport into the local hub. Server-side retention and monitoring belong to
DA. The desktop app does not send events to DA, and no alerting exists on top of
the log. Local append or key-store failure does not block the user's action; a
failed append increments a dropped-event counter, and failure to open the store
disables auditing for the session. This is local fail-open behavior, not an
independently retained central audit record.

## Where it lives

The log is under the app's local data folder, in an `audit` subfolder. The
folder root is Tauri's `app_local_data_dir()`, as described in
[configuration.md](configuration.md).

| OS      | Audit folder                                              |
| ------- | --------------------------------------------------------- |
| Windows | `%LOCALAPPDATA%\ai.xfalcon.desktop\audit\`                |
| macOS   | `~/Library/Application Support/ai.xfalcon.desktop/audit/` |
| Linux   | `~/.local/share/ai.xfalcon.desktop/audit/`                |

Files in the `audit` folder:

- `audit-<first seq, 20 digits>.jsonl`: segment files. One record per line.
- `device-id`: a random UUID v4, created once per install. A quarantine copies it
  into the fresh log, so the id stays the same after a damaged log is set aside.

One more kind of item sits beside the `audit` folder, in the app-local data root:

- `audit.damaged-<unix seconds>/`: a damaged log that was set aside. At most
  three are kept.

The MAC key is not in the folder. It is in the OS credential store. See
[The MAC chain](#the-mac-chain).

## File and record format

Each line of a segment is one JSON object. The object has six top-level fields.
The `event` field holds the content-free event.

Worked example. The values are fake. The prev and mac values are SHA-256 digests
of short labels, not MACs from a real key. The line is one record, wrapped here
for reading only.

```text
{"v":1,"seq":41872,"ts_ms":1791891234567,"prev":"6e4f0f3b9c1d2a7e5b8c0d4f1a9e3b72c5d8f0a1b3e6c9d2f4a7b0e1c3d5f8a2","mac":"b2d7e41f08a6c39d5e1b7a0f2c4d86e9b13f5a7c0e2d4b6f8a1c3e5d7f9b0a42","event":{"event_id":"3f2b9c1e-7a4d-4e8b-9c2a-5d1f0e6b7a90","sequence":14,"actor_id":"user-example-01","tenant_id":"tenant-example","device_id":"8d1e4c2a-6b3f-4a9d-8e7c-2f5b1a0d9c3e","action":"tool.execute","resource_type":"tool_call","resource_id":"inv-example-0001","outcome":"success","policy_id":"tools.policy.v1","request_id":"run-example-0001","occurred_at_ms":1791891234567,"detail":"example.read_file"}}
```

| Field                  | Meaning                                                                                                                                                                                              |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `v`                    | Record format version. Currently `1`. `verify` rejects any other value.                                                                                                                              |
| `seq`                  | Position in the chain. Strictly +1 for each record. It continues across app restarts.                                                                                                                |
| `ts_ms`                | Time the record was written, in Unix milliseconds (UTC).                                                                                                                                             |
| `prev`                 | The MAC of the previous record, as 64 lower-case hex characters. The first record ever written has 64 zeros.                                                                                         |
| `mac`                  | The keyed hash of this record, as 64 lower-case hex characters. See [The MAC chain](#the-mac-chain).                                                                                                 |
| `event.event_id`       | A UUID v4 for the event.                                                                                                                                                                             |
| `event.sequence`       | A counter kept by the running app (see the note below).                                                                                                                                              |
| `event.actor_id`       | The signed-in user id. `unauthenticated` before sign-in. Never an email or a display name.                                                                                                           |
| `event.tenant_id`      | The tenant id. `unconfigured` when no tenant is known.                                                                                                                                               |
| `event.device_id`      | The contents of `audit/device-id`.                                                                                                                                                                   |
| `event.action`         | The action, as its wire string. The full list is in [Recorded actions](#recorded-actions).                                                                                                           |
| `event.resource_type`  | One of `account`, `credential`, `project`, `tool_call`, `mcp_server`, `entitlement`, `update`, `session`, `skill`, `export`, `local_data`, `diagnostics`.                                            |
| `event.resource_id`    | The item acted on, such as a row id, a plan id or the word `session`.                                                                                                                                |
| `event.outcome`        | One of `allowed`, `approval_required`, `approved`, `rejected`, `denied`, `updated`, `success`, `failure`.                                                                                            |
| `event.policy_id`      | The policy that decided. Examples: `tools.policy.v1` for tool events, `approvals.resolve.v1` for plan decisions, `projects.trust.v1` for trust changes, `desktop.app.v1` for other app-level events. |
| `event.request_id`     | The run id for chat tool events, `app` for most app-level events, and a fresh UUID for approval and trust events.                                                                                    |
| `event.occurred_at_ms` | The event time in Unix milliseconds. Same value as `ts_ms` for a record written by the app.                                                                                                          |
| `event.detail`         | Optional. One content-free identifier, such as a tool id or a provider name. Left out when there is none.                                                                                            |

Identifier rules. Every identifier field uses `[A-Za-z0-9-_.:]`, is 1 to 160
bytes long, and cannot be empty. Before punctuation normalization, the publisher
passes identifiers through the diagnostics sensitive-shape redactor, then replaces
other characters with `_`. Supported email, phone, JWT, URL-userinfo and
credential shapes therefore become redaction markers before normalization. Safe
UUIDs, provider/model IDs and ordinary opaque identifiers remain usable. An empty
value becomes `unrepresentable`; a long value is cut to 160 bytes. Pattern matching
does not guarantee that every personal or secret value is removed from a
user-supplied identifier.

**Note on `event.sequence`.** The running app sets this field from a counter that
starts at 1 on each launch. The record `seq` is the chain position and keeps
counting across launches. So the two values differ after the first launch. Use
`seq` to order records. Use `event.sequence` only within one launch.

## The MAC chain

Each record's `mac` is a keyed BLAKE3 hash. The input is these bytes, in order:

1. `v` as 4 bytes, little-endian.
2. `seq` as 8 bytes, little-endian.
3. `ts_ms` as 8 bytes, little-endian.
4. `prev` as 32 raw bytes.
5. The serialized `event` JSON.

The key is 32 random bytes from the OS credential store. The app creates it the
first time it opens the log. The key is stored under service `ai.xfalcon.desktop`
and account `device-audit-chain`. The entries are `device-audit-chain:key-v1`
and `device-audit-chain:active-version`.

**What the MAC covers.** Every field of the record, including `v`, `seq`,
`ts_ms`, the link to the previous record and the whole event.

**What the MAC does not cover.** The file names, the segment boundaries, the
`device-id` file, and the order of the segment files on disk. Those are checked
by sequence and chain rules, not by the MAC.

### What `verify` proves

Source: the module doc comment in `src-tauri/src/audit/local.rs`.

- Changing a record's content, version, sequence, timestamp or links is
  detected by anyone who does not hold the keychain key.
- Reordering records is detected by anyone who does not hold the keychain key.
- Removing a record from the middle of the retained log is detected by anyone
  who does not hold the keychain key.
- Removing records from the start of a segment is detected. Each segment's file
  name is the sequence of its first record. The first record of the oldest
  segment must also have a sequence that matches its file name, and an all-zero
  link only if it is record 1. So deleting head lines of the oldest segment makes
  `verify` fail.
- A record that fails to parse or to chain is reported with its sequence number
  and a reason. The report never includes event content.

### What `verify` does not prove

Source: the same module doc comment.

- Removing the newest records is not detected. The chain has no external anchor
  for its head.
- Someone who holds the keychain key can rebuild the whole log so that it
  verifies. The key is the trust root. This log does not defend against the
  account's own machine.
- Retention pruning removes the oldest segments. The first retained record then
  links to a deleted predecessor. That is reported as `anchored_after_prune`, not
  as tampering. The first record is accepted that way only when its sequence
  matches the segment's file name and its link is not all zeros. Deleting whole
  oldest segments therefore still verifies, and it is reported as pruning. A
  stripped oldest segment reads the same way, so it is not told apart from pruning.
- Opening parses only the newest segment, and open does not check MACs. `verify`
  checks every retained segment and its MACs. Retention uses the device clock.
- Torn-tail repair cuts only the bytes after the last newline, when the log is
  opened. The repair is reported as `torn_tail_repaired`. A cut of that kind
  cannot be told apart from a deliberate truncation of the last record.
- Structural damage sets the log aside, and a fresh chain starts. A MAC failure
  found by `verify` is reported and is not set aside. An I/O error never sets the
  log aside. See [Damaged logs and keychain failure](#damaged-logs-and-keychain-failure).

## Recorded actions

The table lists all 29 action values defined in `src-tauri/src/audit/event.rs`.
Six of them have no call site in the code, and one (`project.read`) has only a
wrapper that no production code calls. Those rows are marked **not recorded**.
Each "recorded by" entry names the function that makes the call. Those names
match the `emit::record`, `emit::record_with_detail`, `publish_tool_audit` and
`publish` calls in the source.

Shorthand used below:

- _hub call_ means `emit::record` or `emit::record_with_detail`, which write
  straight to the local log when it is open.
- _publisher call_ means `AuditPublisher::publish` or `publish_with_detail`.

| Action (wire string)   | When it is recorded                                                                                         | Resource type / resource id                                                                                         | Detail                                                                                                       | Outcomes                                         | Recorded by                                                                                                                                                                   |
| ---------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ | ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `auth.login`           | Sign-in completes and the account scope is bound.                                                           | `session` / `session`                                                                                               | none                                                                                                         | `success`                                        | `auth/commands.rs`, `auth_login`                                                                                                                                              |
| `auth.login_failed`    | `begin_login` returns an error, or the signed-in identity cannot be bound to local storage.                 | `session` / the sign-in error code in snake case, or `account_bind_failed`                                          | none                                                                                                         | `failure`                                        | `auth/commands.rs`, `auth_login`                                                                                                                                              |
| `auth.logout`          | The user signs out. Recorded before the identity is cleared.                                                | `session` / `session`                                                                                               | none                                                                                                         | `success`                                        | `auth/commands.rs`, `auth_logout`                                                                                                                                             |
| `credential.create`    | A provider key is created, or a custom model is created.                                                    | `credential` / the new row id, or `new` when the create failed. For a custom model, the model id or `custom_model`. | Provider name for a key, such as `openai`. The word `custom_model` for a custom model.                       | `success`, `failure`                             | `credentials/commands.rs`, `credential_create`; `chat/commands.rs`, `chat_custom_model_create`                                                                                |
| `credential.replace`   | A provider key secret is replaced, or a custom model is updated.                                            | `credential` / row id, or model id or `custom_model`                                                                | none for a key; `custom_model` for a model                                                                   | `success`, `failure`                             | `credentials/commands.rs`, `credential_replace`; `chat/commands.rs`, `chat_custom_model_update`                                                                               |
| `credential.delete`    | A provider key is deleted, or a custom model is deleted.                                                    | `credential` / row id, or model id or `custom_model`                                                                | none for a key; `custom_model` for a model                                                                   | `success`, `failure`                             | `credentials/commands.rs`, `credential_delete`; `chat/commands.rs`, `chat_custom_model_delete`                                                                                |
| `credential.use`       | Never. No call site.                                                                                        | none                                                                                                                | none                                                                                                         | none                                             | not recorded                                                                                                                                                                  |
| `project.trust`        | A project folder is trusted.                                                                                | `project` / project id                                                                                              | none                                                                                                         | `success`                                        | `projects/trust.rs`, `create` (publisher call, reached from `project_create` in `app/commands.rs`)                                                                            |
| `project.revoke`       | Trust for a project is revoked.                                                                             | `project` / project id                                                                                              | none                                                                                                         | `success`                                        | `projects/trust.rs`, `revoke_for_account` (publisher call; reached from `revoke`, which `project_revoke` in `app/commands.rs` calls, and from two paths in `chat/service.rs`) |
| `project.read`         | Defined. Only the sensitive-read decision wrapper records it. That wrapper has no production caller.        | `project` / the file's relative path                                                                                | none                                                                                                         | `allowed`, `approval_required`                   | `projects/read.rs`, `read_file_with_audit` (not called in production)                                                                                                         |
| `project.write`        | Never. No call site.                                                                                        | none                                                                                                                | none                                                                                                         | none                                             | not recorded                                                                                                                                                                  |
| `tool.approve`         | A tool call is approved, or a call is held for approval.                                                    | `tool_call` / invocation id                                                                                         | tool id, when the loop knows it                                                                              | `approved`, `approval_required`                  | `agent/tool_loop.rs`, `prepare` and `resume` (publisher call through `publish_tool_audit`)                                                                                    |
| `tool.deny`            | A tool call is rejected or denied in the chat loop. Also a pending plan is denied in the approvals service. | `tool_call` / invocation id (chat loop), or `project` / plan id (approvals service)                                 | tool id (chat loop only)                                                                                     | `rejected`, `denied`                             | `agent/tool_loop.rs`, `run_round`, `resume`, `prepare`, `decline_pending_approval`; `approvals/service.rs`, `resolve_local`                                                   |
| `tool.execute`         | A tool call finishes in the chat loop. Also an approved plan is applied in the approvals service.           | `tool_call` / invocation id (chat loop), or `project` / plan id (approvals service)                                 | tool id (chat loop only)                                                                                     | `success`, `failure`                             | `agent/tool_loop.rs`, `prepare`, `finalize`, `finalize_failure`; `approvals/service.rs`, `resolve_local`                                                                      |
| `mcp.connect`          | Never. No call site.                                                                                        | none                                                                                                                | none                                                                                                         | none                                             | not recorded                                                                                                                                                                  |
| `mcp.disconnect`       | Never. No call site.                                                                                        | none                                                                                                                | none                                                                                                         | none                                             | not recorded                                                                                                                                                                  |
| `mcp.invoke`           | Never. No call site.                                                                                        | none                                                                                                                | none                                                                                                         | none                                             | not recorded                                                                                                                                                                  |
| `entitlement.evaluate` | Never. No call site.                                                                                        | none                                                                                                                | none                                                                                                         | none                                             | not recorded                                                                                                                                                                  |
| `update.install`       | The user starts an update install.                                                                          | `update` / `desktop`                                                                                                | none                                                                                                         | `success`, `failure`                             | `desktop_update.rs`, `update_install` (hub call)                                                                                                                              |
| `mcp.server_create`    | An MCP server is added, or the bundled server is restored.                                                  | `mcp_server` / server id, `new` when the add failed, or `bundled` for a restore                                     | none                                                                                                         | `success`, `failure`                             | `mcp/commands.rs`, `mcp_server_create` and `mcp_server_restore_bundled`                                                                                                       |
| `mcp.server_update`    | An MCP server is changed.                                                                                   | `mcp_server` / server id                                                                                            | none                                                                                                         | `success`, `failure`                             | `mcp/commands.rs`, `mcp_server_update`                                                                                                                                        |
| `mcp.server_delete`    | An MCP server is deleted.                                                                                   | `mcp_server` / server id                                                                                            | none                                                                                                         | `success`, `failure`                             | `mcp/commands.rs`, `mcp_server_delete`                                                                                                                                        |
| `mcp.sign_in`          | The user signs in to an MCP server.                                                                         | `mcp_server` / server id                                                                                            | none                                                                                                         | `success`, `failure`                             | `mcp/commands.rs`, `mcp_server_sign_in`                                                                                                                                       |
| `mcp.sign_out`         | The user signs out of an MCP server.                                                                        | `mcp_server` / server id                                                                                            | none                                                                                                         | `success`, `failure`                             | `mcp/commands.rs`, `mcp_server_sign_out`                                                                                                                                      |
| `skill.import`         | A skill import is attempted.                                                                                | `skill` / `import`                                                                                                  | none                                                                                                         | `success`, `failure`                             | `skills/commands.rs`, `skill_import` (hub call)                                                                                                                               |
| `skill.delete`         | A skill is deleted.                                                                                         | `skill` / the first 12 hex characters of the BLAKE3 hash of the skill name. The name is not recorded.              | none                                                                                                         | `success`, `failure`                             | `skills/commands.rs`, `skill_delete` (hub call)                                                                                                                               |
| `data.export`          | A document, PDF, report or dataset is written to a file the user chose, or the write fails.                 | `export` / `document`, `pdf`, `report` or `dataset`                                                                 | the file extension, when known. Never a path or a file name. Reports pass no format, so they have no detail. | `success`, `failure`                             | `export/mod.rs`, `record_export` (hub call); `reports/commands.rs`; `datasets/commands.rs`                                                                                    |
| `data.erase`           | Reset xFalcon (`device`) starts. It writes the attempt before anything is removed, or a failure when the helper did not start. | `local_data` / `device` | none on the attempt. `not_started` on the failure. | `allowed` (attempt). `failure` only when the helper did not start. | `app/commands.rs`, `app_reset` (hub call; one event either way) |
| `diagnostics.export`   | A diagnostics report is generated. Recorded before the trail is read, so the export lists itself.           | `diagnostics` / `report`                                                                                            | none                                                                                                         | `success`                                        | `diagnostics/mod.rs`, `diagnostics_bundle_generate` (hub call)                                                                                                                |

Notes on the table:

- A cancelled save dialog records nothing for `data.export`. A PDF hand-off to
  the print dialog records nothing either. See the limits below.
- The approvals service and the chat loop both record `tool.execute` and
  `tool.deny`. They use different resource types and different identifiers, and
  only the chat loop adds the tool id as detail. A reader must not assume that
  one row equals one tool call.
- `data.erase` is recorded only by Reset xFalcon (`app_reset`). The account clear
  that Reset runs before the purge writes no event. The headless purge and the
  reset's helper delete the audit folder, so the record of the erase goes with it.
  A reset's own attempt event is erased with the rest of the log. See
  [data-lifecycle.md](data-lifecycle.md).

## What is not recorded

Known limits. Each one is a gap a reviewer should weigh.

- **No event for several defined actions.** `credential.use`, `project.write`,
  `mcp.connect`, `mcp.disconnect`, `mcp.invoke` and `entitlement.evaluate` are
  defined but never recorded. MCP connect and disconnect leave no trace here.
- **Project reads are not recorded.** The only project-read audit code has no
  production caller.
- **Project writes appear as `tool.execute`.** The approvals service records the
  plan id, not the tool id, so its rows carry no detail. The chat loop adds the
  tool id.
- **Sensitive-file refusals inside a tool are not separate events.** The tool's
  own outcome is what gets recorded.
- **macOS and Linux PDF export is not audited.** On those systems the PDF path
  hands off to the print dialog, and the dialog writes the file outside the app.
- **Report exports have no format detail.** They record `data.export` with resource id `report` and no detail.
- **Custom-model changes are recorded as credential events.** They carry the
  detail `custom_model`. They are not separate model events.
- **Nothing else is audited.** Chat deletion, settings changes, folder picks and
  most ordinary file reads leave no record.
- **Events stay on the device.** There is no central copy and no alerting.
- **Failed sign-in codes depend on the sign-in error.** Every error returned by
  `begin_login` is written as `auth.login_failed`. Which error codes a cancelled
  sign-in produces is not verified here.
- **The headless purge removes the log.** It deletes the whole app folder,
  including `audit/`, and it cannot record its own run.

## Identity binding

The actor and tenant come from the live session. They are not fixed at startup.

- Before sign-in, the actor is `unauthenticated` and the tenant is
  `unconfigured`.
- After `auth_login` succeeds and the account scope is bound, the hub takes the
  user id and the tenant id from the session (`bind_audit_identity` in
  `auth/commands.rs`).
- `auth_logout` records `auth.logout` first. Then the session ends.
- The identity is also cleared whenever a session ends or is replaced. The
  session store runs an end hook, set once in `lib.rs`
  (`AuthSessionStore::set_session_end_hook`), on `clear()` and on `set()` that
  replaces an authenticated session. A renewal of the same user does not run it.
  A token refresh that finishes after sign-out writes nothing. A refresh that
  returns a different user is refused and clears the session, so the hook runs.
  A sign-in cancelled after the loopback callback does not complete, so it binds
  no identity. An `invalid_grant` or identity-mismatch `clear()` can remove a newer
  session in a very narrow race. This is a known, accepted limit.
- The actor is always the user id. Email and display name are never written.

## Retention and size limits

Values are in `src-tauri/src/audit/local.rs`.

| Rule              | Value               | Behaviour                                                                                                                       |
| ----------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| Segment size      | 4 MiB               | A new segment starts when the current one is larger than 4 MiB before the next write. A segment can exceed 4 MiB by one record. |
| Month rollover    | UTC month           | A new segment starts when the month of the next event differs from the month of the current segment.                            |
| Age limit         | 400 days            | Segments whose newest event is older than 400 days are deleted, oldest first.                                                   |
| Total size        | 128 MiB             | While the total is above 128 MiB, the oldest segment is deleted.                                                                |
| Newest segment    | Never deleted       | Pruning always keeps at least one segment.                                                                                      |
| When pruning runs | At segment creation | Pruning runs only when a new segment is created, not on every write.                                                            |
| Damaged logs kept | 3                   | Older set-aside folders are deleted beyond the newest three.                                                                    |

The 400-day figure was chosen to cover a 12-month period with a margin. That is
a design choice. It is not a claim about any audit period.

## Crash safety and torn-tail repair

- Each append writes the line, flushes it and calls `sync_data`. All of this
  runs under one lock, so the chain stays in order.
- If the write fails, the file is cut back to its previous length. If the call
  created the file, the file is removed.
- On open, only the bytes after the last newline are cut off. That is the torn
  tail, and its repair sets the report flag "Torn tail repaired: yes".
- A complete line that does not parse is never cut off. It is damage. The log is
  set aside and a fresh chain starts (see the next section). A last line that
  ends in a newline and no longer parses is handled the same way, so an edit is
  never dropped silently.
- An edited last line that still parses fails `verify` on its MAC.
- An I/O error while opening, listing, reading, truncating or removing a segment
  never cuts or moves anything. The open fails and auditing is unavailable for
  the session. The files stay as they were, and the next launch retries.
- A failed append does not block the user action. The hub adds one to its
  dropped-event counter and logs only the error kind.

Tests that cover this: `a_truncated_last_line_is_repaired_on_reopen_and_the_chain_continues`,
`a_failed_append_is_counted_as_dropped_and_does_not_panic`,
`concurrent_appends_keep_the_chain_valid_and_gap_free`
(all in `src-tauri/src/audit/tests_local.rs`).

## Damaged logs and keychain failure

Three different failures are handled differently.

**Damaged log (structural).** Only structural damage moves the log. That is a
complete line that does not parse, or a broken chain in the newest segment. The
app renames the `audit` folder to `audit.damaged-<unix seconds>` beside it, keeps
the newest three of these, and starts a fresh chain. The `device-id` file is copied
into the fresh chain. The report shows "Previous log damaged and set aside: yes".

**I/O error.** An I/O error while the log is opened, listed, read, truncated or
removed never moves it. The open fails and auditing is unavailable for the
session. The files stay as they were, so the next launch retries. Do not delete
the `audit` folder to clear this. Fix the cause, such as the folder permissions
or a full disk, and restart.

**Device id.** A malformed or empty `audit/device-id` file is not damage. The app
regenerates it, writing a temporary file, syncing it and renaming it into place.
The log is kept.

**Keychain failure.** When the MAC key cannot be read or created, the log is
not set aside. Auditing is turned off for the session. The app logs an error,
and startup continues. The diagnostics report says the trail is not available.
The exact text is in the next section.

Tests that cover this: `a_damaged_log_is_set_aside_and_a_fresh_chain_starts`,
`at_most_three_damaged_logs_are_kept_after_four_recoveries`,
`a_keychain_failure_is_returned_and_never_sets_the_log_aside`
(in `src-tauri/src/audit/tests_local.rs`), and `an_unavailable_key_store_disables_auditing_without_failing`
(in `src-tauri/src/audit/tests_open_hub.rs`).

## Privacy

- Records hold identifiers, actions and outcomes only. The table above lists
  every field.
- The publisher redacts supported sensitive shapes before lossy ASCII
  normalization. The allowlist bounds the persisted alphabet, but normalization
  alone is not a privacy control and unsupported sensitive shapes may remain
  recognizable.
- The diagnostics section passes through the diagnostics redactor. It handles
  supported credential, bearer/JWT, email, phone, URL-userinfo and related
  patterns; it is not complete PII detection. See [Troubleshooting](troubleshooting.md)
  for current limits and safe handling of exported reports.
- Two identifiers are chosen by the user, and a reader should know that. A skill
  name is not recorded: `skill.delete` records a short hash of it. A relative file
  path would be recorded if the unwired project-read call were ever connected. That
  path is sanitized, not removed. Review it before wiring the project-read call.
- The diagnostics report and this log hold the user id. Recognized email-shaped
  identifiers are redacted before audit normalization, but this does not prove
  that every user id is non-personal or that unmatched identifiers are safe.

## How an auditor or support engineer reads it

Steps:

1. Open Settings, then Application, then Support, then choose **Export diagnostics report**. The
   save dialog writes a text file whose name starts with `xfalcon-diagnostics-`.
   The audit section is computed on a blocking thread, so the export does not
   hold up the app while the log is read.
2. Open the file and find the heading `## 4. Security Audit Trail`. That section
   is the audit trail. The sections before it are the rest of the report.
3. Read the first line under the heading. It is the integrity result. See the
   table below.
4. Read the counts and flags. They are listed in the next section.
5. Read the event lines. The newest 200 events are listed, oldest first.
6. Check the dropped-event count.
7. If the result is a failure, follow the failure steps below.

The section always has one of these headers:

| Line                                                                                                                                | Meaning                                                                                                                         |
| ----------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `Integrity: verified`                                                                                                               | Every record in the retained log passed the sequence, chain and MAC checks.                                                     |
| `Integrity: FAILED — <sentence>`                                                                                                    | `verify` stopped at the first problem. The sentence names it. See the list below.                                               |
| `The audit trail could not be read. Error kind: <code>`                                                                             | The log could not be read at all. The code is the internal error kind, such as `audit_storage_failed` or `persistence_invalid`. |
| `The audit trail is not available on this device: the OS key store or the log folder could not be opened. See the application log.` | Auditing is off for this session. No new events are written.                                                                    |

The possible `FAILED` sentences, from `verify` in `src-tauri/src/audit/local.rs`:

| Sentence                                                | What it means                                                                                          |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| `the first retained record is not a valid audit record` | The first line did not parse, or it failed the structural checks.                                      |
| `record N is not a valid audit record`                  | A later line did not parse, or it failed the structural checks.                                        |
| `record N fails its integrity check`                    | The MAC does not match, or `v` is not `1`. The record was changed, or it was written with another key. |
| `the first record has sequence 0`                       | The first retained record has sequence 0.                                                              |
| `record 1 does not chain from the start of the log`     | The first retained record is sequence 1, but its `prev` is not zeros.                                  |
| `sequence jumps from A to B`                            | Records are missing between A and B, or a sequence number was edited.                                  |
| `record B is out of order after record A`               | A record with a lower sequence number follows a higher one.                                            |
| `record N does not chain from record M`                 | The `prev` value of record N is not the MAC of record M.                                               |

When the first retained record links to a deleted predecessor, `verify` does not
fail. It shows `Older segments pruned: yes`.

The counts and flags in the section:

| Line                                 | Meaning                                                                                                                                       |
| ------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- |
| `Records`                            | Number of records that `verify` read in the retained log.                                                                                     |
| `Segments`                           | Number of segment files.                                                                                                                      |
| `Time range`                         | From the first to the last record time, in UTC. `none` when the log is empty.                                                                 |
| `Older segments pruned`              | `yes` when the oldest retained record links to deleted records. Not an error.                                                                 |
| `Torn tail repaired`                 | `yes` when the log was cut back on open. See the torn-tail section.                                                                           |
| `Previous log damaged and set aside` | `yes` when this log is a fresh chain after a damaged log was moved aside.                                                                     |
| `Dropped events this session`        | Events the app tried to write and could not. The count resets at each launch. A value above zero means some actions are missing from the log. |

Event lines look like this:

```text
2026-10-13T11:33:54.567Z seq=41872 tool.execute tool_call:inv-example-0001 outcome=success actor=user-example-01 detail=example.read_file
```

The fields are the time, the record `seq`, the action, the resource type and id,
the outcome, the actor, and the detail when there is one.

**What to do when the result is a failure.**

1. Do not edit, delete or "repair" the audit folder.
2. Close the app. Copy the whole `audit` folder and any `audit.damaged-*` folders
   somewhere safe. Keep the diagnostics report with them.
3. Record the sentence, the time of the report, the OS, the app version and the
   device id from `device-id`.
4. Tell the security owner. Compare the failure with the other evidence you have:
   the application log, the release history and what the user did.
5. Remember what the check cannot show. A changed last record, a deleted newest
   record, and an insider with the key are not detected. See the limits above.

## Limits of the guarantee

The limits are repeated from the module doc in `src-tauri/src/audit/local.rs`:

- The check proves that edits, reordering and mid-log removal are detected by
  anyone without the keychain key.
- The check does not prove that the newest records are present. The head of the
  chain has no external anchor.
- The check does not prove the log was not rebuilt by someone who holds the key.
  The key is the trust root.
- Pruning is not tampering. It is reported as `anchored_after_prune`.
- A torn tail is repaired, and the repair cannot be told apart from a deliberate
  truncation of the last record.
- A set-aside log starts a fresh chain.

Further limits. Each one is a known gap, not a defect that is hidden.

- Opening the log does not check MACs. Only `verify()`, at export, does.
- `verify()` takes the append lock only to snapshot the segment list and each
  segment's length. It reads and recomputes the MACs after releasing the lock, so
  an export does not stall writes. If retention removes a snapshotted segment
  during the check, that call fails once. The next call is correct.
- Retention is judged by the device clock. A large forward clock jump can age
  history out.
- A deleted oldest segment reads as normal pruning.
- The record carries no key id, so a reader cannot tell which key wrote it.
- `recent()` still holds the append lock.
- One approved file operation can produce two audit events, one from the approvals
  service and one from the tool loop.
- `mcp_server_update` and `mcp_server_delete` record the id as the webview sent it,
  before checking that it exists. `skill_delete` records the hash described above. The value is limited to
  ASCII `[A-Za-z0-9.-_:]`, at most 160 characters. So a label typed by a user can
  appear in the log.
- An MCP "restore bundled" is recorded as `mcp.server_create` with id `bundled`.

Two more points that follow from the code:

- The key is in the same OS credential store as the app's other secrets. Anyone
  who can read that store as the user can read the key.
- The chain is per device. It says nothing about what happened on another device.

## How to add an action

Follow these steps. Each one is needed.

1. In `src-tauri/src/audit/event.rs`, add a variant to `AuditAction`. Add it to
   `parse` and to `as_str`. Give it a `#[serde(rename = "...")]` attribute with
   the wire string. The wire string uses lower-case dotted form, such as
   `mcp.sign_in`.
2. If the action needs a new resource type or outcome, add it to the matching
   enum, its `as_str` and its serde rename. Check `audit_outcomes_match_the_unchanged_da_constraint`
   in `src-tauri/src/audit/tests.rs`. The DA transport takes these values as
   well.
3. Add the call. For a command handler, use `emit::record` or
   `emit::record_with_detail` from `src-tauri/src/audit/emit.rs`. For a service
   that holds a publisher, use `publish` or `publish_with_detail`. Pass only
   identifiers. Never pass names, paths, URLs or content.
4. Add a test. The existing one is `new_enum_strings_round_trip_through_parse_as_str_and_serde`
   in `src-tauri/src/audit/tests_local.rs`. Extend it to cover the new variant.
   Add a test at the call site that checks the recorded action, resource id and
   outcome, as the handlers in `credentials/commands.rs` and `skills/commands.rs`
   do.
5. Update the table in this document. Add a row. Name the function that records
   it. Confirm the call with `grep -rn "AuditAction::<Variant>" src-tauri/src`.
6. Run `cargo test` for the audit and command modules. Then run the full gate
   described in [contributing.md](contributing.md).

Source map for this document:

- Events and wire strings: `src-tauri/src/audit/event.rs`
- Chain, storage, retention and verify: `src-tauri/src/audit/local.rs`
- Identity and the in-memory hub: `src-tauri/src/audit/hub.rs`
- Publisher used by services: `src-tauri/src/audit/publisher.rs`
- Command helpers: `src-tauri/src/audit/emit.rs`
- Diagnostics section: `src-tauri/src/audit/diagnostics.rs`, and `audit_section`
  in `src-tauri/src/diagnostics/mod.rs`
- Opening the log at startup: `open_hub` in `src-tauri/src/audit/mod.rs`

For the wider design, see [security.md](security.md) and
[threat-model.md](threat-model.md) (TM-015). Erasure of the log is covered in
[data-lifecycle.md](data-lifecycle.md).
