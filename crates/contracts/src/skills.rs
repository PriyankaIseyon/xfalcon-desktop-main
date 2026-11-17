//! The signed-in account's imported skills, as Settings sees them.
//!
//! A skill's instructions body is deliberately absent from every view here.
//! The catalog is metadata; the body is loaded only when a run's model asks
//! for it by name (`__internal__get_skill`), which is the same progressive
//! disclosure xfalcon-web uses and the reason a library of 50 KB skills does
//! not have to fit in a prompt.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

/// One imported skill. `name` is both the declared name and the directory it
/// is stored under — import normalizes them into the same slug, so there is
/// no second identifier to keep in step.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SkillView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub name: String,
    pub description: String,
    /// The manifest's own `version`, or `0.0.0` when it declares none.
    pub declared_version: String,
    /// Whether runs are told about it. A disabled skill stays on disk.
    pub enabled: bool,
    /// SHA-256 of the stored `SKILL.md` bytes, so a re-import that changed
    /// nothing is visibly a no-op.
    pub content_hash: String,
    /// How many files the skill bundles beside its `SKILL.md` — references,
    /// scripts, assets. Zero for a skill imported as a single file.
    pub bundled_files: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SkillListResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub skills: Vec<SkillView>,
}

impl SkillListResult {
    pub fn new(skills: Vec<SkillView>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            skills,
        }
    }
}

/// One chosen file that did not import, and why. Reported per file rather
/// than failing the whole selection: choosing five files and losing all five
/// to one malformed sibling is not a useful outcome.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SkillImportFailure {
    /// The file's name alone, for display: never its full path. A file nested
    /// in a dropped folder has a path the webview was never given.
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SkillImportResult {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub imported: Vec<SkillView>,
    pub failed: Vec<SkillImportFailure>,
}

impl SkillImportResult {
    pub fn new(imported: Vec<SkillView>, failed: Vec<SkillImportFailure>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            imported,
            failed,
        }
    }
}

/// Which native picker Settings opens for skills.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum SkillPickKind {
    /// Skill files and archives, several at once.
    Files,
    /// A folder, searched for skills beneath it.
    Folder,
}

/// What the person chose for an import: a grant id to import with, and how
/// much of it there is. `labels` are the final path component of up to a few
/// of the sources, for display only; no path crosses this view.
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SkillSourceGrantView {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub grant_id: String,
    pub count: u32,
    pub labels: Vec<String>,
}

impl SkillSourceGrantView {
    pub fn new(grant_id: impl Into<String>, count: u32, labels: Vec<String>) -> Self {
        Self {
            version: CONTRACT_VERSION,
            grant_id: grant_id.into(),
            count,
            labels,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum SkillErrorCode {
    /// No skill of that name is imported, or it is disabled. One code for
    /// both: from the caller's side they are the same miss, and splitting
    /// them would tell an unauthorized caller which names exist.
    NotFound,
    /// The skills directory could not be read or written.
    StorageUnavailable,
    /// No account is signed in. Each account has its own library, so there
    /// is no library to read or write until one is.
    NoActiveAccount,
    /// The grant id is unknown, expired, or was issued for something else.
    /// The choice is no longer usable, so the person chooses again.
    InvalidChoice,
    /// The selection names more files than one choice can carry. Nothing was
    /// imported; the person chooses fewer files, or a folder.
    SelectionTooLarge,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct SkillOperationError {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub code: SkillErrorCode,
}

impl SkillOperationError {
    pub fn new(code: SkillErrorCode) -> Self {
        Self {
            version: CONTRACT_VERSION,
            code,
        }
    }
}
