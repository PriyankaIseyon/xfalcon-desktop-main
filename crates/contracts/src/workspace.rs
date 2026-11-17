use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct McpServerSummary {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: String,
    pub name: String,
    pub transport: String,
    pub enabled: bool,
}

impl McpServerSummary {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        transport: impl Into<String>,
        enabled: bool,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id: id.into(),
            name: name.into(),
            transport: transport.into(),
            enabled,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceStatus {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub tenant_host: String,
    pub mcp_resource_url: String,
    pub mcp_servers: Vec<McpServerSummary>,
    pub models_available: u32,
    pub default_model_id: Option<String>,
    pub default_provider: Option<String>,
    pub chat_ready: bool,
    pub setup_note: String,
}
