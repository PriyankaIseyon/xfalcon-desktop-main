//! Shared best-effort "publish, or spool on failure" helper.
//!
//! Every real [`AuditSink`]/[`AuditQueue`] caller in this codebase
//! (`crate::agent::tool_loop::ToolLoop`'s `ToolLoopAudit`,
//! `crate::projects::trust::ProjectsService`,
//! `crate::approvals::service::ApprovalsService`,
//! `crate::projects::read::read_file_with_audit`) goes through one
//! [`AuditPublisher`] rather than re-implementing this logic per call site.
//! Never blocks or reverses the caller's own already-decided outcome —
//! mirrors `crate::approvals::service::ApprovalsService::sync_da`'s
//! established "best-effort side channel, must never block or reverse a
//! decision" precedent.
//!
//! # Never silently discard a security-relevant event
//!
//! [`AuditEvent::new`] validates every identifier field (ASCII
//! alphanumeric/`-`/`_`/`.`/`:`, non-empty, bounded length —
//! `super::event::validate_identifier`). A caller-supplied `resource_id`
//! (e.g. an `InvocationId` that, once a live model/provider-controlled tool
//! call ships, is no longer exclusively Rust-generated — see
//! `crate::agent::executor`'s module doc comment) can violate that charset.
//! Rather than let one out-of-charset identifier silently drop the whole
//! audit event (no publish, no spool, no signal), [`Self::publish`]
//! sanitizes every identifier first (replacing any disallowed byte with
//! `_`, bounding length) so construction succeeds in practice, and still
//! reports the rare residual failure (e.g. every event's identity fields
//! being empty after sanitization) rather than swallowing it — this
//! codebase has no `tracing`/`log` subscriber wired anywhere yet (verified:
//! neither crate is a direct dependency, and nothing calls
//! `tracing_subscriber::fmt::init`/`env_logger::init`/`log::set_logger`), so
//! emitting through either would be a silent no-op in practice; `eprintln!`
//! (already this codebase's own precedent —
//! `crate::security::panic_boundary::install_redacted_panic_hook`) is the
//! only channel guaranteed to actually surface today.
//!
//! # Sequence numbering
//!
//! [`AuditQueue::events_after`] is an ordered, gap-detecting cursor replay
//! over one account's local retry queue. Seeding a fresh sequence counter at
//! a fixed `0` for every new [`AuditPublisher`] instance (one per run/
//! service call in practice) would let a *later* instance's low-numbered
//! event collide with (and be skipped by a cursor already past) an
//! *earlier* instance's own higher-numbered one — the queue only retains
//! *unacknowledged* (i.e. still-failed) events, so there is no durable
//! "last sequence used" watermark to reconstruct once older events have
//! published successfully and left the queue. Seeding from wall-clock
//! nanoseconds instead (monotonically time-ordered across process
//! restarts, and fine-grained enough that two instances constructed
//! "simultaneously" in human terms still receive distinct seeds in
//! practice) closes the *practical* collision risk without inventing a new
//! persisted counter/schema migration in this fix — a fully gap-free,
//! durable, atomically-incrementing counter remains a legitimate, separate
//! follow-up if this is ever wired to genuinely concurrent multi-process
//! callers.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use uuid::Uuid;

use super::hub::AuditHub;
use super::{
    AuditAction, AuditEvent, AuditOutcome, AuditPublishStatus, AuditQueue, AuditResourceType,
    AuditSink,
};

/// Mirrors `super::event::validate_identifier`'s own bound exactly (kept as
/// a duplicated literal, not imported: that constant is private to
/// `super::event`, and this module's sanitizer only needs to agree on the
/// bound, not share the validator itself).
const MAX_IDENTIFIER_BYTES: usize = 160;

/// Cheap to clone: both modes share their state behind an `Arc`, so clones
/// publish into the same sequence / the same hub chain.
#[derive(Clone)]
pub struct AuditPublisher {
    backend: Backend,
}

#[derive(Clone)]
enum Backend {
    /// Publishes to the DA sink and spools failures into the encrypted queue.
    Spool(Arc<SpoolPublisher>),
    /// Appends to the local tamper-evident log through the hub, which owns
    /// the live identity and the sequence.
    Hub(Arc<AuditHub>),
}

struct SpoolPublisher {
    actor_id: String,
    tenant_id: String,
    device_id: String,
    sink: Arc<dyn AuditSink>,
    queue: Arc<AuditQueue>,
    sequence: AtomicU64,
}

impl AuditPublisher {
    #[must_use]
    pub fn new(
        actor_id: impl Into<String>,
        tenant_id: impl Into<String>,
        device_id: impl Into<String>,
        sink: Arc<dyn AuditSink>,
        queue: Arc<AuditQueue>,
    ) -> Self {
        Self {
            backend: Backend::Spool(Arc::new(SpoolPublisher {
                actor_id: actor_id.into(),
                tenant_id: tenant_id.into(),
                device_id: device_id.into(),
                sink,
                queue,
                sequence: AtomicU64::new(initial_audit_sequence()),
            })),
        }
    }

