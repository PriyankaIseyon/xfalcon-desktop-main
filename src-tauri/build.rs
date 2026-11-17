fn main() {
    embed_app_manifest();
    pin_tenant_config();
    let manifest = tauri_build::AppManifest::new().commands(&[
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
        "chat_conversation_rename",
        "chat_conversation_archive",
        "chat_conversation_delete",
        "chat_conversation_pin",
        "chat_conversation_reset_active",
        "chat_conversation_search",
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
    ]);
    // `new_without_app_manifest`: `embed_app_manifest` above supplies the one
    // manifest every target in this package gets. Leaving tauri's in place
    // too puts two RT_MANIFEST id-1 resources in the binary's link and fails
    // it outright.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(manifest)
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    )
    .expect("failed to build Tauri application");
}

/// Compiles the SHA-256 of `resources/tenant-config.json` into the binary as
/// `XFALCON_TENANT_CONFIG_SHA256`.
///
/// The file ships beside the binary as a bundle resource, and on a per-user
/// Windows install it sits in a directory the user can write. Unchecked, anyone
/// could point an installed copy at an issuer and a DA they run themselves and
/// sign in as whoever they like. `crate::auth::config` refuses a bundled file
/// whose digest is not this one, so the only tenant a build trusts is the one it
/// was compiled with.
///
/// A digest, not the file itself: the file stays the one source the bundle
/// verification in `.github/workflows/desktop-build.yml` reads, and the release
/// workflow writes it before cargo runs, so the two cannot disagree.
fn pin_tenant_config() {
    use sha2::Digest as _;

    println!("cargo:rerun-if-changed=resources/tenant-config.json");
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set for a build script");
    let path = std::path::Path::new(&manifest_dir).join("resources/tenant-config.json");
    let contents = std::fs::read(&path).expect("resources/tenant-config.json is tracked");
    let digest = sha2::Sha256::digest(&contents);
    println!("cargo:rustc-env=XFALCON_TENANT_CONFIG_SHA256={digest:x}");
}

/// Embeds `app.manifest` into **every** cargo target in this package.
///
/// `tauri_build` normally supplies the Windows manifest through a resource
/// script, but that script is linked only into the binary. A cargo test
/// harness is a separate link unit with no resource script, so once
/// `tauri-plugin-dialog` (via `rfd`) put `comctl32!TaskDialogIndirect` in the
/// import table, every test executable began dying at image load with
/// STATUS_ENTRYPOINT_NOT_FOUND — the export exists only in the side-by-side
/// comctl32 v6 assembly, which a manifest is what requests.
///
/// Cargo can scope linker arguments to bins or to declared `[[test]]`
/// targets, but not to "every target except bins", and the executable that
/// breaks is the *lib* unit-test harness, which the `-tests` scope does not
/// cover. So the manifest goes on unscoped — which in turn requires
/// `WindowsAttributes::new_without_app_manifest()` below, because two
/// RT_MANIFEST resources with id 1 in a single link is a hard linker error
/// (`CVT1100: duplicate resource` / `LNK1123`), and that is exactly what
/// happened when this was first added alongside tauri's own manifest.
///
/// `app.manifest` is byte-equivalent to `tauri_build`'s default, so the
/// shipping binary is unaffected.
fn embed_app_manifest() {
    println!("cargo:rerun-if-changed=app.manifest");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        return;
    }
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR is always set for a build script");
    let manifest = std::path::Path::new(&manifest_dir).join("app.manifest");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
