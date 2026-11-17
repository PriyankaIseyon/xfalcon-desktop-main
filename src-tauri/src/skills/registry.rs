//! In-memory skill catalog with lazy relevance selection.
//!
//! Discovery (`super::discovery`) produces metadata-only [`DiscoveredSkill`]
//! entries; this registry holds them by name and only loads a skill's full
//! instructions body when [`SkillRegistry::select`] is called for it.
//! [`SkillSelection::Explicit`] (slash-command selection) is the only
//! concrete selection mechanism today; bounded semantic/name relevance
//! selection is a documented future variant of the same enum, not a
//! different loading path — either way, [`SkillRegistry::catalog`] exposes
//! only name/description/version/source/hash for every registered skill, and
//! a skill's instructions are never read until it is actually selected. This
//! mirrors `source-examples/claude-code-source-code-full`'s
//! `estimateSkillFrontmatterTokens` / lazy `getPromptForCommand` split (full
//! content "is only loaded on invocation").

use indexmap::IndexMap;

use super::discovery::{load_instructions, DiscoveredSkill, DiscoveryError, SkillSource};
use super::manifest::{SkillName, SkillVersion};

#[derive(Debug, thiserror::Error)]
pub enum SkillRegistryError {
    /// A skill name collided with one already registered — whether from the
    /// same source or a different one, and whether the declared version is
    /// identical or different. This registry's policy is to fail the
    /// registration closed rather than silently keep one and discard the
    /// other (contrast `source-examples/grok-build`'s `PluginRegistry`,
    /// which resolves a name collision by last-write-wins plus only a soft
    /// `conflict: Option<String>` annotation — rejected here, see
    /// `docs/decisions.md`).
    /// The error payload always names both sides, so a plain duplicate
    /// (identical version) and a genuine version conflict are equally
    /// diagnosable rather than silently dropped.
    #[error(
        "skill {name:?} already registered from {existing_source} at version {existing_version:?}; \
         cannot register {incoming_source} at version {incoming_version:?}"
    )]
    DuplicateSkillName {
        name: SkillName,
        existing_source: SkillSource,
        existing_version: SkillVersion,
        incoming_source: SkillSource,
        incoming_version: SkillVersion,
    },
    #[error("skill {0:?} is not registered")]
    UnknownSkill(SkillName),
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
}

/// A skill whose full instructions body has been loaded because it was
/// selected for the current run. Contrast [`DiscoveredSkill`], which never
/// carries this.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectedSkill {
    name: SkillName,
    instructions: String,
}

impl SelectedSkill {
    #[must_use]
    pub fn name(&self) -> &SkillName {
        &self.name
    }

    #[must_use]
    pub fn instructions(&self) -> &str {
        &self.instructions
    }
}

/// How a skill was chosen to have its body loaded for a run. Only
/// [`Self::Explicit`] (slash-command invocation) exists today; bounded
/// semantic/name relevance selection is a future variant of this same enum
/// — see the module doc comment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SkillSelection {
    Explicit(SkillName),
}

/// Holds discovered skills by name, deterministically ordered (via
/// `IndexMap`, mirroring `crate::tools::registry::ToolRegistry`'s own
/// choice for the same audit/listing-determinism reason).
#[derive(Debug, Default)]
pub struct SkillRegistry {
    skills: IndexMap<SkillName, DiscoveredSkill>,
}

impl SkillRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            skills: IndexMap::new(),
        }
    }

    /// Registers a discovered skill. Fails closed on any name collision —
    /// see [`SkillRegistryError::DuplicateSkillName`]'s doc comment.
    pub fn register(&mut self, skill: DiscoveredSkill) -> Result<(), SkillRegistryError> {
        let name = skill.manifest().name().clone();
        if let Some(existing) = self.skills.get(&name) {
            return Err(SkillRegistryError::DuplicateSkillName {
                name,
                existing_source: existing.source(),
                existing_version: existing.manifest().version().clone(),
                incoming_source: skill.source(),
                incoming_version: skill.manifest().version().clone(),
            });
        }
        self.skills.insert(name, skill);
        Ok(())
    }

    /// The full catalog: name/description/version/source/hash for every
    /// registered skill, with no instructions body loaded.
    pub fn catalog(&self) -> impl Iterator<Item = &DiscoveredSkill> {
        self.skills.values()
    }

    #[must_use]
    pub fn get(&self, name: &SkillName) -> Option<&DiscoveredSkill> {
        self.skills.get(name)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.skills.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.skills.is_empty()
    }

    /// Loads a skill's full instructions body for `selection` — the lazy
    /// relevance-selection seam (see the module doc comment). This is the
    /// only path in this module that reads a `SKILL.md` file's body.
    pub fn select(&self, selection: &SkillSelection) -> Result<SelectedSkill, SkillRegistryError> {
        let SkillSelection::Explicit(name) = selection;
        let discovered = self
            .get(name)
            .ok_or_else(|| SkillRegistryError::UnknownSkill(name.clone()))?;
        let instructions = load_instructions(discovered)?;
        Ok(SelectedSkill {
            name: name.clone(),
            instructions,
        })
    }
}
