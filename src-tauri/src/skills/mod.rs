//! Constrained, lazily-loaded skills.
//!
//! A skill is a `SKILL.md` file: a data-only frontmatter manifest
//! (`manifest`) plus an instructions body that is treated as inert text,
//! never executed, and any files it bundles beside it, which are inert data
//! the model may read by path (`dispatch`). Skills come from two sources —
//! the ones the user imported through Settings (`package` reads what they
//! chose, `store` keeps it), under each account's `skills/*/SKILL.md`,
//! and (once a caller resolves project trust)
//! `<trusted-root>/.xfalcon/skills/*/SKILL.md` — discovered by `discovery`
//! and cataloged by `registry`. This app bundles none of its own.
//!
//! A skill can only ever narrow the effective tool policy for a run
//! (`policy`), never expand it; see `crate::tools::policy`, whose
//! `SkillToolScope` and `PolicyEngine::authorize` ordering this module
//! composes with rather than duplicates.
//!
//! Skills are not all loaded into every agent context: discovery/cataloging
//! surfaces only name, description, version, source, and content hash for
//! every skill; a skill's full instructions body is loaded only when the
//! model asks for it by name (`dispatch`, serving
//! `__internal__get_skill`) or when a skill is explicitly selected
//! (`registry::SkillRegistry::select`). See
//! `docs/decisions.md` for the
//! reference implementations this design was compared against.

pub mod commands;
pub mod discovery;
pub mod dispatch;
mod drop;
pub mod manifest;
mod package;
pub mod policy;
pub mod registry;
pub mod store;

pub use discovery::{DiscoveredSkill, DiscoveryError, SkillSource, TrustState};
pub use drop::register_skill_drop_handler;
pub use manifest::{
    ManifestError, ParsedSkillDocument, SkillManifest, SkillName, SkillProjectPath, SkillVersion,
};
pub use policy::effective_skill_scope;
pub use registry::{SelectedSkill, SkillRegistry, SkillRegistryError, SkillSelection};
pub use store::{SkillStore, SkillStoreError};

#[cfg(test)]
mod tests;
