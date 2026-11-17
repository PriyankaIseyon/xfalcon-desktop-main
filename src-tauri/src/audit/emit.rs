//! Fire-and-forget audit helpers for command handlers. They look the hub up
//! in managed state, so a command never needs the hub passed to it, and they
//! do nothing when auditing is off for the session.

use std::sync::Arc;

use tauri::Manager;

use super::{AuditAction, AuditHub, AuditOutcome, AuditResourceType};

/// Policy id for application-level events, which no tool or project policy
/// decided.
const APP_POLICY_ID: &str = "desktop.app.v1";
const APP_REQUEST_ID: &str = "app";

pub(crate) fn record<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    action: AuditAction,
    resource_type: AuditResourceType,
    resource_id: &str,
    outcome: AuditOutcome,
) {
    if let Some(hub) = app.try_state::<Arc<AuditHub>>() {
        hub.record(
            action,
            resource_type,
            resource_id,
            outcome,
            APP_POLICY_ID,
            APP_REQUEST_ID,
        );
    }
}

pub(crate) fn record_with_detail<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    action: AuditAction,
    resource_type: AuditResourceType,
    resource_id: &str,
    outcome: AuditOutcome,
    detail: &str,
) {
    if let Some(hub) = app.try_state::<Arc<AuditHub>>() {
        hub.record_with_detail(
            action,
            resource_type,
            resource_id,
            outcome,
            APP_POLICY_ID,
            APP_REQUEST_ID,
            detail,
        );
    }
}

/// The outcome of an attempted action whose result is known.
pub(crate) fn outcome_for<T, E>(result: &Result<T, E>) -> AuditOutcome {
    if result.is_ok() {
        AuditOutcome::Success
    } else {
        AuditOutcome::Failure
    }
}

/// Opens a hub over a log under `root` and manages it on `app`, for tests that
/// check what a command records.
#[cfg(test)]
pub(crate) fn test_hub<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    root: &std::path::Path,
) -> Arc<AuditHub> {
    let secrets = Arc::new(crate::persistence::InMemorySecretStore::default());
    let store = super::LocalAuditStore::open(root.join("audit"), secrets).expect("open audit log");
    let hub = Arc::new(AuditHub::new(Arc::new(store)));
    app.manage(hub.clone());
    hub
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tauri::Manager;
    use tempfile::tempdir;

    use super::*;
    use crate::audit::LocalAuditStore;
    use crate::persistence::InMemorySecretStore;

    fn managed_hub(root: &std::path::Path) -> Arc<AuditHub> {
        let secrets = Arc::new(InMemorySecretStore::default());
        let store = LocalAuditStore::open(root.join("audit"), secrets).expect("open audit log");
        Arc::new(AuditHub::new(Arc::new(store)))
    }

    #[test]
    fn record_writes_an_app_event_through_the_managed_hub() {
        let root = tempdir().expect("temp dir");
        let hub = managed_hub(root.path());
        let app = tauri::test::mock_app();
        app.manage(hub.clone());

        record(
            app.handle(),
            AuditAction::DiagnosticsExport,
            AuditResourceType::Diagnostics,
            "report",
            AuditOutcome::Success,
        );

        let records = hub.store().recent(10).expect("read audit log");
        assert_eq!(records.len(), 1);
        let event = records[0].event();
        assert_eq!(event.action(), AuditAction::DiagnosticsExport);
        assert_eq!(event.resource_id(), "report");
        assert_eq!(event.policy_id(), APP_POLICY_ID);
        assert_eq!(event.request_id(), APP_REQUEST_ID);
        assert_eq!(event.detail(), None);
    }

    #[test]
    fn record_with_detail_stores_the_detail() {
        let root = tempdir().expect("temp dir");
        let hub = managed_hub(root.path());
        let app = tauri::test::mock_app();
        app.manage(hub.clone());

        record_with_detail(
            app.handle(),
            AuditAction::AuthLoginFailed,
            AuditResourceType::Session,
            "callback_invalid",
            AuditOutcome::Failure,
            "loopback",
        );

        let records = hub.store().recent(10).expect("read audit log");
        assert_eq!(records[0].event().detail(), Some("loopback"));
    }

    #[test]
    fn record_is_a_silent_no_op_without_a_managed_hub() {
        let app = tauri::test::mock_app();

        record(
            app.handle(),
            AuditAction::AuthLogout,
            AuditResourceType::Session,
            "session",
            AuditOutcome::Success,
        );

        assert!(app.try_state::<Arc<AuditHub>>().is_none());
    }
}
