use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::error::{AppError, AppResult};

const MAX_IDENTIFIER_BYTES: usize = 160;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AuditAction {
    #[serde(rename = "auth.login")]
    AuthLogin,
    #[serde(rename = "auth.logout")]
    AuthLogout,
    #[serde(rename = "credential.create")]
    CredentialCreate,
    #[serde(rename = "credential.replace")]
    CredentialReplace,
    #[serde(rename = "credential.delete")]
    CredentialDelete,
    #[serde(rename = "credential.use")]
    CredentialUse,
    #[serde(rename = "project.trust")]
    ProjectTrust,
    #[serde(rename = "project.revoke")]
    ProjectRevoke,
    #[serde(rename = "project.read")]
    ProjectRead,
    #[serde(rename = "project.write")]
    ProjectWrite,
    #[serde(rename = "tool.approve")]
    ToolApprove,
    #[serde(rename = "tool.deny")]
    ToolDeny,
    #[serde(rename = "tool.execute")]
    ToolExecute,
    #[serde(rename = "mcp.connect")]
    McpConnect,
    #[serde(rename = "mcp.disconnect")]
    McpDisconnect,
    #[serde(rename = "mcp.invoke")]
    McpInvoke,
    #[serde(rename = "entitlement.evaluate")]
    EntitlementEvaluate,
    #[serde(rename = "update.install")]
    UpdateInstall,
    #[serde(rename = "auth.login_failed")]
    AuthLoginFailed,
    #[serde(rename = "mcp.server_create")]
    McpServerCreate,
    #[serde(rename = "mcp.server_update")]
    McpServerUpdate,
    #[serde(rename = "mcp.server_delete")]
    McpServerDelete,
    #[serde(rename = "mcp.sign_in")]
    McpSignIn,
    #[serde(rename = "mcp.sign_out")]
    McpSignOut,
    #[serde(rename = "skill.import")]
    SkillImport,
    #[serde(rename = "skill.delete")]
    SkillDelete,
    #[serde(rename = "data.export")]
    DataExport,
    #[serde(rename = "data.erase")]
    DataErase,
    #[serde(rename = "diagnostics.export")]
    DiagnosticsExport,
}

impl AuditAction {
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "auth.login" => Ok(Self::AuthLogin),
            "auth.logout" => Ok(Self::AuthLogout),
            "credential.create" => Ok(Self::CredentialCreate),
            "credential.replace" => Ok(Self::CredentialReplace),
            "credential.delete" => Ok(Self::CredentialDelete),
            "credential.use" => Ok(Self::CredentialUse),
            "project.trust" => Ok(Self::ProjectTrust),
            "project.revoke" => Ok(Self::ProjectRevoke),
            "project.read" => Ok(Self::ProjectRead),
            "project.write" => Ok(Self::ProjectWrite),
            "tool.approve" => Ok(Self::ToolApprove),
            "tool.deny" => Ok(Self::ToolDeny),
            "tool.execute" => Ok(Self::ToolExecute),
            "mcp.connect" => Ok(Self::McpConnect),
            "mcp.disconnect" => Ok(Self::McpDisconnect),
            "mcp.invoke" => Ok(Self::McpInvoke),
            "entitlement.evaluate" => Ok(Self::EntitlementEvaluate),
            "update.install" => Ok(Self::UpdateInstall),
            "auth.login_failed" => Ok(Self::AuthLoginFailed),
            "mcp.server_create" => Ok(Self::McpServerCreate),
            "mcp.server_update" => Ok(Self::McpServerUpdate),
            "mcp.server_delete" => Ok(Self::McpServerDelete),
            "mcp.sign_in" => Ok(Self::McpSignIn),
            "mcp.sign_out" => Ok(Self::McpSignOut),
            "skill.import" => Ok(Self::SkillImport),
            "skill.delete" => Ok(Self::SkillDelete),
            "data.export" => Ok(Self::DataExport),
            "data.erase" => Ok(Self::DataErase),
            "diagnostics.export" => Ok(Self::DiagnosticsExport),
            _ => Err(AppError::PersistenceInvalid("unsupported audit action")),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthLogin => "auth.login",
            Self::AuthLogout => "auth.logout",
            Self::CredentialCreate => "credential.create",
            Self::CredentialReplace => "credential.replace",
            Self::CredentialDelete => "credential.delete",
            Self::CredentialUse => "credential.use",
            Self::ProjectTrust => "project.trust",
            Self::ProjectRevoke => "project.revoke",
            Self::ProjectRead => "project.read",
            Self::ProjectWrite => "project.write",
            Self::ToolApprove => "tool.approve",
            Self::ToolDeny => "tool.deny",
            Self::ToolExecute => "tool.execute",
            Self::McpConnect => "mcp.connect",
            Self::McpDisconnect => "mcp.disconnect",
            Self::McpInvoke => "mcp.invoke",
            Self::EntitlementEvaluate => "entitlement.evaluate",
            Self::UpdateInstall => "update.install",
            Self::AuthLoginFailed => "auth.login_failed",
            Self::McpServerCreate => "mcp.server_create",
            Self::McpServerUpdate => "mcp.server_update",
            Self::McpServerDelete => "mcp.server_delete",
            Self::McpSignIn => "mcp.sign_in",
            Self::McpSignOut => "mcp.sign_out",
            Self::SkillImport => "skill.import",
            Self::SkillDelete => "skill.delete",
            Self::DataExport => "data.export",
            Self::DataErase => "data.erase",
            Self::DiagnosticsExport => "diagnostics.export",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditResourceType {
    Account,
    Credential,
    Project,
    ToolCall,
    McpServer,
    Entitlement,
    Update,
    Session,
    Skill,
    Export,
    LocalData,
    Diagnostics,
}

impl AuditResourceType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Account => "account",
            Self::Credential => "credential",
            Self::Project => "project",
            Self::ToolCall => "tool_call",
            Self::McpServer => "mcp_server",
            Self::Entitlement => "entitlement",
            Self::Update => "update",
            Self::Session => "session",
            Self::Skill => "skill",
            Self::Export => "export",
            Self::LocalData => "local_data",
            Self::Diagnostics => "diagnostics",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    Allowed,
    ApprovalRequired,
    Approved,
    Rejected,
    Denied,
    Updated,
    Success,
    Failure,
}

