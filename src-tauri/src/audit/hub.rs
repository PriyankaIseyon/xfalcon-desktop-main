//! Late-bound identity hub in front of the local audit log.
//!
//! Publishers are created once, usually before a sign-in exists. The hub
//! holds the live actor and tenant, so a publisher read at publish time picks
//! up whatever identity is current. Until one is set, events are attributed
//! to `unauthenticated` / `unconfigured`.
//!
//! [`AuditHub::record`] never fails and never panics. A failed append is
//! counted in [`AuditHub::dropped_events`] and logged by kind only, so a lost
//! security event stays visible in the diagnostics section.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use chrono::Utc;
use uuid::Uuid;

use super::publisher::sanitize_identifier;
use super::{
    AuditAction, AuditEvent, AuditOutcome, AuditPublisher, AuditResourceType, LocalAuditStore,
};

const UNAUTHENTICATED_ACTOR: &str = "unauthenticated";
const UNCONFIGURED_TENANT: &str = "unconfigured";

#[derive(Default)]
struct Identity {
    actor: Option<String>,
    tenant: Option<String>,
}

pub struct AuditHub {
    store: Arc<LocalAuditStore>,
    identity: RwLock<Identity>,
    sequence: AtomicU64,
    dropped: AtomicU64,
}

impl AuditHub {
    #[must_use]
    pub fn new(store: Arc<LocalAuditStore>) -> Self {
        Self {
            store,
            identity: RwLock::new(Identity::default()),
            sequence: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    pub fn set_identity(&self, actor: &str, tenant: &str) {
        let mut identity = self
            .identity
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        identity.actor = Some(sanitize_identifier(actor));
        identity.tenant = Some(sanitize_identifier(tenant));
    }

    pub fn clear_identity(&self) {
        let mut identity = self
            .identity
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        *identity = Identity::default();
    }

    /// Records one content-free event under the current identity. Never
    /// returns an error and never panics.
    pub fn record(
        &self,
        action: AuditAction,
        resource_type: AuditResourceType,
        resource_id: &str,
        outcome: AuditOutcome,
        policy_id: &str,
        request_id: &str,
    ) {
        self.append_event(
            action,
            resource_type,
            resource_id,
            outcome,
            policy_id,
            request_id,
            None,
        );
    }

    /// Like [`Self::record`], with one extra content-free identifier (such as
    /// a tool id) stored as the event's detail.
    #[allow(clippy::too_many_arguments)]
    pub fn record_with_detail(
        &self,
        action: AuditAction,
        resource_type: AuditResourceType,
        resource_id: &str,
        outcome: AuditOutcome,
        policy_id: &str,
        request_id: &str,
        detail: &str,
    ) {
        self.append_event(
            action,
            resource_type,
            resource_id,
            outcome,
            policy_id,
            request_id,
            Some(sanitize_identifier(detail)),
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn append_event(
        &self,
        action: AuditAction,
        resource_type: AuditResourceType,
        resource_id: &str,
        outcome: AuditOutcome,
        policy_id: &str,
        request_id: &str,
        detail: Option<String>,
    ) {
        let (actor, tenant) = {
            let identity = self.identity.read().unwrap_or_else(PoisonError::into_inner);
            (
                identity
                    .actor
                    .clone()
                    .unwrap_or_else(|| UNAUTHENTICATED_ACTOR.to_owned()),
                identity
                    .tenant
                    .clone()
                    .unwrap_or_else(|| UNCONFIGURED_TENANT.to_owned()),
            )
        };
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let event = AuditEvent::new(
            Uuid::new_v4(),
            sequence,
            actor,
            tenant,
            self.store.device_id(),
            action,
            resource_type,
            sanitize_identifier(resource_id),
            outcome,
            sanitize_identifier(policy_id),
            sanitize_identifier(request_id),
            Utc::now().timestamp_millis(),
        )
        .and_then(|event| match detail {
            Some(detail) => event.with_detail(detail),
            None => Ok(event),
        })
        .and_then(|event| self.store.append(event));
        if let Err(error) = event {
            self.dropped.fetch_add(1, Ordering::SeqCst);
            tracing::warn!(kind = error.code(), "security audit event was not recorded");
        }
    }

    /// The local log this hub appends to, for reading (diagnostics).
    #[must_use]
    pub fn store(&self) -> &Arc<LocalAuditStore> {
        &self.store
    }

    /// Number of events that could not be appended since this hub was created.
    #[must_use]
    pub fn dropped_events(&self) -> u64 {
        self.dropped.load(Ordering::SeqCst)
    }

    /// A publisher whose events are recorded through this hub. Clones share
    /// the same chain.
    #[must_use]
    pub fn publisher(self: &Arc<Self>) -> AuditPublisher {
        AuditPublisher::bound(Arc::clone(self))
    }
}
