use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ProviderId {
    #[serde(rename = "openai")]
    #[ts(rename = "openai")]
    OpenAi,
    Anthropic,
    Google,
    /// An aggregator, not a first-party model vendor: one key reaches many
    /// vendors' models over the OpenAI-compatible Chat Completions protocol.
    /// It is a distinct arm rather than an `OpenAi` variant with a different
    /// base URL because the credential, the wire dialect's extras, and the
    /// `provider` value DA stores all differ.
    #[serde(rename = "openrouter")]
    #[ts(rename = "openrouter")]
    OpenRouter,
    /// A private user-owned OpenAI-compatible endpoint with a model-bound key.
    #[serde(rename = "custom_openai")]
    #[ts(rename = "custom_openai")]
    CustomOpenAi,
    /// The user's own Claude account, reached by running their installed
    /// `claude` program. There is no provider key, so key storage refuses it.
    ClaudeCode,
    /// The user's ChatGPT plan, reached with a token from OpenAI's official
    /// sign-in. There is no provider key, so key storage refuses it.
    ChatgptPlan,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct CredentialId(String);

impl From<&str> for CredentialId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for CredentialId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct ModelId(String);

impl From<&str> for ModelId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ModelId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CredentialStatus {
    Ready,
    Disabled,
    NeedsReentry,
    VerificationFailed,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ProviderCredentialView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: CredentialId,
    pub provider: ProviderId,
    pub display_name: String,
    pub status: CredentialStatus,
    pub is_default: bool,
}

impl ProviderCredentialView {
    pub fn new(
        id: impl Into<CredentialId>,
        provider: ProviderId,
        display_name: impl Into<String>,
        status: CredentialStatus,
        is_default: bool,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id: id.into(),
            provider,
            display_name: display_name.into(),
            status,
            is_default,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CredentialErrorCode {
    InvalidRequest,
    InvalidCredential,
    InsufficientPermissions,
    ProviderUnavailable,
    RateLimited,
    StorageUnavailable,
    ServiceUnavailable,
    Conflict,
    ReconciliationRequired,
    Unauthorized,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CredentialOperationError {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub code: CredentialErrorCode,
    /// Present only when an ambiguous mutation was followed by a successful
    /// redacted list reconciliation. `None` means reconciliation itself failed.
    pub credentials: Option<Vec<ProviderCredentialView>>,
}

impl CredentialOperationError {
    pub fn new(
        code: CredentialErrorCode,
        credentials: Option<Vec<ProviderCredentialView>>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            code,
            credentials,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ModelCapabilitiesView {
    pub tools: bool,
    pub image_input: bool,
    pub reasoning_summary: bool,
}

impl ModelCapabilitiesView {
    pub const fn new(tools: bool, image_input: bool, reasoning_summary: bool) -> Self {
        Self {
            tools,
            image_input,
            reasoning_summary,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ModelView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: ModelId,
    pub provider: ProviderId,
    pub display_name: String,
    pub capabilities: ModelCapabilitiesView,
}

impl ModelView {
    pub fn new(
        id: impl Into<ModelId>,
        provider: ProviderId,
        display_name: impl Into<String>,
        capabilities: ModelCapabilitiesView,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            id: id.into(),
            provider,
            display_name: display_name.into(),
            capabilities,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn credential_error_has_a_stable_typed_wire_shape() {
        let error = CredentialOperationError::new(
            CredentialErrorCode::ReconciliationRequired,
            Some(Vec::new()),
        );

        assert_eq!(
            serde_json::to_value(error).expect("credential error serializes"),
            json!({
                "version": 1,
                "code": "reconciliation_required",
                "credentials": [],
            })
        );
    }

    #[test]
    fn credential_error_distinguishes_verifier_failures() {
        for (code, expected) in [
            (CredentialErrorCode::InvalidCredential, "invalid_credential"),
            (
                CredentialErrorCode::InsufficientPermissions,
                "insufficient_permissions",
            ),
            (
                CredentialErrorCode::ProviderUnavailable,
                "provider_unavailable",
            ),
            (CredentialErrorCode::RateLimited, "rate_limited"),
        ] {
            assert_eq!(
                serde_json::to_value(code).expect("code serializes"),
                json!(expected)
            );
        }
    }
}
