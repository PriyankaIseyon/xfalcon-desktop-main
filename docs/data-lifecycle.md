# Data lifecycle

Version 0.5.42 · reviewed 2026-10-15 · **How-to + Reference** · audience: engineers, support, compliance owners

What the desktop app stores on a device, how long it keeps each item, and how
each kind of erasure works on Windows and macOS. It also gives the commands to
check for leftovers after an erase or uninstall. Scope is the desktop app only.
Server-side data held by DA is out of scope. So is data that a user saved
somewhere else.

> Status: this document describes the v0.5.42 source. Windows and Linux native tests pass; this is not real-system erasure evidence. See [assurance-plan.md](assurance-plan.md#staging-validation-evidence-2026-10-15). It is not a certification, an attestation or an audit
> opinion. The uninstall and erase paths on real Windows and macOS machines are
> not yet verified. Until a real-machine run is recorded, do not state that the
> app leaves no residue. See [Verification status](#verification-status).

## Data inventory

Paths are shown relative to the app's local data folder. The folder for each OS
is in [Roots per OS](#roots-per-os). The per-account folder is named
`accounts/acct-<sha256-hex>/`. The hex is the SHA-256 of the account id, which
is `<tenant_id>:<user_id>`. The name rule is in `account_directory_name` in
`src-tauri/src/security/account_cleanup.rs`. See also
[configuration.md](configuration.md).

Columns:

- **Account clear** means the first step of **Reset xFalcon** in
  **Settings > Data & reset**. It is not a separate action. It removes the items
  marked "Yes" in its column and keeps the items marked "No". The purge is the
  second step. It removes everything in the Purge column, so Reset leaves none of
  these items behind. See [Erase method 1](#erase-method-1-reset-xfalcon).
- **Purge** means `xfalcon-desktop --purge-local-data`. See
  [Erase method 2](#erase-method-2-the-headless-purge). Reset xFalcon runs the
  same full purge after the account clear, so the Purge column applies to Reset too.
- **Encrypted** is "Yes (AES-256-GCM)" for sealed SQLite stores. The sealing is
  in `src-tauri/src/persistence/crypto.rs`. "No" means the file is written as
  plain text or JSON.
- **Sensitivity** is this document's rating for a reviewer. It is not a
  classification from the data policy.

### Per-account data

These items sit under `accounts/acct-<sha256-hex>/`. Each account on the device
has its own folder.

| Item (under the account folder)                                | Contents                                                                                                                    | Sensitivity      | Encrypted  | Retention                                                                                                | Account clear | Purge |
| -------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | ---------------- | ---------- | -------------------------------------------------------------------------------------------------------- | ------------ | ----- |
| `local-projections/chat_projection.sqlite3` (+ `-wal`, `-shm`) | Conversations, messages, drafts and run records.                                                                            | High             | Yes        | Until the user deletes them, or an erase runs. No time-based expiry found.                               | Yes          | Yes   |
| `local-projections/run-journal/<run-id>.json`                  | Run state transitions: run id, sequence, from and to states. No message content.                                            | Low              | No         | Until erased. No pruning found.                                                                          | Yes          | Yes   |
| `local-projections/reports_projection.sqlite3`                 | Earlier builds only. Not read or written now.                                                                               | High             | Yes        | Until erased.                                                                                            | Yes          | Yes   |
| `project-data/projects.sqlite3`                                | Trusted project roots and conversation bindings.                                                                            | High             | Yes        | Until the trust is revoked or an erase runs.                                                             | Yes          | Yes   |
| `project-data/project-index.sqlite3`                           | File index for trusted projects.                                                                                            | High             | Yes        | Until erased.                                                                                            | Yes          | Yes   |
| `project-data/approvals.sqlite3`                               | Pending and past change plans.                                                                                              | High             | Yes        | Until erased.                                                                                            | Yes          | Yes   |
| `project-data/shadow_backups.sqlite3`                          | Earlier file contents kept before an overwrite. Up to 50 revisions per file and 512 MiB per account (`projects/backup.rs`). | High             | Yes        | Revision caps, then until erased.                                                                        | Yes          | Yes   |
| `mcp-data/server-credentials.sqlite3`                          | Sealed MCP server credentials, OAuth clients and grants.                                                                    | High             | Yes        | Until the server is deleted or an erase runs.                                                            | Yes          | Yes   |
| `chatgpt-plan/` (`meta.json` and `session-<digest>.bin`) | `meta.json`: host id, active client id, notice state, and per profile the subject and e-mail, no token. `session-<digest>.bin`: sealed ID, access and refresh tokens, subject, e-mail and scopes, one file per issued client id. | High | Session files only | Every session file is deleted at sign-out, and a session file is deleted on a terminal refresh error or a 401 refusal. `meta.json` stays until an erase runs. | Yes | Yes |
| `reports/reports.sqlite3`                                      | Saved reports from earlier builds. Nothing reads this folder now.                                                           | High             | Yes        | Until erased.                                                                                            | Yes          | Yes   |
| `export/`                                                      | Conversation exports from earlier builds. Nothing writes here now.                                                          | High             | Unverified | Until erased.                                                                                            | Yes          | Yes   |
| `active-work/`                                                 | No writer found in this review.                                                                                             | Unverified       | Unverified | Until erased.                                                                                            | Yes          | Yes   |
| `journal.sqlite3` (+ `-wal`, `-shm`)                           | No writer found in this review.                                                                                             | Unverified       | Unverified | Until erased.                                                                                            | Yes          | Yes   |
| `local.sqlite3` (+ `-wal`, `-shm`)                             | No writer found in this review.                                                                                             | Unverified       | Unverified | Until erased.                                                                                            | Yes          | Yes   |
| `decrypted-working-files/`                                     | Reserved for decrypted working copies of project files. No writer exists yet.                                               | High (once used) | No         | Until erased.                                                                                            | Yes          | Yes   |
| `chat-model.json`                                              | The selected model id.                                                                                                      | Low              | No         | Until changed or erased.                                                                                 | Yes          | Yes   |
| `custom-model-settings.json`                                   | Sampling values (for example temperature and top-p) per custom model id.                                                    | Low              | No         | Until changed or erased.                                                                                 | Yes          | Yes   |
| `claude-chat-effort.json`                                      | The reasoning level each Claude chat keeps (conversation id and level, at most 2,048 entries).                              | Low              | No         | Until the account is cleared or the device is reset.                                                     | Yes          | Yes   |
| `skills/` and `skills/state.json`                              | Imported skill library: `SKILL.md` and bundled files, and the list of skills the user turned off.                           | Medium           | No         | Until the skill is deleted or an erase runs.                                                             | Yes          | Yes   |
| `.skills-staging/`                                             | Skill folders being written during an import.                                                                               | Medium           | No         | On import, best-effort cleanup removes only recognized `import-` and `obsolete-` UUID folders older than 1 hour. `replaced-` recovery folders are preserved. | Yes          | Yes   |

Skill import cleanup is deliberately narrow. It runs when a per-account skill import begins,
removes only old `import-<32 lowercase hex>` and `obsolete-<32 lowercase hex>` directories,
and leaves `replaced-*` recovery copies untouched. Failed removal is best effort and is retried
on a later import. User-selected source files and folders are outside these roots and are
not removed by this cleanup.

When every handler succeeds, the account folder itself is removed, but only if
it is empty. An unknown file in it keeps it, and the erase reports `partial`
(or `failed` when nothing else was cleared).

### Device-level data

These items sit in the app's local data folder, outside any account folder.

| Item (app local data root)                                  | Contents                                                                                                    | Sensitivity               | Encrypted         | Retention                     | Account clear | Purge |
| ----------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- | ------------------------- | ----------------- | ----------------------------- | ------------------------------ | ----- |
| `logs/xfalcon-desktop.<YYYY-MM-DD>.log`                     | Application log. Redacted at write time: emails, JWT-shaped values, sensitive query parameters and headers. | Medium                    | No                | Daily rotation; 7 files kept. | No (kept)                      | Yes   |
| `EBWebView/`                                                | WebView2 profile for the app window: browser-side storage.                                                  | Medium                    | Unverified        | Until erased.                 | No (in use by the running app) | Yes   |
| `audit/`                                                    | Local security audit trail. See [audit-log.md](audit-log.md).                                               | Medium (identifiers only) | No; MAC-protected | 400 days; 128 MiB total.      | No (kept on purpose)           | Yes   |
| `audit.damaged-<unix seconds>/`                             | A damaged audit log set aside.                                                                              | Medium                    | No                | At most 3 kept.               | No                             | Yes   |
| `settings.json`                                             | Appearance preferences, and the reasoning and follow-up switches.                                                                                     | Low                       | No                | Until changed or erased.      | Yes                            | Yes   |
| `keepsakes.json`                                            | Greeting day and a sent-message count.                                                                      | Low                       | No                | Until erased.                 | Yes                            | Yes   |
| `standing-tool-approvals.json`                              | "Always allow" tool grants (`src-tauri/src/tools/standing.rs`).                                             | Medium                    | No                | Until changed or erased.      | Yes                            | Yes   |
| `plan-providers.json`                                       | Claude runtime switch and the reasoning level for new chats, device-wide (4 KiB limit).                     | Low                       | No                | Until changed or erased.      | Yes           | Yes   |
| `skills/` (legacy, device-wide)                             | Skill library from earlier builds.                                                                          | Medium                    | No                | Until erased.                 | Yes                            | Yes   |
| `.skills-staging/`                                          | Skill import staging.                                                                                       | Medium                    | No                | Until erased.                 | Yes                            | Yes   |
| `staging/`                                                  | Dataset files staged for upload in UUID child directories.                                                  | High                      | No                | No startup sweep; first background pass after 30 minutes, then every 30 minutes, best effort. Active/retryable work is leased. | Yes                            | Yes   |
| `claude-turns/` | One `turn-<uuid>/` per Claude turn, holding the system prompt and the MCP configuration with the turn's bearer token. | High | No | Removed when the turn ends. A directory left by a crash is removed at the next start-up. | Yes | Yes |
| `upload-snapshots/`                                         | Attachment snapshots in UUID child directories.                                                             | High                      | No                | No startup sweep; first background pass after 30 minutes, then every 30 minutes, best effort. Active/retryable work is leased. | Yes                            | Yes   |
| `migrations/webview-storage-v1-development` and `-packaged` | Marker files for the web-storage migration.                                                                 | Low                       | No                | Kept.                         | **No** (kept on purpose)       | Yes   |

Two more items are not in the app local data folder:

| Item                       | Location                                                                                                                                                                                 | Contents                                                                                                                                                            | Retention                                                 | Account clear | Purge               |
| -------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------- | ------------------- |
| `desktop-updates/`         | The app cache folder, `app_cache_dir()`. On Windows this is the same folder as the local data root (Tauri's path rules; not checked on a Windows machine). On macOS and Linux it is not. | Staged update packages named `<sha256-of-signature>.package`, and `.part` files while a download runs. Installs go through the updater, which checks the signature. | Only the current staged package is kept (`prune_staged`). | Yes on Windows. On macOS and Linux, the device sweep looks in the local data root and misses the cache folder. | Yes, both locations |
| Legacy shared-temp upload snapshots | `<system-temp>/xfalcon-upload-snapshots/` from earlier builds. | May contain snapshots created by older builds; current code does not own this shared path. | No automatic cleanup. If present, verify that a directory belongs to this app before any manual cleanup; never delete the shared path blindly. | Left untouched because ownership cannot be proved. | Left untouched because ownership cannot be proved. |

### Credential store entries

Secrets are kept in the OS credential store, not in files.

| Service                            | Entry name                                                          | Holds                                                                                                | Account clear | Purge |
| ---------------------------------- | ------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------- | ----- |
| `ai.xfalcon.desktop`               | `<account-id>:key-v<N>`                                             | A 32-byte key that seals the account's local database. `N` is the key version, up to 64 per account. | Yes. The account's own entries are deleted. | Yes   |
| `ai.xfalcon.desktop`               | `<account-id>:active-version`                                       | The active key version for the account.                                                              | Yes                                         | Yes   |
| `ai.xfalcon.desktop`               | `device-audit-chain:key-v1` and `device-audit-chain:active-version` | The MAC key for the audit trail. Not account-scoped.                                                 | **No.** The audit trail is kept on purpose. | Yes   |
| `ai.xfalcon.desktop.provider-keys` | `<account-id>:key-v<N>` and `<account-id>:active-version`           | A device key that seals MCP server credentials, OAuth clients and grants.                            | Yes                                         | Yes   |

`<account-id>` is `<tenant_id>:<user_id>`. Each service is one keychain service
name. The naming rules are in `src-tauri/src/persistence/secret_store.rs`.

Store representation per OS:

- **Windows.** Windows Credential Manager, as generic credentials. The target
  name has the form `keyring:<entry name>@<service>`. The purge matches
  `^keyring:.*@<service>$`.
- **macOS.** The login Keychain, as generic password items with the service
  and account attributes. Not verified on a Mac in this review.
- **Linux.** A Secret Service item, through the keyring crate. Not verified on a
  Linux desktop in this review.

The app also keeps the refresh token in memory only. It is never written to the
credential store or to disk.

### Roots per OS

| OS      | App local data root                                              | Cache root                                                  | Other folders the purge removes                                                                                                                                                                                                                                                       |
| ------- | ---------------------------------------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows | `%LOCALAPPDATA%\ai.xfalcon.desktop\`                             | Same folder                                                 | `%APPDATA%\ai.xfalcon.desktop\`                                                                                                                                                                                                                   |
| macOS   | `~/Library/Application Support/ai.xfalcon.desktop/`              | `~/Library/Caches/ai.xfalcon.desktop/`                      | `~/Library/WebKit/ai.xfalcon.desktop/`, `~/Library/HTTPStorages/ai.xfalcon.desktop/`, `~/Library/Logs/ai.xfalcon.desktop/`, `~/Library/Preferences/ai.xfalcon.desktop.plist`, `~/Library/Saved Application State/ai.xfalcon.desktop.savedState/` |
| Linux   | `$XDG_DATA_HOME/ai.xfalcon.desktop/` (default `~/.local/share/`) | `$XDG_CACHE_HOME/ai.xfalcon.desktop/` (default `~/.cache/`) | `$XDG_CONFIG_HOME/ai.xfalcon.desktop/` (default `~/.config/`)                                                                                                                                                                        |

The purge does not name a WebKitGTK profile folder for Linux. Where WebKitGTK
keeps storage on Linux is unverified in this review.

The purge reports required roots that it cannot resolve as incomplete; it does
not report a clean result when those roots are missing. It no longer targets
global temporary paths because the app cannot prove ownership there. See
[Erase method 2](#erase-method-2-the-headless-purge).

Windows `%APPDATA%\ai.xfalcon.desktop\` is a purge target only. No code in this
review writes to it. The purge removes it anyway.

## Retention rules

| Data                                                                     | Rule                                                                                               | Where it is set                                                   |
| ------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------- |
| Account data (conversations, projects, reports, skills, MCP credentials) | No time-based expiry found. Kept until the user deletes an item, an erase runs, or the purge runs. | Per-store code. Server-side copies are DA's responsibility.       |
| Application log                                                          | Daily files; 7 kept.                                                                               | `max_log_files(7)` in `src-tauri/src/lib.rs`.                     |
| Audit trail                                                              | 400 days; 128 MiB total; newest segment never deleted.                                             | `src-tauri/src/audit/local.rs`. See [audit-log.md](audit-log.md). |
| Damaged audit logs                                                       | Newest 3 kept.                                                                                     | `MAX_DAMAGED_SIBLINGS`.                                           |
| App-local staging and upload snapshots                                   | Delayed first sweep, then best-effort scan every 30 minutes; active and retryable work holds a lease. | `security/temp_cleanup.rs`; roots registered in `lib.rs`. No startup sweep. |
| Staged update packages                                                   | Only the current package is kept.                                                                  | `prune_staged` in `src-tauri/src/desktop_update.rs`.              |
| File revisions                                                           | 50 per file; 512 MiB per account.                                                                  | `projects/backup.rs`.                                             |
| Keepsakes, settings, chat model, custom model settings                   | Until changed or erased.                                                                           | Per-file code.                                                    |

## Erase method 1: Reset xFalcon

Reset xFalcon is in Settings > Data & reset.
This is the only in-app erase action. The block is the last one in the
Application section of Settings, under the heading "Data & reset"
(`src/features/settings/DataResetSection.tsx`). The Settings page has four
sections; see [architecture.md](architecture.md#settings) for the layout. The Account menu contains **Sign
out only**. Earlier builds also had a **Clear local data…** row, which was removed. Its
account clear is now the first step of Reset.

Reset removes everything the device purge removes, for every account on the
device. It then starts the app again, if the installed build relaunches it, with
the local data erased. The dialog describes this as a restart "as a fresh
installation". The app binaries are not removed. The steps below are
UNVERIFIED on a real Windows or macOS machine. See the status at the end of this
section.

**The dialog** is titled "Reset xFalcon?". It has two lists.

- **What will be erased:** sign-in on this device; cached conversations, trusted
  projects and the file history xFalcon keeps; saved API keys, provider
  credentials and MCP server connections; imported skills, settings and
  preferences; logs, the local security audit log, diagnostics and temporary
  upload files.
- **What stays:** the account and its data on the xFalcon server; files and
  folders on the computer, including folders opened as projects; documents and
  reports exported elsewhere.

The dialog says the app closes and reopens by itself. The confirm button stays
disabled until the person types `RESET`. The match is case-sensitive and the text
is not trimmed. Enter submits only when the text matches.

The typed word is a guard in the webview against a stray click. It is not the
security boundary. The boundary is the capability that allows the webview to call
`app_reset` (`allow-app-reset`).

While the reset runs, the dialog shows "Resetting xFalcon… The app will close and
reopen in a moment." It stays busy until the process exits. If the reset cannot
start, the dialog shows the backend message, or "xFalcon could not start the
reset. Nothing was removed." It keeps the typed text, so the person can retry.

**The command** is `app_reset` in `src-tauri/src/app/commands.rs`. It takes no
arguments and returns `Result<(), String>`. The webview sees no value on success,
or an error string on failure. Its wrapper is `appReset()` in `src/lib/native.ts`.
It is the only erase command.

**The flow.**

1. `device_purge::spawn_reset_helper(relaunch = !tauri::is_dev())` starts a
   detached copy of the same executable with `--purge-local-data
   --after-parent-exit`, plus `--relaunch` in an installed build. The helper's
   stdin is a pipe. The app keeps the write end open in a static for its whole
   life, so the helper's stdin reaches end of file only when the app exits. A
   spawn failure erases nothing and returns an error without any paths. A second
   call while one is under way is refused with the message "A reset is already
   under way. xFalcon will close and reopen on its own." It writes no audit event.
2. The command writes the audit event `data.erase`, with resource type
   `local_data`, resource id `device` and outcome `allowed`. If the helper did not
   start, it writes outcome `failure` with detail `not_started`, and nothing else
   runs.
3. The account clear runs next (`clear_account_state` in `app/commands.rs`):
   upload and staging state, path grants, and `AccountCleanup::clear()`. The
   items it removes and keeps are listed in
   [What the account clear removes and keeps](#what-the-account-clear-removes-and-keeps).
   Its result is ignored, because the purge that follows covers the same items.
   The clear is bounded at 10 seconds. If it times out, the app logs a warning and
   carries on, because the purge covers the rest.
4. After 400 ms the app calls `exit(0)`. This runs even when the clear failed or
   timed out.

The helper then does the following.

- It waits for stdin to reach end of file, for up to 120 seconds. If the app has
  not exited by then, it exits with code `3`. It deletes nothing and does not
  relaunch.
- It runs the full purge (method 2) up to 20 times, 500 ms apart, until nothing
  remains and the credential store reports no error. That is at most about 10
  seconds of waiting. Retries cover an anti-virus scan or a slow release of a file
  handle. On Windows they also cover WebView2 child processes.
- If `--relaunch` is set, it starts the app again whatever the purge found. On
  macOS it runs `open -n <bundle>.app` when the executable sits in
  `<bundle>.app/Contents/MacOS/`, and runs the executable directly otherwise. On
  Windows it starts a detached process.

**Helper exit codes.** The helper's stdout and stderr are discarded, and the app
does not read its exit code. Nothing in the app shows these codes.

| Code | Meaning                                                                                                                       |
| ---- | ----------------------------------------------------------------------------------------------------------------------------- |
| `0`  | Clean. Nothing remains after the retries.                                                                                     |
| `1`  | Refused. A target failed a safety check. Nothing was deleted.                                                                 |
| `2`  | Something remains after all retries, or the credential store could not be cleared.                                            |
| `3`  | The app did not exit within 120 seconds. Nothing was deleted, and the app was not relaunched.                                 |

The app is relaunched after codes `0`, `1` and `2` when `--relaunch` is set. It is
not relaunched after `3`.

**Audit.** The reset writes `data.erase` with outcome `allowed` before anything
is removed. The account clear in step 3 writes no event of its own. The reset
erases the local audit log with everything else, so the reset's own event is
erased too. A device-local trail cannot keep a record of its own erasure. This is
a known limit.

**Limits.**

- The reset does not reach server-side data in DA, or files outside the app's
  folders. See [What the app cannot remove](#what-the-app-cannot-remove).
- The reset cannot be undone. The local audit log is erased with the rest.
- The app does not show the helper's exit code. After a reset, check the folders
  and credentials by hand, as in [Verifying there is no residue](#verifying-there-is-no-residue).
  If something remains, run the purge with the app closed (method 2).
- **One user per device is an assumption.** Reset removes data for every account
  on the device, not only the signed-in account. On a shared OS user profile,
  Reset erases the other accounts' data too. It is not a per-account erase.

**Not verified on a real Windows or macOS machine:** **UNVERIFIED.** The hand-off
from the exiting app to the helper, the end-of-file trigger on the pipe, the
relaunch on each OS, and the real deletion of credential-store entries. The reset
steps in [Verifying there is no residue](#verifying-there-is-no-residue), including
the reset steps at its end, have not been run.

### What the account clear removes and keeps

The account clear is the first step of Reset. It is not a separate action, and
the app does not offer it on its own. The purge that follows removes everything
listed under "keeps" as well, so Reset leaves nothing behind. The account clear
returns `cleared`, `partial` or `failed` (`LocalDataClearStatus` in
`crates/contracts/src/app.rs`). Reset ignores that result, and the app does not
show it.

**What it removes.**

- The account's sealed stores, its MCP credentials, its ChatGPT plan folder (`chatgpt-plan/`), and the keys for both
  keychain services for this account.
- The account's files listed in the per-account table above with "Yes" in the
  account clear column, including `claude-chat-effort.json`.
- The device items the device sweep names: `keepsakes.json`, `settings.json`,
  `plan-providers.json`, `claude-turns/`, `standing-tool-approvals.json`, the legacy root `skills/` library,
  `desktop-updates/` under the local data root, `.skills-staging/`, `staging/`,
  and `upload-snapshots/`. It does not delete shared system-temp folders.
- The account folder, only if it is empty at the end.

**What it keeps, and why.** The purge removes all of these after the app exits.

- Other accounts on the device. Their folders and keys are not touched by the
  account clear.
- `audit/` and the audit MAC key. The account clear does not remove them.
- `logs/`. The log is in use by the running app.
- `EBWebView/`. The WebView profile is in use by the running window.
- `migrations/`. These markers are not removed by the account clear.

**Gaps in this step.**

- On macOS and Linux, the account clear's device sweep looks only in the local
  data root, so it misses the cache folder (`app_cache_dir()`:
  `~/Library/Caches/ai.xfalcon.desktop/` on macOS, `$XDG_CACHE_HOME` on Linux).
  The purge removes that folder, so Reset does too.
- Server-side data in DA is not removed. The app cannot remove it.
- **One user per device is an assumption.** The account clear also removes items
  that belong to the whole device, not to the account: `settings.json`,
  `keepsakes.json`, `plan-providers.json`, `claude-turns`, `standing-tool-approvals.json`, the legacy device skill
  library, `desktop-updates`, `.skills-staging`, `staging`, and
  `upload-snapshots`. If several accounts share one OS user profile, the device
  items are shared by all of them.

## Erase method 2: the headless purge

**What it is.** `xfalcon-desktop --purge-local-data`. The check runs at the very
start of `run()` in `src-tauri/src/lib.rs`, before any window, log file or
keychain setup. The code is in `src-tauri/src/security/device_purge.rs`.

**What it removes, in order.**

1. Every credential store entry whose service is `ai.xfalcon.desktop` or
   `ai.xfalcon.desktop.provider-keys`. The match is exact. On Windows the search
   uses the `keyring:...@<service>` pattern. Both services are searched even when
   the first search fails. The step is an error, and the purge exits `2`, when no
   credential store is available, when the store cannot search for entries, or
   when an entry's service or account cannot be read.
2. Every fixed path in the roots table for the current OS. This includes the
   whole app folder, WebView storage, logs, the audit trail, accounts, updates,
   and the temp upload snapshots.

**It does not stop at a failure.** Each step runs even when an earlier one
failed. Anything still present afterwards is listed as residue.

**Environment variables.** The fixed paths come from `HOME` (or `USERPROFILE`),
`LOCALAPPDATA`, `APPDATA`, `XDG_DATA_HOME`, `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`.
When the purge reads these, a value that is a relative path is treated as unset.
A missing required root is recorded in `unresolved_roots`, makes the result
incomplete, and maps to exit code 2. The purge still processes all resolved
targets. The printed summary does not name unresolved roots, so exit 2 with zero
remaining paths and a clear credential store can still mean a required root was
unresolved. Check the required OS variables and the fixed app locations by hand,
as in [Verifying there is no residue](#verifying-there-is-no-residue).

**Safety checks.** Every target must be an absolute path. It must not contain
`..`. Its last name must be one of `ai.xfalcon.desktop`,
`ai.xfalcon.desktop.plist` or `ai.xfalcon.desktop.savedState`. If any target
fails a check, nothing is deleted.
Symbolic links and Windows junctions are unlinked. They are never followed. A
missing path counts as success.

**Exit codes.**

| Code | Meaning                                                                                      |
| ---- | -------------------------------------------------------------------------------------------- |
| `0`  | Clean. Nothing remains, the credential store was cleared, and no required root was unresolved. |
| `2`  | Incomplete: a path remains, the credential store could not be cleared, or a required root is unresolved. |
| `1`  | Refused. A target failed a safety check. Nothing was deleted.                                |

The command prints counts to standard error. It prints the number of paths and
credentials removed, the number of paths that remain, and whether the
credential store was cleared. It does not print user data paths.

**Run it with the app closed.** A running app holds its files. On Windows, an
open WebView2 profile can block its own deletion. The purge then exits with `2`.
The NSIS uninstall hook also runs Tauri's `CheckIfAppIsRunning` check when it
purges (see method 3).

**Modes.** Run without other flags, the command is one purge with no waiting and
no retry. The flags below are internal. Do not run them by hand.

- `--retry-until-clean` repeats the purge, up to 20 runs 500 ms apart, while
  something remains or the credential store reports an error. It does not wait
  and does not relaunch. The Windows NSIS uninstall hook
  (`src-tauri/windows/hooks.nsh`) and `scripts/uninstall-macos.sh` pass it, so a
  WebView2 process that exits a moment after the app does not leave residue. It
  is honored only together with `--purge-local-data`.
- `--after-parent-exit` and `--relaunch` are used only by the reset helper
  (method 1).

**Warning for developers.** A development build of this repository uses the same
app identifier and the same folders. Running its purge erases the developer's
real local data and keychain entries.

**How to run it.**

- Windows (PowerShell). The installed program is usually under
  `%LOCALAPPDATA%\xFalcon\`. The folder is unverified on the target machine.
  Check it first.

  ```powershell
  & "$env:LOCALAPPDATA\xFalcon\xfalcon-desktop.exe" --purge-local-data
  $LASTEXITCODE
  ```

- macOS (Terminal):

  ```bash
  "/Applications/xFalcon.app/Contents/MacOS/xfalcon-desktop" --purge-local-data
  echo $?
  ```

- Linux: run the installed `xfalcon-desktop` binary with `--purge-local-data`.
  Its location depends on how the app was installed.

The purge removes data for every account on the device. It is not limited to
one account.

## Erase method 3: uninstall

### Windows NSIS installer

A real uninstall runs the same purge that Reset
xFalcon runs (method 2). The person does not need to tick a box. The hook is
`src-tauri/windows/hooks.nsh` (macro `NSIS_HOOK_PREUNINSTALL`). It is set in
`bundle.windows.nsis.installerHooks` in `src-tauri/tauri.conf.json`. This replaces
the earlier rule that the purge ran only when the uninstall checkbox was ticked.

The purge runs when both of these are true.

1. It is not an update. `$UpdateMode` is not `1`. The silent in-app updater
   passes `/UPDATE`.
2. Either the uninstaller is not running in place, or the person ticked **Delete
   the application data** on the uninstall page (`$DeleteAppDataCheckboxState` is
   `1`). "Running in place" means `$EXEDIR` equals `$INSTDIR`. That is the case
   when the installer starts the old uninstaller as a step of "Uninstall before
   installing" (see below).

What the hook does.

1. It runs in the uninstall section, before the program files are removed.
2. When the purge is due, it checks that the app is not running (Tauri's
   `CheckIfAppIsRunning` macro). The exact prompt that macro shows is UNVERIFIED.
3. It runs `"$INSTDIR\xfalcon-desktop.exe" --purge-local-data --retry-until-clean`.
4. A non-zero exit writes a line to the uninstall log. The line reads
   `xFalcon: some local data could not be removed (exit code N)`. The uninstall
   continues either way.

**Why the run location is the signal.** The silent in-app updater always passes
`/UPDATE`, so an update never purges. A newer setup run over an older install
("Uninstall before installing", the default) starts the old uninstaller without
`/UPDATE`. The installer starts it in place with `_?=<install dir>`, so its
`$EXEDIR` equals `$INSTDIR`. A real uninstall (Apps & features, or running
`uninstall.exe`) first copies the uninstaller to a temp directory and runs that
copy, so `$EXEDIR` is the temp directory and differs from `$INSTDIR`. NSIS removes
`_?=` from `$CMDLINE`, so the command line cannot tell the two cases apart, but
`$EXEDIR` can. The check on a throwaway NSIS installer on a Windows machine was:
a normal `uninstall.exe /S` logged `$EXEDIR` as `%TEMP%\~nsu*.tmp` and `$INSTDIR`
as the install directory; `uninstall.exe /S _?=<dir>` logged `$EXEDIR` equal to
`$INSTDIR`.

**What this means for the person.**

- Uninstall from Apps & features, run `uninstall.exe`, or run `uninstall.exe /S`
  directly. The purge runs. Sign-in, credentials, local history, logs, and the
  app's folders and Credential Manager entries are removed.
- Silent update (`/UPDATE`). No purge.
- A newer setup run over an older install, with "Uninstall before installing".
  No purge, unless the person ticks **Delete the application data** on that run's
  uninstall page. Whether that page is shown in that run is **UNVERIFIED**.
- Ticking **Delete the application data** purges in every case except an update.
- MSI installs do not purge. Run Reset xFalcon before uninstalling. See the MSI
  section.

Tauri's own template also removes `%APPDATA%\ai.xfalcon.desktop` and
`%LOCALAPPDATA%\ai.xfalcon.desktop` when that box is ticked. It never touches
Credential Manager. This review read a copy of the Tauri NSIS template text. That
copy is not in the repository, and this review did not confirm that it matches
the Tauri version the build uses. Confirm the behaviour on a Windows build.
**UNVERIFIED** until then.

The guards are pinned by the test
`security::tests::uninstall_hooks::nsis_uninstall_hook_runs_purge_and_skips_updates`.
It checks the hook's source text: the guard lines `${If} $UpdateMode <> 1`,
`${If} $EXEDIR != $INSTDIR` and `${OrIf} $DeleteAppDataCheckboxState = 1`, in that
order, with the purge inside them. It does not run the installer.

**Not verified.** The installer, the uninstall and the upgrade of xFalcon have not
been run on a machine. A real run deletes that machine's data and credentials. The
checks so far are the `$EXEDIR` check on a throwaway NSIS installer (not xFalcon)
and a `makensis` compile of the real installer script with the hook, with no
errors. **UNVERIFIED** until an actual install, uninstall and upgrade are recorded.

### Windows MSI installer

**There is no purge on MSI uninstall.** The app ships no WiX custom action. A
bad fragment could break the release build, and the MSI path could not be
verified. The NSIS installer is the supported way to remove data.

With the current build, an MSI uninstall does not remove the app folder, WebView
storage, logs, the audit trail, or the Credential Manager entries. Nothing in the
repository deletes them on that path.

Manual steps. Run them before uninstalling, while the program is still installed:

```powershell
& "<install folder>\xfalcon-desktop.exe" --purge-local-data
```

If the program is already removed, delete the folders and credentials by hand.
Check each folder and credential with the commands in
[Verifying there is no residue](#verifying-there-is-no-residue). Then delete
each listed credential target with `cmdkey /delete:<target>`. This manual path
is unverified.

### macOS

A drag-installed app has no uninstall hook. Moving the app to the Trash leaves
its data behind.

The repository provides `scripts/uninstall-macos.sh`. Usage:

```bash
scripts/uninstall-macos.sh                       # /Applications/xFalcon.app
scripts/uninstall-macos.sh "/path/to/xFalcon.app"
```

The script does the following, in order.

1. It refuses anything that is not a `.app` with `Contents/MacOS/xfalcon-desktop`
   and the bundle identifier `ai.xfalcon.desktop`.
2. It quits the app. If the app does not quit within about 10 seconds, it stops
   the process. After a forced stop it waits up to 5 more seconds. If the app is
   still running, it warns.
3. It runs `--purge-local-data --retry-until-clean` from inside the app bundle.
4. It removes the app bundle, whatever the purge result.
5. It exits with the purge's exit code. `0` is clean, `2` means something remains,
   and any other code prints "local data purge failed". A non-zero code is a
   warning.

The script is not bundled into the `.app` yet. The `bundle.macOS.files` option is
not verified against a mac build. **The script has not been run on a Mac.** Its
behaviour is unverified until it runs on a real machine.

The manual equivalent is:

```bash
"/Applications/xFalcon.app/Contents/MacOS/xfalcon-desktop" --purge-local-data && rm -rf /Applications/xFalcon.app
```

Run the purge first. Once the app bundle is deleted, the purge is no longer
available from inside it.

## Verifying there is no residue

Run these checks after an erase or uninstall. A `False` from `Test-Path`, or a
"No such file or directory" message, means the path is gone. Anything else is
residue. Record it. Do not assume that a clean result on one machine applies to
another.

### Windows (PowerShell)

```powershell
Test-Path "$env:LOCALAPPDATA\ai.xfalcon.desktop"
Test-Path "$env:APPDATA\ai.xfalcon.desktop"
cmdkey /list | Select-String "ai.xfalcon.desktop"
Test-Path "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\xFalcon"
```

Expected after a clean erase or uninstall: `False`, `False`, no output from
`cmdkey`, and `False`.

Notes on each line:

- The first line covers the app folder, including `EBWebView`, logs, the audit
  trail, accounts and updates.
- `cmdkey` lists the user's Credential Manager entries. Any line that mentions
  `ai.xfalcon.desktop` is residue. The provider-keys service matches the same
  filter, because its target name ends in `@ai.xfalcon.desktop.provider-keys`.
- The registry key name is unverified. It follows the Tauri NSIS template, where
  the uninstall key is named after the product name (`xFalcon`). That assumes a
  per-user install. To check the key name on the machine, run:

  ```powershell
  Get-ChildItem "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall" |
    Where-Object { $_.GetValue("DisplayName") -like "xFalcon*" }
  ```

- The install folder check, `Test-Path "$env:LOCALAPPDATA\xFalcon"`, is
  unverified. It depends on the install mode.

### macOS (Terminal)

```bash
ls ~/Library/Application\ Support/ai.xfalcon.desktop ~/Library/Caches/ai.xfalcon.desktop ~/Library/WebKit/ai.xfalcon.desktop ~/Library/HTTPStorages/ai.xfalcon.desktop ~/Library/Logs/ai.xfalcon.desktop 2>&1
ls ~/Library/Preferences/ai.xfalcon.desktop.plist 2>&1
ls -d ~/Library/Saved\ Application\ State/ai.xfalcon.desktop.savedState 2>&1
security find-generic-password -s ai.xfalcon.desktop 2>&1
security find-generic-password -s ai.xfalcon.desktop.provider-keys 2>&1
```

Expected after a clean erase: "No such file or directory" for each path, and
"could not be found" for each `security` command. That message comes with exit
status 44 for "item not found". Confirm the exit status on the machine.

Notes:

- `find-generic-password` may ask for keychain access. Answering it does not
  remove anything.
- `find-generic-password` prints the first matching item only. A match on any
  line means residue.

### Linux (Terminal)

```bash
ls -d ~/.local/share/ai.xfalcon.desktop ~/.config/ai.xfalcon.desktop ~/.cache/ai.xfalcon.desktop 2>&1
secret-tool search service ai.xfalcon.desktop
```

The `secret-tool` syntax is unverified in this repository. Check it on the
machine first. Expected after a clean erase: no paths, and no secret items.

### Manual steps for Reset xFalcon and the uninstall

**UNVERIFIED.** None of these steps has been run on a real machine. Run them on a
test machine with a test account, not on a machine that holds real data. Record
each result with the template at the end of this page.

1. Before starting, record what exists. Run the Windows or macOS checks above
   and note which items are present.
2. Open Settings > Data & reset > **Reset xFalcon**. Type `RESET` and confirm.
3. Wait for the app to close and reopen. Expect a fresh first-run state: no
   sign-in, no cached conversations, no saved keys.
4. Run the residue checks above. Expected: the same results as a clean erase.
   If anything remains, run `--purge-local-data` with the app closed and check
   again.
5. Expect no `audit` folder and no `audit.damaged-*` folder under the app's local
   data folder. The reset erases the audit log too, so its own record is gone as
   well. See method 1.
6. For the uninstall: install a test build, sign in, and uninstall from Apps &
   features. Expect the data to be removed. Run the residue checks. Then install
   an older build, sign in, and run the newer setup over it, choosing
   "Uninstall before installing". Expect the data to stay, because that in-place
   run does not purge. Whether the uninstall page is shown in that run is
   **UNVERIFIED**.


## What the app cannot remove

These items stay on the device, or outside it, after any erase. The app has no
way to reach them.

- OS crash dumps and diagnostic reports written by the operating system.
- Windows Prefetch and Recent files, which record that the app ran.
- Spotlight and QuickLook caches on macOS, and Time Machine or other backups.
- Files the user exported to a location of their choice. The audit trail records
  the export, but not the file's location.
- Data inside the user's own project folders. The app does not own those files.
- Server-side data held by DA: conversations, projects, keys and audit records
  that DA stores. Deleting it is a DA action. The local erase does not reach it.
- Copies of the diagnostics report that the user saved.

## Consistency with other documents

This page describes Reset xFalcon as the only in-app erase, and the NSIS purge
that runs on a real uninstall. [operations.md](operations.md) describes the same
erase paths and must match this page. If one of them changes, change this page
in the same change.


## Verification status

What each path has been checked against. "Unit-tested" means a test in this
repository runs the code with temporary folders or a mock keychain. It does not
mean a real machine was used.

| Path or behaviour                                                     | Evidence                                                                                                                                                                | Status                                                 |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ |
| Purge target lists per OS                                             | `security::device_purge` tests `windows_targets_are_the_fixed_set_in_order`, `macos_targets_are_the_fixed_set_in_order`, `linux_targets_use_xdg_defaults_and_overrides` | Unit-tested                                            |
| Purge target safety and unresolved required roots                      | `a_path_not_ending_in_an_app_name_or_containing_parent_segments_is_refused`; source reports missing required roots as incomplete | Windows/Linux native tests pass; real OS behavior remains unverified              |
| Purge exit codes                                                      | `exit_codes_follow_the_documented_mapping`                                                                                                                              | Unit-tested                                            |
| Purge removes fixed paths and leaves neighbours                       | `run_purge_deletes_the_fixed_targets_and_leaves_neighbours_alone`                                                                                                       | Unit-tested, temporary folders                         |
| Purge unlinks symbolic links and does not follow them                 | `a_linked_target_is_unlinked_and_its_destination_survives`                                                                                                              | Unit-tested, temporary folders                         |
| Keychain entries deleted by service                                   | `keychain_purge_deletes_only_the_two_services_entries`, `keychain_is_purged_before_paths_and_its_error_does_not_stop_the_paths`                                         | Unit-tested, mock keychain                             |
| Windows Credential Manager target pattern                             | `windows_pattern_anchors_the_exact_service`                                                                                                                             | Pattern test only. Not run against Credential Manager. |
| Account clear handlers and ordering | `security::tests::account_cleanup` module | Unit-tested, temporary folders and mock keychain |
| Device sweep leaves `logs/`, `EBWebView/`, `migrations/` and `audit/` | `kept_dirs` check in `security::tests::account_cleanup`                                                                                                                 | Unit-tested                                            |
| Audit trail erase record (wire strings) | `audit/tests_local.rs` checks the `data.erase` and `local_data` wire strings. No test covers the `app_reset` emit calls. | Unit-tested for wire strings only |
| Headless purge on Linux                                               | No run recorded in the repository. This review did not run it.                                                                                                          | **Not verified**                                       |
| Windows Credential Manager enumeration and deletion                   | None on Windows                                                                                                                                                         | **UNVERIFIED**                                         |
| macOS Keychain enumeration and deletion                               | None on macOS                                                                                                                                                           | **UNVERIFIED**                                         |
| Linux Secret Service enumeration and deletion                         | None on Linux                                                                                                                                                           | **UNVERIFIED**                                         |
| NSIS install and uninstall with the hook                              | None                                                                                                                                                                    | **UNVERIFIED**                                         |
| NSIS real uninstall, in-place run, and the checkbox | None | **UNVERIFIED** |
| MSI uninstall leaves data behind (as documented)                      | None                                                                                                                                                                    | **UNVERIFIED**                                         |
| macOS uninstall script (`scripts/uninstall-macos.sh`)                 | None. The script has not been run on a Mac.                                                                                                                             | **UNVERIFIED**                                         |
| Reset xFalcon on real Windows and macOS machines | None | **UNVERIFIED** |
| NSIS hook guards (`$UpdateMode`, `$EXEDIR`, `$DeleteAppDataCheckboxState`) | Source-text pin: `security::tests::uninstall_hooks::nsis_uninstall_hook_runs_purge_and_skips_updates`. `$EXEDIR` checked on a throwaway NSIS installer. Hook compiled with `makensis`. | Text pin and compile check only. Not run by an xFalcon install. |
| Reset helper argument parsing and the relaunch command | `parse_flags`, `relaunch_command` and `helper_args_wait_for_the_parent_and_relaunch_only_when_asked` in `security::device_purge` tests | Unit-tested |
| Reset hand-off: app exit, pipe end-of-file, helper run, relaunch | None on Windows, macOS or Linux | **UNVERIFIED** |
| Reset erases the credential store and every folder on a real machine | None | **UNVERIFIED** |
| The residue checklists above                                          | None. They have not been run on a clean machine.                                                                                                                        | **UNVERIFIED**                                         |

**Do not claim "no residue" as a fact.** A claim of that kind needs a recorded
real-machine run that covers the affected OS. The run must use the checklist
above, and it must record the result. Until one exists, state the test status
from the table.

### Recording a real-machine run

Keep one record per run. Store it outside the repository, or in an approved
place. Do not put user content or secrets in it.

```text
Run ID:
Date and time (UTC):
Person who ran it, and role:
OS name and version:
Install method (NSIS, MSI, DMG or other):
App version (from the About screen or the installer name):
Starting state (which items existed before the erase):
Erase method (Reset xFalcon, headless purge, uninstall script, uninstall hook):
Uninstall run type, if the hook ran (Apps & features, uninstall.exe, /S, /UPDATE, or in-place run):
Checkbox state, if the page was shown (ticked or not):
Command or steps used, exactly as run:
Exit code or status shown:
Checklist output (paste the results, with secrets removed):
Keychain state before and after:
Residue found, and its explanation:
Log or report saved for evidence (location only):
Reviewer, and date reviewed:
```

## Related

- [audit-log.md](audit-log.md): the audit trail. Reset erases it with the rest of
  the local data.
- [configuration.md](configuration.md): the on-disk layout and the keychain
  naming.
- [security.md](security.md): the security model and its known gaps.
- [operations.md](operations.md): release, signing and support procedures.
- [threat-model.md](threat-model.md): TM-013 (incomplete wipe) and TM-015 (audit
  loss).
- [compliance.md](compliance.md): control items for deletion and monitoring.
- [soc2-control-mapping.md](soc2-control-mapping.md) and
  [assurance-plan.md](assurance-plan.md): control mapping and assurance status.
