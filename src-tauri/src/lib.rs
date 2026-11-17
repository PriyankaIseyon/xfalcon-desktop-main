use std::sync::{Arc, Mutex};

use app::commands::{
    app_bootstrap, app_health, app_reset, project_activate, project_create, project_deactivate,
    project_find_by_folder, project_list, project_pick_folder, project_revoke, AppRuntimeState,
};
use approvals::{
    project_file_restore, project_file_revision_diff, project_file_revisions, project_save_file,
    ApprovalsService,
};
use auth::{
    auth_cancel, auth_login, auth_logout, auth_status, init_bundled_tenant_config, AuthSessionStore,
};
use chat::{
    chat_conversation_archive, chat_conversation_create, chat_conversation_delete,
    chat_conversation_open, chat_conversation_pin, chat_conversation_preview,
    chat_conversation_rename, chat_conversation_reset_active, chat_conversation_search,
    chat_custom_model_create, chat_custom_model_delete, chat_custom_model_discover,
    chat_custom_model_list, chat_custom_model_update, chat_default_model_set, chat_follow_ups,
    chat_mcp_selection_get, chat_message_edit, chat_message_feedback_list,
    chat_message_feedback_set, chat_message_regenerate, chat_message_send, chat_model_catalog,
    chat_run_cancel, chat_run_retry, chat_tool_approval_resolve, chat_workspace_bootstrap,
};
use credentials::{credential_create, credential_delete, credential_list, credential_replace};
use datasets::{
    dataset_delete, dataset_detail, dataset_download, dataset_list, dataset_pick, dataset_preview,
    dataset_stage_discard, dataset_upload_start, register_drop_handler, Staging,
};
use delight::commands::{delight_greeting_claim, delight_message_sent};
use desktop_update::{update_check, update_install, update_restart, DesktopUpdateService};
use diagnostics::{diagnostics_bundle_generate, diagnostics_log_renderer_error};
use export::{document_export_pdf, document_export_save};
use mcp::commands::{
    mcp_server_create, mcp_server_delete, mcp_server_list, mcp_server_restore_bundled,
    mcp_server_sign_in, mcp_server_sign_out, mcp_server_test, mcp_server_update, McpServerRuntime,
};
use path_grants::PathGrants;
use persistence::{CachedSecretStore, KeyringSecretStore, SecretStore};
use plan_providers::commands::{
    chatgpt_plan_sign_in, chatgpt_plan_sign_in_cancel, chatgpt_plan_sign_out, chatgpt_plan_status,
    chatgpt_plan_welcome_ack, claude_runtime_set_effort, claude_runtime_set_enabled,
    claude_runtime_status,
};
use preview::{
    handle_preview_request, preview_document_register, PreviewDocumentStore, PREVIEW_URI_SCHEME,
};
use projects::{
    project_document_read, project_file_read_bytes, project_file_stat, project_list_directory,
    project_office_preview, project_office_preview_cancel, project_office_save_original,
    project_office_save_pdf, project_preview_register, project_schema_summary, project_table_rows,
    report_office_preview, ProjectsService,
};
use reports::{
    report_delete, report_export, report_list, report_open, report_refresh, report_rename,
    report_save_library_to_project, report_save_to_project, report_set_hidden, report_set_pinned,
    report_set_scope, report_thumbnail, report_thumbnail_capture_open, report_thumbnail_save,
};
use security::account_cleanup::{
    AccountCleanup, CurrentAccountScope, FixedAccountResources, JournalSlot, SessionVault,
};
use security::browsing_data::{
    spawn_acknowledgement_timeout, StartupMigration, StorageOrigin, ACKNOWLEDGEMENT_TIMEOUT,
};
use security::capabilities::allows_navigation;
use security::panic_boundary::install_redacted_panic_hook;
use security::temp_cleanup::TempCleanup;
use settings::commands::{settings_appearance_get, settings_appearance_set};
use skills::commands::{skill_delete, skill_import, skill_list, skill_pick, skill_set_enabled};
use skills::register_skill_drop_handler;
use tauri::{webview::PageLoadEvent, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
use uploads::{
    service::UploadService, upload_cancel, upload_choose, upload_paste, upload_queue, upload_retry,
};
use work_barrier::WorkBarrier;
use workspace::workspace_status;

pub mod agent;
pub mod app;
pub mod approvals;
pub mod audit;
pub mod auth;
pub mod chat;
pub(crate) mod clients;
pub mod credentials;
pub mod datasets;
pub mod delight;
mod desktop_update;
pub mod diagnostics;
pub(crate) mod documents;
pub mod domain;
pub mod export;
pub(crate) mod mcp;
pub(crate) mod path_grants;
pub mod persistence;
mod plan_providers;
pub mod preview;
pub mod projects;
pub(crate) mod providers;
pub mod reports;
pub mod security;
pub mod settings;
pub mod skills;
pub mod tools;
mod update_schedule;
pub mod uploads;
mod window_chrome;
mod work_barrier;
pub mod workspace;

#[cfg(test)]
mod prompt_scan;

#[cfg(debug_assertions)]
fn load_workspace_dotenv() {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    if let Some(root) = manifest.parent() {
        let _ = dotenvy::from_path(root.join(".env"));
    }
}

#[cfg(not(debug_assertions))]
fn load_workspace_dotenv() {}

/// Structured logs to `<app-local-data>/logs/` (daily rotation) and stderr.
///
/// The audit pipeline (`crate::audit::publisher`) deliberately never installs
/// a global subscriber; this is the one place that does. Log content policy:
/// error kinds, ids, and step outcomes — never tokens, secrets, message text,
/// or file paths from user projects. Output is also redacted as it is written
/// (`RedactingMakeWriter`), a second line of defence behind that policy, and
/// existing active log files are redacted at startup. Every line begins with
/// an RFC 3339 UTC timestamp. `XFALCON_LOG` tunes the filter (default `info`,
/// e.g. `xfalcon_desktop=debug`).
fn init_logging(app_local_data: &std::path::Path) {
    use crate::diagnostics::{redact_existing_logs, RedactingMakeWriter};
    use tracing_subscriber::layer::SubscriberExt as _;
    use tracing_subscriber::util::SubscriberInitExt as _;

    let directory = app_local_data.join("logs");
    // Must run before the appender is built: it opens today's file and keeps
    // that handle, and the redaction replaces files by rename.
    let redacted = redact_existing_logs(&directory);
    let file_appender = tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("xfalcon-desktop")
        .filename_suffix("log")
        .max_log_files(7)
        .build(&directory);
    let Ok(file_appender) = file_appender else {
        return;
    };
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    // The guard must live for the process lifetime or buffered lines are lost.
    std::mem::forget(guard);
    let filter = tracing_subscriber::EnvFilter::try_from_env("XFALCON_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(RedactingMakeWriter::new(file_writer))
                .with_ansi(false)
                .with_target(true),
        )
        // No colour escapes: they would split the `key=value` syntax the redaction patterns match.
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(RedactingMakeWriter::new(std::io::stderr))
                .with_ansi(false),
        )
        .try_init();
    match crate::security::panic_boundary::configure_panic_log_directory(&directory) {
        Ok(()) => {}
        Err(error) => tracing::error!(error = %error, "panic log directory configuration failed"),
    }
    match crate::diagnostics::prune_legacy_log_files(&directory) {
        Ok(removed) => tracing::info!(removed, "legacy diagnostic logs pruned"),
        Err(error) => tracing::warn!(error = %error, "legacy diagnostic log pruning failed"),
    }
    match redacted {
        Ok(redacted) => tracing::info!(redacted, "existing log files redacted"),
        Err(error) => tracing::warn!(error = %error, "existing log redaction failed"),
    }
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "logging initialized");
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Headless full purge for uninstallers: no window, no log file, no keyring
    // or Tauri setup. Must stay the first statement.
    if std::env::args_os()
        .skip(1)
        .any(|argument| argument == "--purge-local-data")
    {
        std::process::exit(security::device_purge::purge_exit_code());
    }
    install_redacted_panic_hook();
    load_workspace_dotenv();
    let startup_migration = Arc::new(Mutex::new(None::<Arc<StartupMigration>>));
    let page_load_migration = startup_migration.clone();
    let setup_migration = startup_migration.clone();
    let builder = tauri::Builder::default();
    #[cfg(any(target_os = "macos", windows, target_os = "linux"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(
        |app, _arguments, _working_directory| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        },
    ));
    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        // Previewed documents get a real, non-app origin with a
        // per-response CSP — the mechanism behind chat's HTML/SVG preview
        // cards (spec: preview-protocol).
        .register_uri_scheme_protocol(PREVIEW_URI_SCHEME, handle_preview_request)
        .plugin(
            tauri::plugin::Builder::<_, ()>::new("webview-boundary")
                .on_navigation(|_webview, url| allows_navigation(url, tauri::is_dev()))
                .build(),
        )
        .on_page_load(move |webview, payload| {
            if webview.label() != "main" || payload.event() != PageLoadEvent::Finished {
                return;
            }
            let migration = page_load_migration
                .lock()
                .ok()
                .and_then(|slot| slot.clone());
            let Some(migration) = migration else {
                webview.app_handle().exit(1);
                return;
            };
            match migration.handle_loaded_url(payload.url()) {
                Ok(true) => {
                    if webview.window().show().is_ok() {
                        migration.mark_ready();
                    } else {
                        webview.app_handle().exit(1);
                    }
                }
                // Not the entry document yet. The storage-migration page loads
                // first and then navigates to the acknowledgement URL, so one
                // unrecognized load is expected. Logged because the failure mode
                // when it is the LAST load is invisible: no window, no crash, and
                // a silent exit on the deadline below.
                Ok(false) => tracing::warn!(
                    url = %payload.url(),
                    "startup: loaded document is not the entry document; window stays hidden"
                ),
                Err(_) => webview.app_handle().exit(1),
            }
        })
        .setup(move |app| {
            let app_local_data = app.path().app_local_data_dir()?;
            init_logging(&app_local_data);
            let temp_cleanup = Arc::new(TempCleanup::new([
                datasets::staging::staging_directory(&app_local_data),
                uploads::service::snapshot_directory(&app_local_data),
            ]));
            app.manage(Arc::clone(&temp_cleanup));
            temp_cleanup.start_background();
            app.manage(Arc::new(WorkBarrier::default()));
            app.manage(Arc::new(DesktopUpdateService::default()));
            update_schedule::spawn(app.handle().clone());
            // A distributed build carries its tenant's auth config bundled at
            // `resources/tenant-config.json` (see `.github/workflows/desktop-build.yml`),
            // accepted only if it is the file this binary was compiled with
            // (`build.rs`). Local dev has no such file and falls through to
            // `.env`/process env inside `AuthConfig::from_env`, which a release
            // build never does (spec: desktop-tenant-config).
            let bundled_tenant_config = app
                .path()
                .resolve(
                    "resources/tenant-config.json",
                    tauri::path::BaseDirectory::Resource,
                )
                .ok();
            init_bundled_tenant_config(bundled_tenant_config.as_deref());
            let vault = Arc::new(SessionVault::default());
            let accounts = Arc::new(CurrentAccountScope::default());
            let journal = Arc::new(JournalSlot::default());
            // Both stores are wrapped in `CachedSecretStore` so a device key
            // costs one keychain read per launch instead of one per sealed
            // record. See its doc comment: uncached, every decrypted message
            // was a `securityd` round trip, and on a build the keychain ACL
            // does not recognize, a user-facing authorization prompt.
            let secrets: Arc<dyn SecretStore> = Arc::new(CachedSecretStore::new(Arc::new(
                KeyringSecretStore::new("ai.xfalcon.desktop")
                    .map_err(|_| std::io::Error::other("secret store unavailable"))?,
            )));
            let credential_secrets: Arc<dyn SecretStore> =
                Arc::new(CachedSecretStore::new(Arc::new(
                    KeyringSecretStore::new("ai.xfalcon.desktop.provider-keys").map_err(|_| {
                        std::io::Error::other("credential secret store unavailable")
                    })?,
                )));
            // The local security audit log. `None` when it cannot be opened:
            // auditing is then off for the session and nothing else changes.
            let audit_hub = audit::open_hub(&app_local_data, secrets.clone());
            let audit_publisher = audit_hub.as_ref().map(|hub| hub.publisher());
            if let Some(hub) = &audit_hub {
                app.manage(Arc::clone(hub));
            }
            // Identity is bound late, through the hub: it only exists after a
            // sign-in, and this runs before any account has one. `auth_login`
            // binds it and `auth_logout` clears it.
            let projects = ProjectsService::new(
                app_local_data.clone(),
                accounts.clone(),
                secrets.clone(),
            );
            let projects = Arc::new(match &audit_publisher {
                Some(publisher) => projects.with_audit(publisher.clone()),
                None => projects,
            });
            let approvals = ApprovalsService::new(
                app_local_data.clone(),
                accounts.clone(),
                secrets.clone(),
            );
            let approvals = Arc::new(match &audit_publisher {
                Some(publisher) => approvals.with_audit(publisher.clone()),
                None => approvals,
            });
            let chat_approvals = approvals.clone();
            app.manage(approvals);
            let registry = FixedAccountResources::new(
                app_local_data.clone(),
                accounts.clone(),
                vault,
                journal.clone(),
                secrets.clone(),
            )
            .with_credential_secrets(credential_secrets.clone())
            .with_project_cancellation(projects.clone())
            .registry()
            .map_err(|_| std::io::Error::other("invalid local cleanup registry"))?;
            let auth_sessions = Arc::new(AuthSessionStore::new());
            // The audit identity follows the session: whenever a session ends
            // (logout, refused renewal, cancelled or replaced sign-in) the actor
            // is forgotten, so no later record names a user who is not signed in.
            if let Some(hub) = &audit_hub {
                let hub = Arc::clone(hub);
                auth_sessions.set_session_end_hook(move || hub.clear_identity());
            }
            let mcp_auth_sessions = auth_sessions.clone();
            let upload_sessions = auth_sessions.clone();
            let mcp_credential_secrets = credential_secrets.clone();
            app.manage(AppRuntimeState::new(
                AccountCleanup::new(registry),
                accounts,
                journal,
                auth_sessions,
                app_local_data.clone(),
                secrets.clone(),
                credential_secrets,
                projects,
                chat_approvals,
            ));
            if let Some(publisher) = audit_publisher {
                app.state::<AppRuntimeState>().chat().set_audit(publisher);
            }

            // Registered-MCP-server management is its own managed state rather
            // than a field on `AppRuntimeState`: the Settings surface must be
            // able to dial a server *now* to test it, and doing that through a
            // session an in-flight run is using would invalidate that run's
            // connection. Same legacy-provider secret store as
            // `CredentialRuntime`, so old `xfdc:v1` rows can migrate with the
            // device key that still seals registered-server credentials.
            app.manage(McpServerRuntime::new(
                mcp_auth_sessions,
                app_local_data.clone(),
                mcp_credential_secrets,
            ));

            let upload_handle = app.handle().clone();
            app.manage(Arc::new(
                UploadService::new(
                    upload_sessions,
                    Arc::new(move |queue| {
                        let _ = upload_handle.emit("xfalcon://upload-queue", queue);
                    }),
                )
                .map(|service| {
                    service.with_snapshot_root(uploads::service::snapshot_directory(&app_local_data))
                        .with_temp_cleanup(Arc::clone(&temp_cleanup))
                })
                .map_err(|_| std::io::Error::other("upload runtime unavailable"))?,
            ));

            // Staged datasets are process-lifetime: a file picked but not
            // uploaded is a decision in progress, not state to persist.
            app.manage(Arc::new(Staging::new()));
            // Chosen folders and files, held in the backend so the webview
            // names them by id only (see `crate::path_grants`).
            app.manage(Arc::new(PathGrants::default()));

            // Preview documents are process-lifetime, in-memory only.
            app.manage(Arc::new(PreviewDocumentStore::default()));

            let migration = Arc::new(
                StartupMigration::new(app_local_data, StorageOrigin::for_runtime(tauri::is_dev()))
                    .map_err(|_| std::io::Error::other("webview storage migration unavailable"))?,
            );
            let mut window_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .cloned()
                .ok_or_else(|| std::io::Error::other("main window configuration unavailable"))?;
            window_config.url = WebviewUrl::App(migration.initial_page().into());
            *setup_migration
                .lock()
                .map_err(|_| std::io::Error::other("webview storage migration unavailable"))? =
                Some(migration.clone());
            let main_window = WebviewWindowBuilder::from_config(app.handle(), &window_config)?.build()?;
            window_chrome::keep_window_buttons_placed(&main_window);

            // Dropped files are staged in the backend, so the webview never
            // receives a path (spec: dataset-uploads). Registered after the
            // window exists, because that is what it subscribes to.
            register_drop_handler(app.handle());
            register_skill_drop_handler(app.handle());

            let app_handle = app.handle().clone();
            spawn_acknowledgement_timeout(
                migration.ready_flag(),
                ACKNOWLEDGEMENT_TIMEOUT,
                move || {
                    // Say why before going. Without this the person sees a dock
                    // icon, no window, and the app vanishing after 30 seconds,
                    // with nothing in any log to explain it.
                    tracing::error!(
                        timeout_secs = ACKNOWLEDGEMENT_TIMEOUT.as_secs(),
                        "startup: entry document was never recognized, exiting on the acknowledgement deadline"
                    );
                    app_handle.exit(1)
                },
            )
            .map_err(|_| std::io::Error::other("startup timeout unavailable"))?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_bootstrap,
            app_health,
            app_reset,
            auth_status,
            auth_login,
            auth_cancel,
            auth_logout,
            workspace_status,
            chat_workspace_bootstrap,
            chat_model_catalog,
            chat_custom_model_list,
            chat_custom_model_discover,
            chat_custom_model_create,
            chat_custom_model_update,
            chat_custom_model_delete,
            chat_default_model_set,
            chat_conversation_create,
            chat_mcp_selection_get,
            chat_conversation_open,
            chat_conversation_preview,
            chat_conversation_search,
            chat_conversation_rename,
            chat_conversation_archive,
            chat_conversation_delete,
            chat_conversation_pin,
            chat_conversation_reset_active,
            chat_message_send,
            chat_run_cancel,
            chat_run_retry,
            chat_tool_approval_resolve,
            chat_message_edit,
            chat_message_regenerate,
            chat_message_feedback_list,
            chat_message_feedback_set,
            chat_follow_ups,
            project_create,
            project_list,
            project_pick_folder,
            project_find_by_folder,
            project_activate,
            project_deactivate,
            project_revoke,
            project_save_file,
            project_file_revisions,
            project_file_revision_diff,
            project_file_restore,
            credential_list,
            credential_create,
            credential_replace,
            credential_delete,
            mcp_server_list,
            mcp_server_create,
            mcp_server_update,
            mcp_server_delete,
            mcp_server_restore_bundled,
            mcp_server_sign_in,
            mcp_server_sign_out,
            mcp_server_test,
            project_list_directory,
            project_file_stat,
            project_file_read_bytes,
            project_document_read,
            project_office_preview,
            project_office_preview_cancel,
            project_office_save_pdf,
            report_office_preview,
            project_office_save_original,
            project_preview_register,
            project_table_rows,
            project_schema_summary,
            preview_document_register,
            delight_greeting_claim,
            delight_message_sent,
            settings_appearance_get,
            settings_appearance_set,
            skill_list,
            skill_pick,
            skill_import,
            skill_set_enabled,
            skill_delete,
            report_list,
            report_open,
            report_thumbnail,
            report_thumbnail_capture_open,
            report_thumbnail_save,
            report_rename,
            report_set_pinned,
            report_delete,
            report_refresh,
            report_set_scope,
            report_set_hidden,
            report_export,
            report_save_library_to_project,
            report_save_to_project,
            document_export_save,
            document_export_pdf,
            claude_runtime_status,
            claude_runtime_set_enabled,
            claude_runtime_set_effort,
            chatgpt_plan_status,
            chatgpt_plan_sign_in,
            chatgpt_plan_sign_in_cancel,
            chatgpt_plan_sign_out,
            chatgpt_plan_welcome_ack,
            diagnostics_log_renderer_error,
            diagnostics_bundle_generate,
            upload_choose,
            upload_paste,
            upload_cancel,
            upload_retry,
            upload_queue,
            dataset_pick,
            dataset_preview,
            dataset_stage_discard,
            dataset_upload_start,
            dataset_list,
            dataset_detail,
            dataset_delete,
            dataset_download,
            update_check,
            update_install,
            update_restart,
        ])
        .run(tauri::generate_context!())
        .expect("error while running xFalcon desktop application");
}
