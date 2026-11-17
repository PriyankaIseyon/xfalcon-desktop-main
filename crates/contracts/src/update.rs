use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdateCheckResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub current_version: String,
    pub available_version: Option<String>,
    pub release_notes: Option<String>,
    pub is_staged: bool,
}

impl UpdateCheckResult {
    pub fn new(
        current_version: impl Into<String>,
        available_version: Option<String>,
        release_notes: Option<String>,
        is_staged: bool,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            current_version: current_version.into(),
            available_version,
            release_notes,
            is_staged,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct UpdateDownloadProgress {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub downloaded_bytes: u32,
    pub total_bytes: Option<u32>,
    pub percent: Option<u8>,
}

impl UpdateDownloadProgress {
    pub fn new(downloaded_bytes: u32, total_bytes: Option<u32>) -> Self {
        let percent = match total_bytes {
            Some(total) if total > 0 => {
                Some(((u64::from(downloaded_bytes) * 100 / u64::from(total)).min(100)) as u8)
            }
            _ => None,
        };
        Self {
            version: CONTRACT_VERSION,
            downloaded_bytes,
            total_bytes,
            percent,
        }
    }
}
