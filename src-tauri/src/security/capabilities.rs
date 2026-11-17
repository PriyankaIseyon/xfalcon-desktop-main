//! Privileged surface the main window may reach.
//!
//! [`APP_COMMANDS`] is the production allowlist. Registration (`lib.rs`),
//! permission generation (`build.rs`), and the window capability
//! (`capabilities/default.json`) must match it. Tests parse those files and
//! fail on drift — they do not own a second copy of the list.

use crate::preview::{PREVIEW_HTTP_HOST, PREVIEW_URI_SCHEME};

/// The exact set of privileged commands the webview may invoke.
///
/// Widening this set is a production change. Tests parse `lib.rs`,
/// `build.rs`, and `capabilities/default.json` and assert equality with this
/// list, so a command cannot land in the handler without landing here, and
/// cannot land here without being registered and admitted.
pub const APP_COMMANDS: [&str; 122] = [
    "app_bootstrap",
    "app_health",
    "app_reset",
    "auth_status",
    "auth_login",
    "auth_cancel",
    "auth_logout",
    "workspace_status",
    "chat_workspace_bootstrap",
    "chat_model_catalog",
    "chat_custom_model_list",
    "chat_custom_model_discover",
    "chat_custom_model_create",
    "chat_custom_model_update",
    "chat_custom_model_delete",
    "chat_default_model_set",
    "chat_conversation_create",
    "chat_mcp_selection_get",
    "chat_conversation_open",
    "chat_conversation_preview",
    "chat_conversation_search",
    "chat_conversation_rename",
    "chat_conversation_archive",
    "chat_conversation_delete",
    "chat_conversation_pin",
    "chat_conversation_reset_active",
    "chat_message_send",
    "chat_run_cancel",
    "chat_run_retry",
    "chat_tool_approval_resolve",
    "chat_message_edit",
    "chat_message_regenerate",
    "chat_message_feedback_list",
    "chat_message_feedback_set",
    "chat_follow_ups",
    "project_create",
    "project_pick_folder",
    "project_list",
    "project_find_by_folder",
    "project_activate",
    "project_deactivate",
    "project_revoke",
    "project_save_file",
    "project_file_revisions",
    "project_file_revision_diff",
    "project_file_restore",
    "credential_list",
    "credential_create",
    "credential_replace",
    "credential_delete",
    "mcp_server_list",
    "mcp_server_create",
    "mcp_server_update",
    "mcp_server_delete",
    "mcp_server_restore_bundled",
    "mcp_server_sign_in",
    "mcp_server_sign_out",
    "mcp_server_test",
    "project_list_directory",
    "project_file_stat",
    "project_file_read_bytes",
    "project_document_read",
    "project_office_preview",
    "project_office_preview_cancel",
    "project_office_save_pdf",
    "project_office_save_original",
    "project_preview_register",
    "project_table_rows",
    "project_schema_summary",
    "preview_document_register",
    "delight_greeting_claim",
    "delight_message_sent",
    "settings_appearance_get",
    "settings_appearance_set",
    "skill_list",
    "skill_pick",
    "skill_import",
    "skill_set_enabled",
    "skill_delete",
    "report_list",
    "report_open",
    "report_thumbnail",
    "report_thumbnail_capture_open",
    "report_thumbnail_save",
    "report_rename",
    "report_set_pinned",
    "report_delete",
    "report_refresh",
    "report_set_scope",
    "report_set_hidden",
    "report_export",
    "report_save_library_to_project",
    "report_save_to_project",
    "report_office_preview",
    "document_export_save",
    "document_export_pdf",
    "claude_runtime_status",
    "claude_runtime_set_enabled",
    "claude_runtime_set_effort",
    "chatgpt_plan_status",
    "chatgpt_plan_sign_in",
    "chatgpt_plan_sign_in_cancel",
    "chatgpt_plan_sign_out",
    "chatgpt_plan_welcome_ack",
    "diagnostics_log_renderer_error",
    "diagnostics_bundle_generate",
    "upload_choose",
    "upload_paste",
    "upload_cancel",
    "upload_retry",
    "upload_queue",
    "dataset_pick",
    "dataset_preview",
    "dataset_stage_discard",
    "dataset_upload_start",
    "dataset_list",
    "dataset_detail",
    "dataset_delete",
    "dataset_download",
    "update_check",
    "update_install",
    "update_restart",
];

