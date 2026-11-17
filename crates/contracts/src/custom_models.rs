use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use ts_rs::TS;

use crate::ContractVersion;

/// The configuration Desktop may display. DA never returns the stored key.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CustomModelView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub id: String,
    pub model_id: String,
    pub display_name: String,
    pub base_url: String,
    pub upstream_model_id: String,
    // DA bounds these to i32::MAX, so the JSON numbers are safe in JavaScript.
    #[ts(type = "number")]
    pub context_length: i64,
    #[ts(type = "number | null")]
    pub max_output_tokens: Option<i64>,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub sampling: CustomModelSampling,
    pub is_enabled: bool,
    pub is_active: bool,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CustomModelSampling {
    #[serde(default, deserialize_with = "temperature")]
    pub temperature: Option<f64>,
    #[serde(default, deserialize_with = "top_p")]
    pub top_p: Option<f64>,
}

/// Plaintext is accepted only on the input path and is moved into zeroizing
/// memory before the authenticated DA request is built.
#[derive(Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct CustomModelCreateInput {
    pub base_url: String,
    pub secret: String,
    pub upstream_model_id: String,
    pub display_name: String,
    #[ts(type = "number")]
    pub context_length: i64,
    #[ts(type = "number")]
    pub max_output_tokens: i64,
    pub supports_tools: bool,
    pub supports_vision: bool,
    pub sampling: CustomModelSampling,
}

#[derive(Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct CustomModelUpdateInput {
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "string")]
    pub base_url: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "string")]
    pub secret: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "boolean")]
    pub is_enabled: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "boolean")]
    pub is_active: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "string")]
    pub upstream_model_id: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "string")]
    pub display_name: Option<String>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "number")]
    pub context_length: Option<i64>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "number")]
    pub max_output_tokens: Option<i64>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "boolean")]
    pub supports_tools: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "boolean")]
    pub supports_vision: Option<bool>,
    #[serde(
        default,
        deserialize_with = "present_non_null",
        skip_serializing_if = "Option::is_none"
    )]
    #[ts(type = "CustomModelSampling")]
    pub sampling: Option<CustomModelSampling>,
}

#[derive(Deserialize, JsonSchema, TS)]
#[serde(deny_unknown_fields)]
pub struct CustomModelDiscoveryInput {
    pub base_url: String,
    pub secret: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct CustomModelDiscoveryResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub models: Vec<DiscoveredCustomModel>,
    pub existing_models: Vec<ExistingCustomModel>,
    pub unavailable_reason: Option<CustomModelDiscoveryUnavailableReason>,
}

/// Models already saved with this endpoint and key. No credential crosses IPC.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ExistingCustomModel {
    pub upstream_model_id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum CustomModelDiscoveryUnavailableReason {
    Unsupported,
    Unreachable,
    InvalidResponse,
    Empty,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DiscoveredCustomModel {
    pub upstream_model_id: String,
    pub display_name: String,
    #[ts(type = "number | null")]
    pub context_length: Option<i64>,
    #[ts(type = "number | null")]
    pub max_output_tokens: Option<i64>,
    pub supports_tools: Option<bool>,
    pub supports_vision: Option<bool>,
    pub sampling: CustomModelSampling,
    pub supported_sampling_parameters: Option<Vec<String>>,
}

// A missing update field means "keep it"; an explicit null is invalid in DA.
// Serde's ordinary Option<T> would silently collapse both to None at IPC.
fn present_non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn temperature<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    bounded_number(deserializer, 2.0, "temperature")
}

fn top_p<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    bounded_number(deserializer, 1.0, "top_p")
}

fn bounded_number<'de, D>(
    deserializer: D,
    maximum: f64,
    field: &str,
) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<f64>::deserialize(deserializer)?;
    match value {
        Some(value) if value.is_finite() && (0.0..=maximum).contains(&value) => Ok(Some(value)),
        Some(_) => Err(serde::de::Error::custom(format!(
            "{field} must be finite and between 0 and {maximum}"
        ))),
        None => Ok(None),
    }
}
