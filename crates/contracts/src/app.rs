use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{auth::SessionProjection, ContractVersion, CONTRACT_VERSION};

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum Platform {
    Windows,
    Macos,
    Linux,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AppBootstrap {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub application_version: String,
    pub platform: Platform,
    pub session: SessionProjection,
}

impl AppBootstrap {
    pub fn new(application_version: impl Into<String>, platform: Platform) -> Self {
        Self {
            version: CONTRACT_VERSION,
            application_version: application_version.into(),
            platform,
            session: SessionProjection::unauthenticated(),
        }
    }

    #[must_use]
    pub fn with_session(mut self, session: SessionProjection) -> Self {
        self.session = session;
        self
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum AppHealthStatus {
    Ready,
    Degraded,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AppHealth {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub status: AppHealthStatus,
}

impl AppHealth {
    pub const fn new(status: AppHealthStatus) -> Self {
        Self {
            version: CONTRACT_VERSION,
            status,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum LocalDataResource {
    ActiveWork,
    SessionVault,
    Journal,
    LocalDatabase,
    LocalProjections,
    ProjectData,
    UploadTemporary,
    ArtifactCache,
    Export,
    DecryptedWorkingFiles,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CleanupErrorCode {
    Busy,
    NoActiveAccount,
    VaultUnavailable,
    StorageUnavailable,
    CleanupIncomplete,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CleanupRetryPolicy {
    UserInitiated,
    AfterRestart,
    DoNotRetry,
}

impl CleanupRetryPolicy {
    const fn is_retryable(self) -> bool {
        !matches!(self, Self::DoNotRetry)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CleanupNextAction {
    SignIn,
    RetryClearLocalData,
    RestartAndRetry,
    ContactSupport,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CleanupFailure {
    pub resource: LocalDataResource,
    pub code: CleanupErrorCode,
    pub retryable: bool,
    pub retry_policy: CleanupRetryPolicy,
    pub next_action: CleanupNextAction,
}

impl CleanupFailure {
    pub const fn new(
        resource: LocalDataResource,
        code: CleanupErrorCode,
        retry_policy: CleanupRetryPolicy,
        next_action: CleanupNextAction,
    ) -> Self {
        Self {
            resource,
            code,
            retryable: retry_policy.is_retryable(),
            retry_policy,
            next_action,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum LocalDataClearStatus {
    Cleared,
    Partial,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LocalDataClearResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub correlation_id: String,
    pub status: LocalDataClearStatus,
    pub cleared: Vec<LocalDataResource>,
    pub failures: Vec<CleanupFailure>,
}

impl LocalDataClearResult {
    pub fn new(
        correlation_id: impl Into<String>,
        cleared: Vec<LocalDataResource>,
        failures: Vec<CleanupFailure>,
    ) -> Self {
        let status = if failures.is_empty() {
            LocalDataClearStatus::Cleared
        } else if cleared.is_empty() {
            LocalDataClearStatus::Failed
        } else {
            LocalDataClearStatus::Partial
        };
        Self {
            version: CONTRACT_VERSION,
            correlation_id: correlation_id.into(),
            status,
            cleared,
            failures,
        }
    }
}
