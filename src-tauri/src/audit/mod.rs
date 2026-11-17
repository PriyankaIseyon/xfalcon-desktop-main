use std::path::Path;
use std::sync::Arc;

use crate::persistence::SecretStore;

mod da_sink;
mod diagnostics;
pub(crate) mod emit;
mod event;
mod hub;
mod local;
mod publisher;
mod queue;

pub use da_sink::{AuditPublishStatus, AuditSink, DaAuditResponse, DaAuditSink, DaAuditTransport};
pub use diagnostics::render_diagnostics_section;
pub use event::{AuditAction, AuditEvent, AuditOutcome, AuditResourceType};
pub use hub::AuditHub;
pub use local::{AuditRecord, LocalAuditStore, VerificationReport};
pub use publisher::AuditPublisher;
pub use queue::AuditQueue;

/// Opens the local security audit log under `app_local_data/audit`. `None`
/// when it cannot be opened (blocked folder, unavailable key store): auditing
/// is then off for the session, and the error is logged. Never fails startup.
pub(crate) fn open_hub(
    app_local_data: &Path,
    secrets: Arc<dyn SecretStore>,
) -> Option<Arc<AuditHub>> {
    match LocalAuditStore::open_or_quarantine(app_local_data.join("audit"), secrets) {
        Ok(store) => Some(Arc::new(AuditHub::new(Arc::new(store)))),
        Err(error) => {
            tracing::error!(
                kind = error.code(),
                "security audit log could not be opened; auditing is disabled for this session"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_local;
#[cfg(test)]
mod tests_open_hub;
