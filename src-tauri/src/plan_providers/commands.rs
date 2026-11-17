//! The eight commands the plan-provider surface uses.
//!
//! The account comes from the backend's account scope, never from the webview.
//! A command that acts on a ChatGPT sign-in needs a signed-in account and
//! fails with a short message without one. Errors cross as plain strings: the
//! surface shows its own copy for any rejection.

use std::sync::Arc;

use reqwest::Url;
use tauri::Manager as _;
#[cfg(not(target_os = "macos"))]
use tauri_plugin_opener::OpenerExt as _;
use xfalcon_contracts::plan_providers::{
    ChatgptPlanStatusView, ClaudeEffortInput, ClaudeRuntimeStatusView, PlanEnableInput,
};

use super::PlanProviders;
use crate::app::commands::AppRuntimeState;
use crate::providers::chatgpt_plan::session::{BrowserOpenError, BrowserOpener};
use crate::security::account_cleanup::ValidatedAccountId;

const SIGN_IN_FIRST: &str = "Sign in to xFalcon first.";
const SETTING_NOT_SAVED: &str = "The setting could not be saved.";
const WELCOME_NOT_SAVED: &str = "The welcome notice could not be saved.";

fn providers(app: &tauri::AppHandle) -> Arc<PlanProviders> {
    app.state::<AppRuntimeState>().plan_providers().clone()
}

/// The signed-in account, or `None` when nobody is signed in. A failure to
/// read the account scope is not "signed out", so it is an error.
fn current_account(app: &tauri::AppHandle) -> Result<Option<ValidatedAccountId>, String> {
    app.state::<AppRuntimeState>()
        .accounts()
        .current()
        .map_err(|_| SIGN_IN_FIRST.to_owned())
}

fn require_account(app: &tauri::AppHandle) -> Result<ValidatedAccountId, String> {
    current_account(app)?.ok_or_else(|| SIGN_IN_FIRST.to_owned())
}

#[tauri::command]
pub(crate) async fn claude_runtime_status(
    app: tauri::AppHandle,
) -> Result<ClaudeRuntimeStatusView, String> {
    Ok(providers(&app).claude_status(true).await)
}

#[tauri::command]
pub(crate) async fn claude_runtime_set_enabled(
    app: tauri::AppHandle,
    input: PlanEnableInput,
) -> Result<ClaudeRuntimeStatusView, String> {
    providers(&app)
        .claude_set_enabled(input.enabled)
        .await
        .map_err(|_| SETTING_NOT_SAVED.to_owned())
}

/// Sets the level new Claude chats start with. Works while Claude is off.
#[tauri::command]
pub(crate) async fn claude_runtime_set_effort(
    app: tauri::AppHandle,
    input: ClaudeEffortInput,
) -> Result<ClaudeRuntimeStatusView, String> {
    providers(&app)
        .claude_set_effort(input.effort)
        .await
        .map_err(|_| SETTING_NOT_SAVED.to_owned())
}

#[tauri::command]
pub(crate) async fn chatgpt_plan_status(
    app: tauri::AppHandle,
) -> Result<ChatgptPlanStatusView, String> {
    let account = current_account(&app)?;
    Ok(providers(&app).chatgpt_status(account.as_ref()).await)
}

#[tauri::command]
pub(crate) async fn chatgpt_plan_sign_in(
    app: tauri::AppHandle,
) -> Result<ChatgptPlanStatusView, String> {
    let account = require_account(&app)?;
    let opener = TauriBrowserOpener(app.clone());
    Ok(providers(&app).chatgpt_sign_in(&account, &opener).await)
}

