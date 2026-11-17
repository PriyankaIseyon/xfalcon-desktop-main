use async_trait::async_trait;

use crate::domain::error::AppResult;

use super::AuditEvent;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DaAuditResponse {
    Created,
    Unauthorized,
    Forbidden,
    Retryable,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditPublishStatus {
    Published,
    InheritedControl,
    Retryable,
    Rejected,
}

#[async_trait]
pub trait DaAuditTransport: Send + Sync {
    async fn create_audit_event(&self, event: &AuditEvent) -> AppResult<DaAuditResponse>;
}

#[async_trait]
pub trait AuditSink: Send + Sync {
    async fn publish(&self, event: &AuditEvent) -> AppResult<AuditPublishStatus>;
}

pub struct DaAuditSink<T> {
    transport: T,
}

impl<T> DaAuditSink<T> {
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl<T> AuditSink for DaAuditSink<T>
where
    T: DaAuditTransport,
{
    async fn publish(&self, event: &AuditEvent) -> AppResult<AuditPublishStatus> {
        event.validate()?;
        Ok(match self.transport.create_audit_event(event).await? {
            DaAuditResponse::Created => AuditPublishStatus::Published,
            DaAuditResponse::Unauthorized | DaAuditResponse::Forbidden => {
                AuditPublishStatus::InheritedControl
            }
            DaAuditResponse::Retryable => AuditPublishStatus::Retryable,
            DaAuditResponse::Rejected => AuditPublishStatus::Rejected,
        })
    }
}
