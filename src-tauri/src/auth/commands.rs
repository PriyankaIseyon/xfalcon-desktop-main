use std::sync::Arc;

use tauri::Manager;
use xfalcon_contracts::auth::{AuthLoginError, AuthLoginErrorCode, SessionProjection};

use crate::audit::{emit, AuditAction, AuditHub, AuditOutcome, AuditResourceType};
use crate::auth::service::AuthServiceError;
use crate::datasets::Staging;
use crate::path_grants::PathGrants;
use crate::uploads::service::UploadService;
use crate::{app::commands::AppRuntimeState, auth::canonical_account_id};

#[tauri::command]
pub fn auth_status(app: tauri::AppHandle) -> SessionProjection {
    app.state::<AppRuntimeState>().auth().status()
}

/// Runs the browser sign-in and binds the identity it returns.
///
/// A refusal crosses as a code and is logged here with its full message. The
/// message is what an operator needs — it names the issuer, the redirect URI,
/// and the loopback port — and is exactly what must not reach the webview.
#[tauri::command]
pub async fn auth_login(app: tauri::AppHandle) -> Result<SessionProjection, AuthLoginError> {
    let projection = match app
        .state::<AppRuntimeState>()
        .auth()
        .begin_login(&app)
        .await
    {
        Ok(projection) => projection,
        Err(error) => {
            let code = login_error_code(&error);
            tracing::warn!(%error, ?code, "sign-in did not complete");
            emit::record(
                &app,
                AuditAction::AuthLoginFailed,
                AuditResourceType::Session,
                &login_code_label(&code),
                AuditOutcome::Failure,
            );
            return Err(AuthLoginError::new(code));
        }
    };
    if bind_account_scope(&app.state::<AppRuntimeState>(), &projection).is_err() {
        // A session with nowhere to write is worse than none: the next command
        // would read another account's scope. Dropped rather than kept.
        tracing::warn!("signed in identity could not be bound to local storage");
        emit::record(
            &app,
            AuditAction::AuthLoginFailed,
            AuditResourceType::Session,
            "account_bind_failed",
            AuditOutcome::Failure,
        );
        app.state::<AppRuntimeState>().auth().logout();
        clear_transient_account_scope(&app).await;
        let _ = app.state::<AppRuntimeState>().accounts().clear();
        return Err(AuthLoginError::new(AuthLoginErrorCode::AccountBindFailed));
    }
    bind_audit_identity(&app, &projection);
    emit::record(
        &app,
        AuditAction::AuthLogin,
        AuditResourceType::Session,
        "session",
        AuditOutcome::Success,
    );
    app.state::<AppRuntimeState>()
        .credentials()
        .spawn_legacy_provider_key_migration();
    Ok(projection)
}

#[tauri::command]
pub fn auth_cancel(app: tauri::AppHandle) -> SessionProjection {
    app.state::<AppRuntimeState>().auth().cancel_login()
}

#[tauri::command]
pub async fn auth_logout(app: tauri::AppHandle) -> SessionProjection {
    // Recorded before the identity is cleared, so the event names who signed out.
    emit::record(
        &app,
        AuditAction::AuthLogout,
        AuditResourceType::Session,
        "session",
        AuditOutcome::Success,
    );
    if let Some(hub) = app.try_state::<Arc<AuditHub>>() {
        hub.clear_identity();
    }
    app.state::<AppRuntimeState>()
        .credentials()
        .cancel_legacy_provider_key_migration();
    let projection = app.state::<AppRuntimeState>().auth().logout();
    clear_transient_account_scope(&app).await;
    let _ = app.state::<AppRuntimeState>().accounts().clear();
    projection
}

async fn clear_transient_account_scope(app: &tauri::AppHandle) {
    app.state::<Arc<UploadService>>()
        .clear_account_scope()
        .await;
    app.state::<Arc<Staging>>().clear_account_scope();
    // The catalogue is published under the account that read it, so the next
    // account would miss on it anyway. Dropped here regardless: a signed-out
    // process has no reason to still hold what a tenant enabled for someone.
    app.state::<AppRuntimeState>().model_catalog().clear();
    // Cached plan sessions and in-flight plan sign-ins belong to the account that is leaving.
    app.state::<AppRuntimeState>()
        .plan_providers()
        .forget_account();
    // A grant names a path the account chose. It does not outlive the account.
    if let Some(grants) = app.try_state::<Arc<PathGrants>>() {
        grants.clear();
    }
}

/// The code for a refusal. Exhaustive by construction: a new service error
/// has to be given a surface here before it compiles.
fn login_error_code(error: &AuthServiceError) -> AuthLoginErrorCode {
    match error {
        AuthServiceError::AlreadyAuthenticating => AuthLoginErrorCode::AlreadyAuthenticating,
        AuthServiceError::NotConfigured(_) => AuthLoginErrorCode::NotConfigured,
        AuthServiceError::BrowserUnavailable => AuthLoginErrorCode::BrowserUnavailable,
        AuthServiceError::DiscoveryFailed => AuthLoginErrorCode::DiscoveryFailed,
        AuthServiceError::IssuerMismatch => AuthLoginErrorCode::IssuerMismatch,
        AuthServiceError::DiscoveryMetadataInvalid => AuthLoginErrorCode::DiscoveryMetadataInvalid,
        AuthServiceError::LoopbackUnavailable(_) => AuthLoginErrorCode::LoopbackUnavailable,
        AuthServiceError::CallbackInvalid => AuthLoginErrorCode::CallbackInvalid,
        AuthServiceError::CallbackTimedOut => AuthLoginErrorCode::CallbackTimedOut,
        AuthServiceError::RedirectNotRegistered(_) => AuthLoginErrorCode::RedirectNotRegistered,
        AuthServiceError::TokenExchangeFailed => AuthLoginErrorCode::TokenExchangeFailed,
        AuthServiceError::IdentityFailed => AuthLoginErrorCode::IdentityFailed,
        AuthServiceError::DeviceClockSkew => AuthLoginErrorCode::DeviceClockSkew,
        AuthServiceError::Cancelled => AuthLoginErrorCode::Cancelled,
    }
}

/// The wire name of a refusal code (`snake_case`), the stable label the audit
/// log stores. Never the service's own message text.
fn login_code_label(code: &AuthLoginErrorCode) -> String {
    serde_json::to_value(code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

/// Attributes later audit events to the signed-in user. Ids only: the actor is
/// the user id, never the email or display name.
fn bind_audit_identity(app: &tauri::AppHandle, session: &SessionProjection) {
    let (Some(hub), SessionProjection::Authenticated { identity, .. }) =
        (app.try_state::<Arc<AuditHub>>(), session)
    else {
        return;
    };
    hub.set_identity(&identity.user_id, &identity.tenant_id);
}

fn bind_account_scope(state: &AppRuntimeState, session: &SessionProjection) -> Result<(), ()> {
    let SessionProjection::Authenticated { identity, .. } = session else {
        return Err(());
    };
    let account_id = canonical_account_id(identity).map_err(|_| ())?;
    state.accounts().bind_native(&account_id).map_err(|_| ())
}
