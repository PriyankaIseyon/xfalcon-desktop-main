use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::ContractVersion;

/// Where the Claude runtime stands on this device.
///
/// `disabled` means the user has not turned it on, and in that state Desktop
/// has not started `claude` at all.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ClaudeRuntimeState {
    Disabled,
    NotInstalled,
    Outdated,
    SignedOut,
    Ready,
    CheckFailed,
}

/// How deep `claude` reasons. It is passed to `claude` as `--effort`. `ultracode`
/// is not offered: it switches on Claude Code's own workflow mode.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ClaudeEffort {
    Low,
    Medium,
    #[default]
    High,
    Xhigh,
    Max,
}

impl ClaudeEffort {
    pub const ALL: [ClaudeEffort; 5] = [
        ClaudeEffort::Low,
        ClaudeEffort::Medium,
        ClaudeEffort::High,
        ClaudeEffort::Xhigh,
        ClaudeEffort::Max,
    ];

    /// The argument `claude --effort` takes for this level.
    pub const fn flag(self) -> &'static str {
        match self {
            ClaudeEffort::Low => "low",
            ClaudeEffort::Medium => "medium",
            ClaudeEffort::High => "high",
            ClaudeEffort::Xhigh => "xhigh",
            ClaudeEffort::Max => "max",
        }
    }
}

/// What Settings shows about the user's own Claude Code. It carries the plan
/// and the sign-in method that `claude` reports and nothing that identifies the
/// account.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ClaudeRuntimeStatusView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub enabled: bool,
    pub state: ClaudeRuntimeState,
    pub installed_version: Option<String>,
    pub required_version: String,
    pub plan: Option<String>,
    pub sign_in_method: Option<String>,
    /// The command the user runs in their own terminal to sign in.
    pub sign_in_command: String,
    /// The level a new chat starts with. A chat keeps the level it started with.
    pub effort: ClaudeEffort,
}

/// Turns a device-local plan provider on or off.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct PlanEnableInput {
    pub enabled: bool,
}

/// Sets the reasoning level new Claude chats start with.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, TS)]
#[serde(deny_unknown_fields)]
pub struct ClaudeEffortInput {
    pub effort: ClaudeEffort,
}

/// Where the ChatGPT plan sign-in stands for the signed-in xFalcon account.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ChatgptPlanState {
    SignedOut,
    SigningIn,
    Ready,
    /// The user signed in but did not grant plan usage.
    PlanUsageDisabled,
    /// The saved sign-in no longer works. Signing in again fixes it.
    NeedsSignIn,
    CheckFailed,
}

/// What Settings shows about the ChatGPT plan sign-in.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ChatgptPlanStatusView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub state: ChatgptPlanState,
    /// True until the user dismisses OpenAI's first-sign-in notice.
    pub welcome_pending: bool,
    /// The ChatGPT account the user signed in with, shown to that user only.
    pub account_label: Option<String>,
    pub model_count: u32,
    /// OpenAI's page where the user manages plan usage.
    pub usage_url: String,
    /// A short, safe message for the last failure, when there is one.
    pub error: Option<String>,
}