/// Commands that must never appear in [`APP_COMMANDS`] or the Tauri handler.
///
/// Cardinality is pinned so the set cannot grow or shrink without an explicit
/// edit here. `write_file` / `write_text_file` stay forbidden because export
/// and report saves are the replacements for a renderer-named path
/// (spec: webview-capability-lockdown).
pub const FORBIDDEN_COMMANDS: [&str; 8] = [
    "fetch_url",
    "get_credential",
    "read_file",
    "read_text_file",
    "run_shell",
    "upload_file_chunk",
    "write_file",
    "write_text_file",
];

/// Plugin permissions the main window is granted, besides per-command `allow-*`.
///
/// Every other capability entry must be derived from [`APP_COMMANDS`]. No plugin
/// default permission set is granted to the webview. The opener gets only
/// `open-url` and its default URL scope, never `reveal-item-in-dir`.
pub const PLUGIN_PERMISSIONS: [&str; 4] = [
    // System-browser links (`src/ui/Link.tsx`) and the opener's click handler
    // call `open_url`, and its URL scope is `allow-default-urls`: http, https,
    // mailto and tel. Without the scope, `open_url` refuses every URL.
    "opener:allow-open-url",
    "opener:allow-default-urls",
    // Backend→webview events. Listen/unlisten only — the webview may not emit.
    "core:event:allow-listen",
    "core:event:allow-unlisten",
];

/// Window permissions the main window is granted on macOS only, from
/// `capabilities/macos-window.json`. They are not in [`PLUGIN_PERMISSIONS`].
///
/// The macOS title bar is an overlay (tauri.conf.json `titleBarStyle`), so the
/// app's own header is where the window is dragged from and double-clicked to
/// zoom: Tauri's `data-tauri-drag-region` needs exactly these two. Together
/// they let that header drag the window and zoom it (maximize or restore).
/// They grant nothing else: no close, no retitle, no arbitrary size or position.
pub const MACOS_WINDOW_PERMISSIONS: [&str; 2] = [
    "core:window:allow-start-dragging",
    "core:window:allow-internal-toggle-maximize",
];

/// Tauri `allow-*` identifier for one [`APP_COMMANDS`] entry.
#[must_use]
pub fn command_allow_permission(command: &str) -> String {
    format!("allow-{}", command.replace('_', "-"))
}

/// Main-window capability grant: one `allow-*` per command, then plugin
/// permissions. Derived so it cannot drift from [`APP_COMMANDS`].
#[must_use]
pub fn main_window_permissions() -> Vec<String> {
    APP_COMMANDS
        .iter()
        .copied()
        .map(command_allow_permission)
        .chain(PLUGIN_PERMISSIONS.iter().copied().map(str::to_owned))
        .collect()
}

/// The packaged app origin, in this platform's spelling: `http://tauri.localhost`
/// under WebView2, `tauri://localhost` everywhere else.
///
/// `cfg!`, never `#[cfg]`. The local Rust gate runs one host, so an
/// attribute-gated arm for the other platform is never compiled here at all —
/// the gap that already shipped a macOS-only startup bug once (see
/// `scripts/gate.sh`). A `cfg!` branch is const-folded the same way and both
/// arms still type-check on every host.
fn app_origin() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("http", "tauri.localhost")
    } else {
        ("tauri", "localhost")
    }
}