impl AuditOutcome {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::ApprovalRequired => "approval_required",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Denied => "denied",
            Self::Updated => "updated",
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditEvent {
    event_id: Uuid,
    sequence: u64,
    actor_id: String,
    tenant_id: String,
    device_id: String,
    action: AuditAction,
    resource_type: AuditResourceType,
    resource_id: String,
    outcome: AuditOutcome,
    policy_id: String,
    request_id: String,
    occurred_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
}

impl AuditEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        event_id: Uuid,
        sequence: u64,
        actor_id: impl Into<String>,
        tenant_id: impl Into<String>,
        device_id: impl Into<String>,
        action: AuditAction,
        resource_type: AuditResourceType,
        resource_id: impl Into<String>,
        outcome: AuditOutcome,
        policy_id: impl Into<String>,
        request_id: impl Into<String>,
        occurred_at_ms: i64,
    ) -> AppResult<Self> {
        let event = Self {
            event_id,
            sequence,
            actor_id: actor_id.into(),
            tenant_id: tenant_id.into(),
            device_id: device_id.into(),
            action,
            resource_type,
            resource_id: resource_id.into(),
            outcome,
            policy_id: policy_id.into(),
            request_id: request_id.into(),
            occurred_at_ms,
            detail: None,
        };
        event.validate()?;
        Ok(event)
    }

    /// Attaches one optional content-free identifier, such as a tool id.
    /// Validated with the same rules as every other identifier.
    pub fn with_detail(mut self, detail: impl Into<String>) -> AppResult<Self> {
        let detail = detail.into();
        validate_identifier(&detail)?;
        self.detail = Some(detail);
        Ok(self)
    }

    pub(crate) fn validate(&self) -> AppResult<()> {
        if self.event_id.is_nil() {
            return Err(AppError::PersistenceInvalid(
                "audit event_id must not be nil",
            ));
        }
        if self.sequence == 0 {
            return Err(AppError::PersistenceInvalid(
                "audit sequence must be greater than zero",
            ));
        }
        validate_identifier(&self.actor_id)?;
        validate_identifier(&self.tenant_id)?;
        validate_identifier(&self.device_id)?;
        validate_identifier(&self.resource_id)?;
        validate_identifier(&self.policy_id)?;
        validate_identifier(&self.request_id)?;
        if let Some(detail) = &self.detail {
            validate_identifier(detail)?;
        }
        Ok(())
    }

    #[must_use]
    pub const fn event_id(&self) -> Uuid {
        self.event_id
    }

    #[must_use]
    pub const fn sequence(&self) -> u64 {
        self.sequence
    }

    #[must_use]
    pub fn actor_id(&self) -> &str {
        &self.actor_id
    }

    #[must_use]
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    #[must_use]
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    #[must_use]
    pub const fn action(&self) -> AuditAction {
        self.action
    }

    #[must_use]
    pub const fn resource_type(&self) -> AuditResourceType {
        self.resource_type
    }

    #[must_use]
    pub fn resource_id(&self) -> &str {
        &self.resource_id
    }

    #[must_use]
    pub const fn outcome(&self) -> AuditOutcome {
        self.outcome
    }

    #[must_use]
    pub fn policy_id(&self) -> &str {
        &self.policy_id
    }

    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }

    #[must_use]
    pub const fn occurred_at_ms(&self) -> i64 {
        self.occurred_at_ms
    }

    #[must_use]
    pub fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

fn validate_identifier(value: &str) -> AppResult<()> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
    {
        return Err(AppError::PersistenceInvalid("invalid audit identifier"));
    }
    Ok(())
}