    /// A publisher whose events go to the local hub's chain. Identity is read
    /// from the hub at publish time, not at construction.
    pub(super) fn bound(hub: Arc<AuditHub>) -> Self {
        Self {
            backend: Backend::Hub(hub),
        }
    }

    /// Publishes one content-free audit event, best-effort. Never returns an
    /// error and never influences the caller's own already-decided outcome:
    /// a non-published status (or a transport error) spools the event into
    /// the bounded encrypted [`AuditQueue`] for later retry; a permanent
    /// [`AuditPublishStatus::Rejected`] is logged, not re-queued forever
    /// (there is nothing a blind retry could fix).
    pub async fn publish(
        &self,
        action: AuditAction,
        resource_type: AuditResourceType,
        resource_id: &str,
        outcome: AuditOutcome,
        policy_id: &str,
        request_id: &str,
    ) {
        match &self.backend {
            Backend::Spool(spool) => {
                spool
                    .publish(
                        action,
                        resource_type,
                        resource_id,
                        outcome,
                        policy_id,
                        request_id,
                    )
                    .await;
            }
            Backend::Hub(hub) => hub.record(
                action,
                resource_type,
                resource_id,
                outcome,
                policy_id,
                request_id,
            ),
        }
    }

    /// Like [`Self::publish`], with one extra content-free identifier. The
    /// detail is stored by the local hub. The legacy DA spool path has no
    /// detail field, so there it is dropped and the call behaves exactly as
    /// [`Self::publish`].
    #[allow(clippy::too_many_arguments)]
    pub async fn publish_with_detail(
        &self,
        action: AuditAction,
        resource_type: AuditResourceType,
        resource_id: &str,
        outcome: AuditOutcome,
        policy_id: &str,
        request_id: &str,
        detail: &str,
    ) {
        match &self.backend {
            Backend::Spool(spool) => {
                spool
                    .publish(
                        action,
                        resource_type,
                        resource_id,
                        outcome,
                        policy_id,
                        request_id,
                    )
                    .await;
            }
            Backend::Hub(hub) => hub.record_with_detail(
                action,
                resource_type,
                resource_id,
                outcome,
                policy_id,
                request_id,
                detail,
            ),
        }
    }
}

impl SpoolPublisher {
    async fn publish(
        &self,
        action: AuditAction,
        resource_type: AuditResourceType,
        resource_id: &str,
        outcome: AuditOutcome,
        policy_id: &str,
        request_id: &str,
    ) {
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let event = AuditEvent::new(
            Uuid::new_v4(),
            sequence,
            sanitize_identifier(&self.actor_id),
            sanitize_identifier(&self.tenant_id),
            sanitize_identifier(&self.device_id),
            action,
            resource_type,
            sanitize_identifier(resource_id),
            outcome,
            sanitize_identifier(policy_id),
            sanitize_identifier(request_id),
            chrono::Utc::now().timestamp_millis(),
        );
        let Ok(event) = event else {
            // Only reachable if sanitization itself somehow still leaves an
            // invalid field (e.g. every identifier is empty) — genuinely
            // exceptional, but still surfaced rather than silently dropped.
            eprintln!(
                "audit event construction failed even after identifier sanitization \
                 (action={action:?}, resource_type={resource_type:?}); dropping this \
                 audit event"
            );
            return;
        };
        match self.sink.publish(&event).await {
            Ok(AuditPublishStatus::Published | AuditPublishStatus::InheritedControl) => {}
            Ok(AuditPublishStatus::Retryable) | Err(_) => {
                let _ = self.queue.enqueue(&event);
            }
            Ok(AuditPublishStatus::Rejected) => {
                // A permanent rejection is not recoverable by blind retry —
                // requeuing it forever would just churn the bounded queue
                // with an entry that can never succeed. Surface it instead.
                eprintln!(
                    "audit event permanently rejected by sink (action={action:?}, \
                     resource_type={resource_type:?}, resource_id sanitized); not re-queued"
                );
            }
        }
    }
}

/// Replaces any byte outside `super::event::validate_identifier`'s allowed
/// charset (ASCII alphanumeric, `-`, `_`, `.`, `:`) with `_`, and bounds the
/// result to [`MAX_IDENTIFIER_BYTES`]. An all-invalid input (e.g. empty)
/// becomes a fixed, valid placeholder rather than an empty string (which
/// `validate_identifier` itself rejects).
pub(super) fn sanitize_identifier(raw: &str) -> String {
    // Redact sensitive shapes before replacing punctuation; normalization can
    // otherwise leave an email, path, or credential legible to an auditor.
    let redacted = crate::diagnostics::sanitize_diagnostics_text(raw);
    let mut sanitized: String = redacted
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | ':') {
                character
            } else {
                '_'
            }
        })
        .take(MAX_IDENTIFIER_BYTES)
        .collect();
    if sanitized.is_empty() {
        sanitized.push_str("unrepresentable");
    }
    sanitized
}