#[tauri::command]
pub(crate) fn chatgpt_plan_sign_in_cancel(app: tauri::AppHandle) -> Result<(), String> {
    if let Some(account) = current_account(&app)? {
        providers(&app).chatgpt_sign_in_cancel(&account);
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn chatgpt_plan_sign_out(
    app: tauri::AppHandle,
) -> Result<ChatgptPlanStatusView, String> {
    let account = require_account(&app)?;
    Ok(providers(&app).chatgpt_sign_out(&account).await)
}

/// Runs on the async pool, not the main thread: the session read and write reach
/// the keychain, and a macOS Keychain prompt there would freeze the window.
#[tauri::command]
pub(crate) async fn chatgpt_plan_welcome_ack(
    app: tauri::AppHandle,
) -> Result<ChatgptPlanStatusView, String> {
    let account = require_account(&app)?;
    let service = providers(&app);
    tokio::task::spawn_blocking(move || service.chatgpt_welcome_ack(&account))
        .await
        .map_err(|_| WELCOME_NOT_SAVED.to_owned())
}

/// Opens sign-in pages in the system browser. Only `https` URLs reach it.
struct TauriBrowserOpener(tauri::AppHandle);

impl BrowserOpener for TauriBrowserOpener {
    fn open(&self, url: &Url) -> Result<(), BrowserOpenError> {
        if !is_openable(url) {
            return Err(BrowserOpenError);
        }
        open_in_browser(&self.0, url)
    }
}

/// Windows and Linux: the plugin's launcher. It reports a launch it started, not
/// one that succeeded, so a missing handler there is not caught here.
#[cfg(not(target_os = "macos"))]
fn open_in_browser(app: &tauri::AppHandle, url: &Url) -> Result<(), BrowserOpenError> {
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|_| BrowserOpenError)
}

/// macOS: the plugin's launcher is detached and never reads the exit status, so a
/// failed launch looked like success and the sign-in waited out its full timeout.
/// `open` exits non-zero when no handler takes the URL, so its status decides here.
#[cfg(target_os = "macos")]
fn open_in_browser(_app: &tauri::AppHandle, url: &Url) -> Result<(), BrowserOpenError> {
    launch_with_open(url)
}

/// How long macOS's `open` may take before the sign-in gives up on it.
#[cfg(target_os = "macos")]
const MACOS_LAUNCH_LIMIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Runs `open` on its own thread, so the bound holds even when `open` hangs. The
/// caller waits at most `MACOS_LAUNCH_LIMIT`, and a late exit is not read.
#[cfg(target_os = "macos")]
fn launch_with_open(url: &Url) -> Result<(), BrowserOpenError> {
    let (program, args) = macos_open_command(url.as_str());
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let status = std::process::Command::new(program).args(args).status();
        // The receiver is gone once the bound has passed. Nothing is lost by that.
        let _ = sender.send(status);
    });
    match receiver.recv_timeout(MACOS_LAUNCH_LIMIT) {
        Ok(Ok(status)) if status.success() => Ok(()),
        _ => Err(BrowserOpenError),
    }
}

/// The program and arguments that open `url` with macOS's `open`. The `--` ends
/// option parsing, so the URL is never read as a flag.
#[cfg(any(target_os = "macos", test))]
fn macos_open_command(url: &str) -> (&'static str, [String; 2]) {
    ("/usr/bin/open", ["--".to_owned(), url.to_owned()])
}

fn is_openable(url: &Url) -> bool {
    url.scheme() == "https"
}

#[cfg(test)]
mod tests {
    use reqwest::Url;

    use super::{is_openable, macos_open_command};

    #[test]
    fn macos_launch_runs_the_system_open_with_the_url_after_a_separator() {
        let (program, args) = macos_open_command("https://chatgpt.com/auth/login?state=abc");

        assert_eq!(program, "/usr/bin/open");
        assert_eq!(
            args,
            [
                "--".to_owned(),
                "https://chatgpt.com/auth/login?state=abc".to_owned(),
            ]
        );
    }

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("test URL must parse")
    }

    #[test]
    fn https_urls_may_be_opened() {
        assert!(is_openable(&url("https://chatgpt.com/auth/login")));
    }

    #[test]
    fn other_schemes_are_never_opened() {
        for raw in [
            "http://chatgpt.com/auth/login",
            "file:///etc/hosts",
            "javascript:alert(1)",
            "xfalcon://callback",
        ] {
            assert!(!is_openable(&url(raw)), "{raw} must not be opened");
        }
    }
}
