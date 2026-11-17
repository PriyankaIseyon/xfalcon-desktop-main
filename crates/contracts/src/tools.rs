//! Wire vocabulary for the policy-first tool/invocation domain.
//!
//! Raw tool descriptions, advertised schemas, invocation arguments, and
//! structured tool output never cross this boundary: `xfalcon-contracts` has
//! no runtime `serde_json` dependency, and every type here is a bounded,
//! versioned projection. Runtime state remains authoritative in
//! `src-tauri/src/tools`; the UI receives projections only.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, RunId, CONTRACT_VERSION};

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct ToolId(String);

impl ToolId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ToolId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ToolId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct InvocationId(String);

impl InvocationId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for InvocationId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for InvocationId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

/// Whether a tool is one the desktop implements itself or one an MCP server
/// advertised. Which MCP server is not carried here: [`ToolId`] is namespaced
/// `mcp.<server_slug>.<tool>`, so the id the model, the activity feed, and the
/// approval prompt all already show names its server. Keeping this `Copy` and
/// two-variant is what lets `crate::tools::invocation` use it as a provenance
/// marker throughout.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ToolSource {
    Internal,
    Mcp,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ToolRisk {
    ReadOnly,
    Mutating,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ToolDescriptorView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: ToolId,
    pub source: ToolSource,
    pub name: String,
    pub description: String,
    pub risk: ToolRisk,
    pub capabilities: Vec<String>,
    pub idempotent: bool,
}

impl ToolDescriptorView {
    pub fn new(
        id: impl Into<ToolId>,
        source: ToolSource,
        name: impl Into<String>,
        description: impl Into<String>,
        risk: ToolRisk,
        capabilities: Vec<String>,
        idempotent: bool,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id: id.into(),
            source,
            name: name.into(),
            description: description.into(),
            risk,
            capabilities,
            idempotent,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ToolInvocationStatus {
    Requested,
    AwaitingApproval,
    Approved,
    Denied,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum ToolInvocationEvent {
    Requested {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        invocation_id: InvocationId,
        run_id: RunId,
        tool_id: ToolId,
    },
    ApprovalRequired {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        invocation_id: InvocationId,
        tool_id: ToolId,
    },
    Approved {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        invocation_id: InvocationId,
    },
    Denied {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        invocation_id: InvocationId,
        reason_code: String,
    },
    Started {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        invocation_id: InvocationId,
    },
    Progress {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        invocation_id: InvocationId,
        #[ts(type = "number")]
        sequence: u64,
        summary: String,
    },
    Completed {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        invocation_id: InvocationId,
        truncated: bool,
    },
    Failed {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        invocation_id: InvocationId,
        message: String,
        retryable: bool,
    },
    Cancelled {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        invocation_id: InvocationId,
    },
}

impl ToolInvocationEvent {
    pub fn requested(
        invocation_id: impl Into<InvocationId>,
        run_id: RunId,
        tool_id: impl Into<ToolId>,
    ) -> Self {
        Self::Requested {
            version: CONTRACT_VERSION,
            invocation_id: invocation_id.into(),
            run_id,
            tool_id: tool_id.into(),
        }
    }

    pub fn approval_required(
        invocation_id: impl Into<InvocationId>,
        tool_id: impl Into<ToolId>,
    ) -> Self {
        Self::ApprovalRequired {
            version: CONTRACT_VERSION,
            invocation_id: invocation_id.into(),
            tool_id: tool_id.into(),
        }
    }

    pub fn approved(invocation_id: impl Into<InvocationId>) -> Self {
        Self::Approved {
            version: CONTRACT_VERSION,
            invocation_id: invocation_id.into(),
        }
    }

    pub fn denied(invocation_id: impl Into<InvocationId>, reason_code: impl Into<String>) -> Self {
        Self::Denied {
            version: CONTRACT_VERSION,
            invocation_id: invocation_id.into(),
            reason_code: reason_code.into(),
        }
    }

    pub fn started(invocation_id: impl Into<InvocationId>) -> Self {
        Self::Started {
            version: CONTRACT_VERSION,
            invocation_id: invocation_id.into(),
        }
    }

    pub fn progress(
        invocation_id: impl Into<InvocationId>,
        sequence: u64,
        summary: impl Into<String>,
    ) -> Self {
        Self::Progress {
            version: CONTRACT_VERSION,
            invocation_id: invocation_id.into(),
            sequence,
            summary: summary.into(),
        }
    }

    pub fn completed(invocation_id: impl Into<InvocationId>, truncated: bool) -> Self {
        Self::Completed {
            version: CONTRACT_VERSION,
            invocation_id: invocation_id.into(),
            truncated,
        }
    }

    pub fn failed(
        invocation_id: impl Into<InvocationId>,
        message: impl Into<String>,
        retryable: bool,
    ) -> Self {
        Self::Failed {
            version: CONTRACT_VERSION,
            invocation_id: invocation_id.into(),
            message: message.into(),
            retryable,
        }
    }

    pub fn cancelled(invocation_id: impl Into<InvocationId>) -> Self {
        Self::Cancelled {
            version: CONTRACT_VERSION,
            invocation_id: invocation_id.into(),
        }
    }
}