/// See the module doc comment's "Sequence numbering" section.
fn initial_audit_sequence() -> u64 {
    u64::try_from(chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::error::AppResult;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use tempfile::tempdir;

    fn queue() -> Arc<AuditQueue> {
        let directory = tempdir().expect("temp dir");
        let secrets = Arc::new(crate::persistence::InMemorySecretStore::default());
        secrets.put_key("account-1", [0x42; 32]).expect("store key");
        let journal = Arc::new(
            crate::persistence::Journal::open(directory.path().join("audit.sqlite3"), secrets)
                .expect("open journal"),
        );
        Arc::new(AuditQueue::new("account-1", 16, journal).expect("queue"))
    }

    struct RecordingSink {
        status: AuditPublishStatus,
        published: Mutex<Vec<AuditEvent>>,
    }

    #[async_trait]
    impl AuditSink for RecordingSink {
        async fn publish(&self, event: &AuditEvent) -> AppResult<AuditPublishStatus> {
            self.published.lock().expect("lock").push(event.clone());
            Ok(self.status)
        }
    }

    #[tokio::test]
    async fn an_out_of_charset_resource_id_is_sanitized_not_dropped() {
        let sink = Arc::new(RecordingSink {
            status: AuditPublishStatus::Published,
            published: Mutex::new(Vec::new()),
        });
        let publisher = AuditPublisher::new(
            "actor@example.com",
            "tenant-1",
            "device-1",
            sink.clone(),
            queue(),
        );

        publisher
            .publish(
                AuditAction::ToolExecute,
                AuditResourceType::ToolCall,
                "inv/with spaces\nand\tcontrol-chars",
                AuditOutcome::Success,
                "policy-1",
                "run-1",
            )
            .await;

        let published = sink.published.lock().expect("lock");
        assert_eq!(published.len(), 1, "sanitized event must still publish");
        assert_eq!(published[0].actor_id(), "_REDACTED_EMAIL_");
        assert!(!published[0].actor_id().contains("actor_example.com"));
    }

    #[test]
    fn identifier_sanitization_preserves_safe_opaque_ids() {
        for identifier in [
            "550e8400-e29b-41d4-a716-446655440000",
            "openai:gpt-4.1",
            "provider/model.name",
        ] {
            let expected = identifier.replace('/', "_");
            assert_eq!(sanitize_identifier(identifier), expected);
        }
    }

    #[tokio::test]
    async fn a_permanent_rejection_is_not_requeued() {
        let sink = Arc::new(RecordingSink {
            status: AuditPublishStatus::Rejected,
            published: Mutex::new(Vec::new()),
        });
        let queue = queue();
        let publisher = AuditPublisher::new(
            "actor-1",
            "tenant-1",
            "device-1",
            sink.clone(),
            queue.clone(),
        );

        publisher
            .publish(
                AuditAction::ToolExecute,
                AuditResourceType::ToolCall,
                "resource-1",
                AuditOutcome::Success,
                "policy-1",
                "run-1",
            )
            .await;

        assert!(
            queue.events_after(0).expect("replay").is_empty(),
            "a permanent rejection must never be spooled for retry"
        );
    }

    #[tokio::test]
    async fn a_retryable_status_is_spooled_into_the_queue() {
        let sink = Arc::new(RecordingSink {
            status: AuditPublishStatus::Retryable,
            published: Mutex::new(Vec::new()),
        });
        let queue = queue();
        let publisher = AuditPublisher::new(
            "actor-1",
            "tenant-1",
            "device-1",
            sink.clone(),
            queue.clone(),
        );

        publisher
            .publish(
                AuditAction::ToolExecute,
                AuditResourceType::ToolCall,
                "resource-1",
                AuditOutcome::Success,
                "policy-1",
                "run-1",
            )
            .await;

        assert_eq!(queue.events_after(0).expect("replay").len(), 1);
    }

    #[tokio::test]
    async fn two_publishers_constructed_moments_apart_never_reuse_a_sequence() {
        let sink = Arc::new(RecordingSink {
            status: AuditPublishStatus::Published,
            published: Mutex::new(Vec::new()),
        });
        let first = AuditPublisher::new("actor-1", "tenant-1", "device-1", sink.clone(), queue());
        // A real, if small, sleep rather than back-to-back construction:
        // guards this assertion against a coarse-grained wall clock (some
        // platforms' `SystemTime` ticks no finer than ~15ms) making two
        // in-process constructions read the identical nanosecond value,
        // which would be a false failure of this test, not of the
        // production seeding logic itself.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = AuditPublisher::new("actor-1", "tenant-1", "device-1", sink.clone(), queue());

        first
            .publish(
                AuditAction::ToolExecute,
                AuditResourceType::ToolCall,
                "resource-1",
                AuditOutcome::Success,
                "policy-1",
                "run-1",
            )
            .await;
        second
            .publish(
                AuditAction::ToolExecute,
                AuditResourceType::ToolCall,
                "resource-2",
                AuditOutcome::Success,
                "policy-1",
                "run-2",
            )
            .await;

        let published = sink.published.lock().expect("lock");
        assert_ne!(
            published[0].sequence(),
            published[1].sequence(),
            "two independently-constructed publishers must never hand out the same sequence"
        );
    }
}
