## Implementation and verification

- [x] `build.rs` compiles the SHA-256 of `resources/tenant-config.json` into the
      binary, and a bundled file with any other digest is refused. Evidence:
  - `auth::config::tests::an_edited_bundled_file_is_refused_even_though_it_parses`
  - `auth::config::tests::the_compiled_digest_is_the_digest_of_the_checked_in_file`
- [x] A release build has no environment fallback for the four tenant values.
      Evidence:
  - `auth::config::tests::a_release_build_without_a_bundled_config_is_not_configured`
- [x] Both hold on a real release binary. It was built with
      `cargo build --release --features custom-protocol` and a CI-shaped config,
      then launched three times on Windows. Its startup log showed:
  - intact config: the bundled tenant config resolved for `iseyon`;
  - edited issuer: the file was rejected as not the one the build was compiled
    with, and sign-in was unavailable;
  - file deleted, all four `XFALCON_*` set to another tenant: the file could not
    be read, and sign-in was unavailable.
- [x] A session with no authenticated state, for another account, mid-sign-in,
      or dead does not own the bound account. An expired session that holds a
      refresh token still does. Evidence: `auth::tests`, the nine
      `owns`/`outlives`/`dead`/`tolerance` cases.
- [x] Every project command is gated through `AppRuntimeState::projects()`.
      Evidence:
  - `security::tests::sign_in_gate::project_commands_are_refused_once_the_session_behind_the_account_ends`
- [x] The approvals commands (save, revisions, restore, patch decision) are
      gated, driven through the real command functions over a mock app. A
      signed-in save lands. After the session ends it is refused and the file
      is unchanged. Evidence:
  - `security::tests::sign_in_gate::a_project_save_is_refused_once_the_session_behind_the_account_ends`
- [x] Report list and open are gated in `ReportsService`, and the no-session
      fallback is removed. Evidence, over a cache that has synced before:
  - `reports::tests::cached_reports_are_refused_once_the_session_behind_the_account_ends`
  - `reports::tests::cached_reports_are_refused_to_a_dead_session`

  Against the previous service both tests fail. It served that cache as a stale
  offline copy with no session and with a dead one.

- [x] Conversation search and preview refuse a dead session with
      `SessionExpired`, and a signed-out device still gets `AuthRequired`.
      Evidence:
  - `chat::tests::service::conversation_search_refuses_a_dead_session`
  - `chat::tests::service::conversation_preview_refuses_a_dead_session`
  - `chat::tests::service::conversation_search_while_signed_out_still_asks_for_sign_in`
- [x] Strict Clippy passes in the debug and release profiles, rustfmt is clean,
      and the native suite passes.

Verification on 2026-09-23, on the Windows mirror:

- `cargo test --workspace --locked` passed: 1,750 native, 28 contract,
  9 release.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` was clean.
- `cargo clippy -p xfalcon-desktop --release --all-targets -- -D warnings` was
  clean.
- `cargo fmt --check` was clean.
- `prettier --check` passed on every changed Markdown file.

One earlier full run failed only the two documented parallel-suite flakes
(`mcp::tests::reauth::diagnostics_export_*`). Both passed alone and in the next
full run.

No frontend source changed, so the renderer suite was not re-run.