/// The preview origin, in this platform's spelling.
///
/// One origin, two spellings: wry maps a custom scheme to
/// `http://<scheme>.localhost` under WebView2 and to `<scheme>://localhost`
/// everywhere else, and the frontend's `convertFileSrc` picks the same form
/// from the user agent. Only the form this build can produce is admitted —
/// `.localhost` resolves to loopback, so admitting the http spelling on a
/// platform that never mints it would hand the webview any local port-80
/// server under that name.
fn preview_origin() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("http", PREVIEW_HTTP_HOST)
    } else {
        (PREVIEW_URI_SCHEME, "localhost")
    }
}

fn is_origin(url: &tauri::Url, (scheme, host): (&str, &str)) -> bool {
    url.scheme() == scheme && url.host_str() == Some(host) && url.port().is_none()
}

pub fn is_exact_app_origin(url: &tauri::Url, development: bool) -> bool {
    let packaged = is_origin(url, app_origin());
    let exact_development_origin = development
        && url.scheme() == "http"
        && matches!(url.host_str(), Some("localhost") | Some("127.0.0.1"))
        && url.port() == Some(1420);

    let active_origin = if development {
        exact_development_origin
    } else {
        packaged
    };
    active_origin && url.username().is_empty() && url.password().is_none()
}

fn is_preview_origin(url: &tauri::Url) -> bool {
    is_origin(url, preview_origin()) && url.username().is_empty() && url.password().is_none()
}

/// Which documents the webview may load — and, on WebKit, that includes every
/// SUBFRAME, which is why the preview origin has to be named here.
///
/// `on_navigation` is not a top-level-only hook, and the two engines disagree
/// about which loads reach it. WebView2 raises `NavigationStarting` for the
/// main document alone (iframes get the separate `FrameNavigationStarting`,
/// which wry does not subscribe to), so on Windows a preview frame is never
/// policed here. WKWebView asks `decidePolicyForNavigationAction:` for EVERY
/// frame and wry forwards all of them unfiltered, so on macOS the same frame
/// IS policed here — and a policy naming only the app origin answers `Cancel`.
/// The document then never loads, the empty frame still fires `load`, and the
/// preview card settles as a blank white panel with nothing logged anywhere.
///
/// Admitting the preview origin grants nothing the app has not already
/// granted it: the app CSP's `frame-src` names this origin, the scheme has one
/// handler serving in-memory documents and three bundled faces, and every
/// preview frame is sandboxed without `allow-top-navigation`, so a previewed
/// document cannot navigate the app away from itself.
pub fn allows_navigation(url: &tauri::Url, development: bool) -> bool {
    is_exact_app_origin(url, development) || is_preview_origin(url)
}

#[cfg(test)]
mod tests {
    use super::{
        command_allow_permission, main_window_permissions, APP_COMMANDS, FORBIDDEN_COMMANDS,
        PLUGIN_PERMISSIONS,
    };
    use std::collections::BTreeSet;

    #[test]
    fn app_commands_are_unique() {
        let unique: BTreeSet<_> = APP_COMMANDS.iter().copied().collect();
        assert_eq!(unique.len(), APP_COMMANDS.len());
    }

    #[test]
    fn forbidden_commands_never_overlap_the_allowlist() {
        for forbidden in FORBIDDEN_COMMANDS {
            assert!(
                !APP_COMMANDS.contains(&forbidden),
                "{forbidden} is forbidden and must not be allowlisted"
            );
        }
    }

    #[test]
    fn main_window_permissions_are_derived_from_the_allowlist() {
        let expected_commands: BTreeSet<String> = APP_COMMANDS
            .iter()
            .copied()
            .map(command_allow_permission)
            .collect();
        let granted: BTreeSet<String> = main_window_permissions().into_iter().collect();
        let plugins: BTreeSet<String> = PLUGIN_PERMISSIONS
            .iter()
            .copied()
            .map(str::to_owned)
            .collect();

        assert_eq!(
            granted.len(),
            expected_commands.len() + plugins.len(),
            "derived grant must be commands plus plugins, with no extras"
        );
        assert!(expected_commands.is_subset(&granted));
        assert!(plugins.is_subset(&granted));
    }
}
