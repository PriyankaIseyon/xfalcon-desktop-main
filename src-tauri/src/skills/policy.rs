//! Skill-declared tool narrowing, composed with
//! [`crate::tools::policy::SkillToolScope`].
//!
//! A skill's `allowed_tools` can only ever narrow the tool set an active
//! run's global/project policy already permits — it can never widen it. This
//! module adds an explicit, independent intersection on top of
//! `crate::tools::policy::PolicyEngine::authorize`'s own ordering (which
//! already checks the global allowlist before it ever consults a
//! `SkillToolScope`, per that module's doc comment): the `SkillToolScope`
//! [`effective_skill_scope`] produces has already been intersected with the
//! effective allowlist before `PolicyEngine` ever sees it, so a skill cannot
//! grant itself a tool absent from global/project policy even if
//! `PolicyEngine`'s own check ordering were ever changed. This mirrors that
//! module's idiom deliberately (see
//! `docs/decisions.md`): a
//! pure, `#[must_use]` function computing a narrowed set, not a mutable
//! filter.

use std::collections::HashSet;

use xfalcon_contracts::tools::ToolId;

use crate::tools::policy::SkillToolScope;

use super::manifest::SkillManifest;

/// Computes the effective [`SkillToolScope`] for `manifest`: the
/// intersection of its declared `allowed_tools` with `effective_allowlist`
/// (the caller's already-narrowed global ∩ project ∩ run policy — see the
/// plan's "Policy intersection" note). A tool the skill declares but that is
/// absent from `effective_allowlist` is silently dropped from the resulting
/// scope: this is the expected, safe outcome of narrowing, not an error —
/// only *widening* beyond `effective_allowlist` would be a bug, and this
/// function's return type makes that structurally impossible.
#[must_use]
pub fn effective_skill_scope(
    manifest: &SkillManifest,
    effective_allowlist: &HashSet<ToolId>,
) -> SkillToolScope {
    let narrowed = manifest
        .allowed_tools()
        .iter()
        .filter(|tool_id| effective_allowlist.contains(*tool_id))
        .cloned();
    SkillToolScope::new(narrowed)
}
