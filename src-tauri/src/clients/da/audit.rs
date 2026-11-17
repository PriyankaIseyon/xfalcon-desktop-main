use async_trait::async_trait;
use indexmap::IndexMap;
use serde_json::Value;

use crate::{
    audit::{AuditEvent, AuditOutcome, DaAuditResponse, DaAuditTransport},
    domain::error::{AppError, AppResult},
};

use super::{
    auth::AuthenticatedDaClient,
    generated::{
        AuditEventCreate, CreateAuditEventApiV1ChatAuditEventsPostRequest,
        CreateAuditEventApiV1ChatAuditEventsPostResponse,
    },
    mapping::DaRoute,
};

pub(crate) struct GeneratedDaAuditTransport {
    client: AuthenticatedDaClient,
}

impl GeneratedDaAuditTransport {
    pub(crate) const fn new(client: AuthenticatedDaClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl DaAuditTransport for GeneratedDaAuditTransport {
    async fn create_audit_event(&self, event: &AuditEvent) -> AppResult<DaAuditResponse> {
        if !self
            .client
            .matches_principal(event.actor_id(), event.tenant_id())
        {
            return Err(AppError::AuditIdentityMismatch);
        }
        let request = CreateAuditEventApiV1ChatAuditEventsPostRequest {
            body: AuditEventCreate {
                action: event.action().as_str().to_owned(),
                outcome: outcome_name(event.outcome()).to_owned(),
                conversation_id: None,
                run_id: None,
                server_id: None,
                tool_call_reference: None,
                resource_type: Some(event.resource_type().as_str().to_owned()),
                resource_id: Some(event.resource_id().to_owned()),
                details: Some(IndexMap::from([
                    (
                        "device_id".to_owned(),
                        Value::String(event.device_id().to_owned()),
                    ),
                    (
                        "event_id".to_owned(),
                        Value::String(event.event_id().to_string()),
                    ),
                    (
                        "policy_id".to_owned(),
                        Value::String(event.policy_id().to_owned()),
                    ),
                    (
                        "request_id".to_owned(),
                        Value::String(event.request_id().to_owned()),
                    ),
                    ("sequence".to_owned(), Value::from(event.sequence())),
                    (
                        "occurred_at_ms".to_owned(),
                        Value::from(event.occurred_at_ms()),
                    ),
                ])),
                actor_user_id: None,
            },
        };
        let response = self
            .client
            .post(DaRoute::AuditEvents)
            .map_err(|_| AppError::AuditTransportFailed)?
            .json(&request.body)
            .send()
            .await
            .map_err(|_| AppError::AuditTransportFailed)?;
        let response = CreateAuditEventApiV1ChatAuditEventsPostRequest::parse_response(response)
            .await
            .map_err(|_| AppError::AuditContractFailed)?;

        Ok(match response {
            CreateAuditEventApiV1ChatAuditEventsPostResponse::Created(_) => {
                DaAuditResponse::Created
            }
            CreateAuditEventApiV1ChatAuditEventsPostResponse::Unauthorized(_) => {
                DaAuditResponse::Unauthorized
            }
            CreateAuditEventApiV1ChatAuditEventsPostResponse::Forbidden(_) => {
                DaAuditResponse::Forbidden
            }
            CreateAuditEventApiV1ChatAuditEventsPostResponse::TooManyRequests(_)
            | CreateAuditEventApiV1ChatAuditEventsPostResponse::ServiceUnavailable(_) => {
                DaAuditResponse::Retryable
            }
            CreateAuditEventApiV1ChatAuditEventsPostResponse::NotFound(_)
            | CreateAuditEventApiV1ChatAuditEventsPostResponse::Conflict(_)
            | CreateAuditEventApiV1ChatAuditEventsPostResponse::ContentTooLarge(_)
            | CreateAuditEventApiV1ChatAuditEventsPostResponse::UnprocessableEntity(_)
            | CreateAuditEventApiV1ChatAuditEventsPostResponse::Unknown => {
                DaAuditResponse::Rejected
            }
        })
    }
}

const fn outcome_name(outcome: AuditOutcome) -> &'static str {
    match outcome {
        AuditOutcome::Allowed => "allowed",
        AuditOutcome::ApprovalRequired => "approval_required",
        AuditOutcome::Approved => "approved",
        AuditOutcome::Rejected => "rejected",
        AuditOutcome::Denied => "denied",
        AuditOutcome::Updated => "updated",
        AuditOutcome::Success => "success",
        AuditOutcome::Failure => "failure",
    }
}
