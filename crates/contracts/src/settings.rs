//! Device-local appearance preferences.
//!
//! Deliberately closed: enumerated choices and booleans, no free-form values.
//! The renderer stamps the two enums onto the document element, so anything
//! wider than an enum there would be an attribute value the frontend had to
//! sanitize.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

/// The palette to render. There is no "follow the system" member on purpose:
/// the choice is explicit, and a third mode would be a second source of truth
/// for what the app is currently showing.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ThemePreference {
    Dark,
    #[default]
    Light,
}

impl ThemePreference {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
        }
    }
}

/// How large the app's type ramp renders. One multiplier drives every type
/// role, so these three steps move the whole ramp rather than one size.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum ReadingSize {
    #[default]
    Small,
    Medium,
    Large,
}

impl ReadingSize {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }
}

/// The whole appearance preference set, read and written as one value.
///
/// `#[serde(default)]` on every field but `version` is what makes a
/// partially-written or older settings file yield defaults for what it does not
/// carry, instead of failing the read and leaving the app with nothing to
/// render.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AppearanceSettings {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    #[serde(default)]
    pub theme: ThemePreference,
    #[serde(default)]
    pub reading_size: ReadingSize,
    /// Whether the transcript offers a run's streamed reasoning behind its
    /// collapsed disclosure. A display preference, not a capability one: the
    /// reasoning is requested, streamed and bounded either way, because it is
    /// what makes a thinking model answer well (spec: chat-reasoning-panel).
    #[serde(default)]
    pub show_reasoning: bool,
    /// Whether settled assistant answers offer generated follow-up prompts.
    #[serde(default = "default_show_follow_ups")]
    pub show_follow_ups: bool,
}

const fn default_show_follow_ups() -> bool {
    true
}

impl AppearanceSettings {
    #[must_use]
    pub const fn new(
        theme: ThemePreference,
        reading_size: ReadingSize,
        show_reasoning: bool,
        show_follow_ups: bool,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            theme,
            reading_size,
            show_reasoning,
            show_follow_ups,
        }
    }
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self::new(ThemePreference::Light, ReadingSize::Small, false, true)
    }
}

/// The only way writing a preference fails. Reading cannot fail — an
/// unreadable file yields defaults — so there is no code for it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum SettingsErrorCode {
    /// The app-local settings file could not be located or written.
    StorageUnavailable,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SettingsOperationError {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub code: SettingsErrorCode,
}

impl SettingsOperationError {
    #[must_use]
    pub const fn new(code: SettingsErrorCode) -> Self {
        Self {
            version: CONTRACT_VERSION,
            code,
        }
    }
}
