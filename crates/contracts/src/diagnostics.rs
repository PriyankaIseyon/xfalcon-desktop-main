use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RendererErrorReport {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub message: String,
    pub component_stack: Option<String>,
}

impl RendererErrorReport {
    pub fn new(message: impl Into<String>, component_stack: Option<String>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            message: message.into(),
            component_stack,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsBundleResponse {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub content: String,
    pub suggested_filename: String,
}

impl DiagnosticsBundleResponse {
    pub fn new(content: impl Into<String>, suggested_filename: impl Into<String>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            content: content.into(),
            suggested_filename: suggested_filename.into(),
        }
    }
}
