use std::sync::Arc;

use crate::{
    domain::error::{AppError, AppResult},
    persistence::{AppendOutcome, Journal, NewJournalEvent},
};

use super::AuditEvent;

const AUDIT_RUN_ID: &str = "__audit_queue__";
const AUDIT_EVENT_TYPE: &str = "security_audit";
const MAXIMUM_QUEUE_CAPACITY: usize = 4_096;

pub struct AuditQueue {
    account_id: String,
    maximum_events: usize,
    journal: Arc<Journal>,
}

impl AuditQueue {
    pub fn new(
        account_id: impl Into<String>,
        maximum_events: usize,
        journal: Arc<Journal>,
    ) -> AppResult<Self> {
        let account_id = account_id.into();
        if account_id.trim().is_empty() {
            return Err(AppError::PersistenceInvalid("account_id is required"));
        }
        if maximum_events == 0 || maximum_events > MAXIMUM_QUEUE_CAPACITY {
            return Err(AppError::PersistenceInvalid(
                "maximum_events must be between 1 and 4096",
            ));
        }
        Ok(Self {
            account_id,
            maximum_events,
            journal,
        })
    }

    pub fn enqueue(&self, event: &AuditEvent) -> AppResult<AppendOutcome> {
        event.validate()?;
        let payload =
            serde_json::to_vec(event).map_err(|_| AppError::PersistenceEncryptionFailed)?;
        self.journal.append_bounded(
            NewJournalEvent {
                account_id: self.account_id.clone(),
                event_id: event.event_id().to_string(),
                run_id: AUDIT_RUN_ID.to_owned(),
                sequence: event.sequence(),
                event_type: AUDIT_EVENT_TYPE.to_owned(),
                state: "queued".to_owned(),
                payload,
                created_at_ms: event.occurred_at_ms(),
            },
            self.maximum_events,
        )
    }

    pub fn events_after(&self, sequence: u64) -> AppResult<Vec<AuditEvent>> {
        let mut cursor = sequence;
        let mut audit_events = Vec::new();
        loop {
            let page = self
                .journal
                .events_after(&self.account_id, AUDIT_RUN_ID, cursor)?;
            let next_sequence = page.next_sequence();
            let decoded = page.into_events().into_iter().map(|event| {
                let event: AuditEvent = serde_json::from_slice(&event.payload)
                    .map_err(|_| AppError::PersistenceDecryptionFailed)?;
                event.validate()?;
                Ok(event)
            });
            audit_events.extend(decoded.collect::<AppResult<Vec<_>>>()?);
            match next_sequence {
                Some(sequence) => cursor = sequence,
                None => return Ok(audit_events),
            }
        }
    }

    pub fn acknowledge(&self, event: &AuditEvent) -> AppResult<bool> {
        event.validate()?;
        Ok(self.journal.delete_event(
            &self.account_id,
            AUDIT_RUN_ID,
            &event.event_id().to_string(),
        )? == 1)
    }

    pub fn purge(&self) -> AppResult<usize> {
        self.journal.purge_run(&self.account_id, AUDIT_RUN_ID)
    }
}
