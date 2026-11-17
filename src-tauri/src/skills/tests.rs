//! Parser/security tests for the skills domain: strict manifest parsing,
//! discovery (symlink escape, project-skill-before-trust), the registry's
//! duplicate/version-conflict and lazy-relevance-selection behavior, and the
//! skill-narrows-never-expands policy invariant composed with
//! `crate::tools::policy`. See
//! `docs/decisions.md`.

use std::collections::HashSet;
use std::path::Path;

use serde_json::json;

use xfalcon_contracts::projects::ProjectTrustStatus;
use xfalcon_contracts::tools::{InvocationId, ToolId, ToolRisk, ToolSource as ContractToolSource};

use super::discovery::{
    discover_project, discover_user, DiscoveredSkill, DiscoveryError, SkillSource, TrustState,
};
use super::manifest::{
    parse_skill_document, ManifestError, SkillName, SkillVersion, MAX_INSTRUCTIONS_BYTES,
};
use super::policy::effective_skill_scope;
use super::registry::{SkillRegistry, SkillRegistryError, SkillSelection};
use crate::tools::policy::{Actor, PolicyDecision, PolicyDenial, PolicyEngine, PolicyRequest};
use crate::tools::registry::{ToolDescriptor, ToolRegistry};

/// Renders a YAML flow sequence, quoting each item so it survives the parser
/// verbatim.
///
/// Single quotes, not double: a YAML *double*-quoted scalar processes escape
/// sequences, so an adversarial fixture path like `C:\Users\attacker` had its
/// `\U` read as a (malformed) unicode escape and the document failed to parse
/// at all — the test then saw `MalformedYaml` instead of the `AbsolutePath`
/// rejection it was written to assert. A single-quoted scalar takes
/// backslashes literally; the only escape is `'` itself, doubled.
fn yaml_list(items: &[&str]) -> String {
    format!(
        "[{}]",
        items
            .iter()
            .map(|item| format!("'{}'", item.replace('\'', "''")))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn skill_document(
    name: &str,
    description: &str,
    version: &str,
    allowed_tools: &[&str],
    allowed_project_paths: &[&str],
    instructions: &str,
) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\nversion: {version}\nallowed_tools: {}\nallowed_project_paths: {}\n---\n{instructions}",
        yaml_list(allowed_tools),
        yaml_list(allowed_project_paths),
    )
}

fn write_skill(root: &Path, dir_name: &str, content: &str) {
    let dir = root.join(dir_name);
    std::fs::create_dir_all(&dir).expect("create skill dir");
    std::fs::write(dir.join("SKILL.md"), content).expect("write SKILL.md");
}

// ── manifest::parse_skill_document ──────────────────────────────────────

#[test]
fn malformed_yaml_frontmatter_rejected() {
    // Unbalanced flow sequence: invalid YAML syntax, not just an unknown
    // field.
    let raw = "---\nname: charting\ndescription: Build charts\nversion: 1.0.0\nallowed_tools: [da.query_data\nallowed_project_paths: []\n---\nBody.\n";
    let err = parse_skill_document(raw).expect_err("malformed YAML must be rejected");
    assert!(matches!(err, ManifestError::MalformedYaml(_)), "{err:?}");
}

#[test]
fn an_alias_expansion_bomb_in_frontmatter_cannot_run_away() {
    // The parser reads untrusted `SKILL.md` frontmatter, and YAML aliases let
    // a few hundred bytes name billions of nodes. Each level below refers to
    // the one before nine times, so a parser that expands them would build
    // 9^8 values. The document is a fraction of `MAX_FRONTMATTER_BYTES`, so
    // the size cap alone does not stop it; the parser's own budgets must.
    let mut bomb = String::from("a0: &a0 [x, x, x, x, x, x, x, x, x]\n");
    for level in 1..9 {
        let refs = vec![format!("*a{}", level - 1); 9].join(", ");
        bomb.push_str(&format!("a{level}: &a{level} [{refs}]\n"));
    }
    let raw = format!("---\nname: charting\ndescription: Build charts\n{bomb}---\nBody.\n");
    let started = std::time::Instant::now();
    let outcome = parse_skill_document(&raw);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "an alias bomb must be refused quickly, took {:?}",
        started.elapsed()
    );
    let err = outcome.expect_err("an alias bomb must be refused, not expanded");
    assert!(matches!(err, ManifestError::MalformedYaml(_)), "{err:?}");
}

#[test]
fn yaml_1_2_reads_yes_and_no_as_the_strings_they_are() {
    // YAML 1.1 turned `yes`, `no` and `on` into booleans. Every field in this
    // contract is a string, so a skill that spells its description `yes`
    // keeps it, and a version `1.2.3` stays a version.
    let raw = "---\nname: approvals\ndescription: yes\nversion: 1.2.3\n---\nBody.\n";
    let parsed = parse_skill_document(raw).expect("scalars typed as strings must parse");
    assert_eq!(parsed.manifest.description(), "yes");
    assert_eq!(parsed.manifest.version().as_str(), "1.2.3");
}

#[test]
fn unknown_frontmatter_fields_are_ignored_as_inert_data() {
    // Real skill files carry keys this contract has no place for --
    // `license`, `metadata`, an agent-specific `model`. Rejecting the file
    // over one would refuse the whole corpus; this contract has exactly two
    // policy fields and neither can widen a run's tools, so an unrecognized
    // key cannot be a policy field being hidden. See `manifest.rs`'s module
    // doc comment.
    let raw = skill_document(
        "charting",
        "Build validated analytical charts",
        "1.0.0",
        &["da.query_data"],
        &[],
        "Body.",
    );
    let raw = raw.replacen(
        "version: 1.0.0",
        "version: 1.0.0\nlicense: Apache-2.0\nunexpected_field: nefarious",
        1,
    );
    let parsed = parse_skill_document(&raw).expect("unknown keys must not fail the parse");
    assert_eq!(parsed.manifest.name().as_str(), "charting");
    assert_eq!(parsed.manifest.allowed_tools().len(), 1);
}

#[test]
fn a_real_skill_file_shape_parses() {
    // Every skill in the `data-analyst` repository declares `name` and
    // `description` and nothing else. A parser that cannot read them is not
    // fail-closed, it is fail-useless.
    let raw = "---\nname: ida-dashboard\ndescription: \"Build self-contained interactive HTML dashboards.\"\n---\n# Dashboard\n";
    let parsed = parse_skill_document(raw).expect("an IDA skill must parse");
    assert_eq!(parsed.manifest.name().as_str(), "ida-dashboard");
    assert_eq!(
        parsed.manifest.version().as_str(),
        SkillVersion::UNVERSIONED
    );
    assert!(parsed.manifest.allowed_tools().is_empty());
}

#[test]
fn the_hyphenated_allowed_tools_spelling_is_honored() {
    // A skill written for another agent means to narrow its tools. Reading
    // the hyphenated key as "declares none" would silently ignore that.
    let raw = "---\nname: narrow\ndescription: \"Narrow.\"\nallowed-tools:\n  - da.query_data\n---\nBody.\n";
    let parsed = parse_skill_document(raw).expect("hyphenated spelling must parse");
    assert_eq!(parsed.manifest.allowed_tools().len(), 1);
}

#[test]
fn oversized_instructions_body_rejected() {
    let instructions = "a".repeat(MAX_INSTRUCTIONS_BYTES + 1);
    let raw = skill_document(
        "charting",
        "Build validated analytical charts",
        "1.0.0",
        &["da.query_data"],
        &[],
        &instructions,
    );
    let err = parse_skill_document(&raw).expect_err("oversized instructions must be rejected");
    assert!(
        matches!(err, ManifestError::InstructionsTooLarge(limit) if limit == MAX_INSTRUCTIONS_BYTES),
        "{err:?}"
    );
}

#[test]
fn path_traversal_in_allowed_project_paths_rejected() {
    let raw = skill_document(
        "charting",
        "Build validated analytical charts",
        "1.0.0",
        &["da.query_data"],
        &["../../etc/passwd"],
        "Body.",
    );
    let err = parse_skill_document(&raw).expect_err("path traversal must be rejected");
    assert!(matches!(err, ManifestError::PathTraversal(_)), "{err:?}");
}

#[test]
fn well_formed_document_parses() {
    let raw = skill_document(
        "charting",
        "Build validated analytical charts",
        "1.0.0",
        &["da.query_data"],
        &[],
        "# Charting\n\nBuild charts.",
    );
    let parsed = parse_skill_document(&raw).expect("valid document must parse");
    assert_eq!(parsed.manifest.name().as_str(), "charting");
    assert_eq!(parsed.manifest.version().as_str(), "1.0.0");
    assert_eq!(
        parsed.manifest.allowed_tools(),
        &[ToolId::from("da.query_data")]
    );
    assert!(parsed.instructions.contains("Build charts."));
}

// ── discovery::discover_user / discover_project ─────────────────────

#[test]
fn project_trust_status_converts_to_the_matching_trust_state() {
    assert_eq!(
        TrustState::from(ProjectTrustStatus::Trusted),
        TrustState::Trusted
    );
    assert_eq!(
        TrustState::from(ProjectTrustStatus::Unavailable),
        TrustState::Untrusted
    );
}

#[cfg(unix)]
#[test]
fn symlink_escape_in_project_skill_rejected() {
    use std::os::unix::fs::symlink;

    let trusted_root = tempfile::tempdir().expect("trusted root");
    let outside = tempfile::tempdir().expect("outside dir");

    // A real skill directory that lives OUTSIDE the trusted root.
    write_skill(
        outside.path(),
        "escaped",
        &skill_document(
            "escaped",
            "Escapes the trusted root",
            "1.0.0",
            &[],
            &[],
            "Body.",
        ),
    );

    // `.xfalcon/skills/escaped` is a symlink pointing outside the root.
    let skills_dir = trusted_root.path().join(".xfalcon").join("skills");
    std::fs::create_dir_all(&skills_dir).expect("skills dir");
    symlink(outside.path().join("escaped"), skills_dir.join("escaped"))
        .expect("plant escaping symlink");

    let err = discover_project(trusted_root.path(), TrustState::Trusted)
        .expect_err("symlink escape must be rejected");
    assert!(
        matches!(err, DiscoveryError::SymlinkEscape { .. }),
        "{err:?}"
    );
}

#[test]
fn project_skills_inert_before_trust() {
    let trusted_root = tempfile::tempdir().expect("trusted root");
    // Deliberately malformed: if discovery ever opened this file while
    // untrusted, parsing it would fail and this test would observe an
    // `Err`, not an empty `Ok`. Observing `Ok(vec![])` is therefore proof
    // the file was never read at all, not merely that it "failed
    // gracefully".
    let skills_dir = trusted_root.path().join(".xfalcon").join("skills");
    write_skill(&skills_dir, "malicious", "not a skill document at all");

    let discovered = discover_project(trusted_root.path(), TrustState::Untrusted)
        .expect("untrusted discovery must not error");
    assert!(
        discovered.is_empty(),
        "project skills must remain undiscovered before trust is active"
    );

    // Once trust is active, the very same on-disk content is inert in a
    // different way: it is read, but rejected as a malformed manifest
    // rather than silently accepted.
    let err = discover_project(trusted_root.path(), TrustState::Trusted)
        .expect_err("malformed manifest must be rejected once trust is active");
    assert!(matches!(err, DiscoveryError::Manifest { .. }), "{err:?}");
}

#[test]
fn imported_discovery_catalogs_metadata_only() {
    let root = tempfile::tempdir().expect("skills root");
    write_skill(
        root.path(),
        "charting",
        &skill_document(
            "charting",
            "Build validated analytical charts",
            "1.0.0",
            &["da.query_data"],
            &[],
            "INSTRUCTIONS_ONLY_MARKER_9f3c",
        ),
    );

    let discovered = discover_user(root.path()).expect("discovery must succeed");
    assert_eq!(discovered.len(), 1);
    let skill = &discovered[0];
    assert_eq!(skill.manifest().name().as_str(), "charting");
    assert_eq!(skill.source(), SkillSource::User);
    assert!(!skill.content_hash().is_empty());
    // Cataloged metadata never carries the instructions body.
    assert!(!skill
        .manifest()
        .description()
        .contains("INSTRUCTIONS_ONLY_MARKER_9f3c"));
}

// ── registry::SkillRegistry ──────────────────────────────────────────────

#[test]
fn duplicate_skill_names_rejected() {
    let user_root = tempfile::tempdir().expect("skills root");
    let project_root = tempfile::tempdir().expect("project root");

    write_skill(
        user_root.path(),
        "widget",
        &skill_document("widget", "Built-in widget", "1.0.0", &[], &[], "Body."),
    );
    write_skill(
        &project_root.path().join(".xfalcon").join("skills"),
        "widget-clone",
        &skill_document("widget", "Project widget", "1.0.0", &[], &[], "Body."),
    );

    let imported = discover_user(user_root.path()).expect("imported discovery");
    let project =
        discover_project(project_root.path(), TrustState::Trusted).expect("project discovery");

    let mut registry = SkillRegistry::new();
    registry
        .register(imported.into_iter().next().expect("one imported skill"))
        .expect("first registration succeeds");

    let err = registry
        .register(project.into_iter().next().expect("one project skill"))
        .expect_err("duplicate skill name must be rejected");
    assert!(
        matches!(err, SkillRegistryError::DuplicateSkillName { .. }),
        "{err:?}"
    );
}

#[test]
fn conflicting_versions_handled_explicitly() {
    let user_root = tempfile::tempdir().expect("skills root");
    let project_root = tempfile::tempdir().expect("project root");

    write_skill(
        user_root.path(),
        "widget",
        &skill_document("widget", "Built-in widget", "1.0.0", &[], &[], "Body."),
    );
    write_skill(
        &project_root.path().join(".xfalcon").join("skills"),
        "widget-clone",
        &skill_document("widget", "Project widget", "2.0.0", &[], &[], "Body."),
    );

    let imported = discover_user(user_root.path()).expect("imported discovery");
    let project =
        discover_project(project_root.path(), TrustState::Trusted).expect("project discovery");

    let mut registry = SkillRegistry::new();
    registry
        .register(imported.into_iter().next().expect("one imported skill"))
        .expect("first registration succeeds");

    let err = registry
        .register(project.into_iter().next().expect("one project skill"))
        .expect_err("version conflict must be rejected explicitly, not silently");
    match err {
        SkillRegistryError::DuplicateSkillName {
            existing_source,
            existing_version,
            incoming_source,
            incoming_version,
            ..
        } => {
            assert_eq!(existing_source, SkillSource::User);
            assert_eq!(existing_version.as_str(), "1.0.0");
            assert_eq!(incoming_source, SkillSource::Project);
            assert_eq!(incoming_version.as_str(), "2.0.0");
        }
        other => panic!("expected DuplicateSkillName, got {other:?}"),
    }
}

#[test]
fn lazy_relevance_selection_defers_body_load() {
    let root = tempfile::tempdir().expect("skills root");
    write_skill(
        root.path(),
        "alpha",
        &skill_document(
            "alpha",
            "Alpha skill for lazy loading",
            "1.0.0",
            &[],
            &[],
            "ALPHA_INSTRUCTIONS_MARKER_1",
        ),
    );
    write_skill(
        root.path(),
        "beta",
        &skill_document(
            "beta",
            "Beta skill for lazy loading",
            "1.0.0",
            &[],
            &[],
            "BETA_INSTRUCTIONS_MARKER_2",
        ),
    );

    let discovered = discover_user(root.path()).expect("discovery must succeed");
    assert_eq!(discovered.len(), 2);

    let mut registry = SkillRegistry::new();
    for skill in discovered {
        registry.register(skill).expect("registration succeeds");
    }

    // Both skills are cataloged (name/description/version visible) without
    // either instructions marker ever appearing in catalog metadata.
    let catalog: Vec<&DiscoveredSkill> = registry.catalog().collect();
    assert_eq!(catalog.len(), 2);
    for skill in &catalog {
        assert!(!skill.manifest().description().contains("MARKER"));
    }

    let alpha_name = SkillName::new("alpha").expect("valid name");
    let beta_name = SkillName::new("beta").expect("valid name");

    // Selecting alpha loads only alpha's body.
    let selected_alpha = registry
        .select(&SkillSelection::Explicit(alpha_name))
        .expect("alpha selection succeeds");
    assert!(selected_alpha
        .instructions()
        .contains("ALPHA_INSTRUCTIONS_MARKER_1"));

    // Corrupting beta's on-disk file *after* discovery/registration proves
    // its body was never read (and cached) during that pass: beta's
    // metadata is still valid in the catalog above, but selecting it now
    // re-reads the file and observes the corruption.
    write_skill(root.path(), "beta", "not a skill document at all");
    let err = registry
        .select(&SkillSelection::Explicit(beta_name))
        .expect_err("beta's now-corrupted file must fail on lazy load");
    assert!(matches!(
        err,
        SkillRegistryError::Discovery(DiscoveryError::Manifest { .. })
    ));
}

// ── policy::effective_skill_scope (tool escalation) ─────────────────────

#[test]
fn skill_tool_scope_narrows_but_never_expands() {
    let raw = skill_document(
        "charting",
        "Build validated analytical charts",
        "1.0.0",
        &["da.query_data", "da.mutate_secret"],
        &[],
        "Body.",
    );
    let parsed = parse_skill_document(&raw).expect("valid document");

    let mut effective_allowlist = HashSet::new();
    effective_allowlist.insert(ToolId::from("da.query_data"));
    // Deliberately excludes "da.mutate_secret" — global/project policy
    // never granted it.

    let scope = effective_skill_scope(&parsed.manifest, &effective_allowlist);
    assert!(scope.allows(&ToolId::from("da.query_data")));
    assert!(!scope.allows(&ToolId::from("da.mutate_secret")));
}

#[test]
fn tool_escalation_rejected_end_to_end_through_policy_engine() {
    // Cross-module integration proving the narrowed `SkillToolScope`
    // composes correctly with `crate::tools::policy::PolicyEngine`: even
    // though the GLOBAL allowlist grants "da.mutate_secret" (so the
    // allowlist gate alone would not stop it), the skill's own declared
    // scope — narrowed by `effective_skill_scope` against a stricter
    // project-level grant that never included it — still denies the call.
    let raw = skill_document(
        "charting",
        "Build validated analytical charts",
        "1.0.0",
        &["da.query_data"],
        &[],
        "Body.",
    );
    let parsed = parse_skill_document(&raw).expect("valid document");

    let mutate_tool = ToolId::from("da.mutate_secret");
    let query_tool = ToolId::from("da.query_data");

    let mut registry = ToolRegistry::new();
    registry
        .register(
            ToolDescriptor::new(
                mutate_tool.clone(),
                ContractToolSource::Mcp,
                "Mutate Secret",
                "A dangerous tool the skill must not be able to grant itself.",
                json!({ "type": "object", "properties": {} }),
                ToolRisk::Mutating,
                vec![],
                false,
            )
            .expect("valid descriptor"),
        )
        .expect("registered");

    let mut global_allowlist = HashSet::new();
    global_allowlist.insert(mutate_tool.clone());
    global_allowlist.insert(query_tool.clone());

    // The skill only ever declared "da.query_data", so the project-level
    // effective allowlist it is narrowed against never includes
    // "da.mutate_secret" — this is what `effective_skill_scope` enforces.
    let mut project_allowlist = HashSet::new();
    project_allowlist.insert(query_tool);
    let skill_scope = effective_skill_scope(&parsed.manifest, &project_allowlist);
    assert!(
        !skill_scope.allows(&mutate_tool),
        "skill must not gain a tool absent from its narrowed project scope"
    );

    let actor = Actor::new("user-1").expect("valid actor");
    let invocation_id = InvocationId::from("inv-1");
    let arguments = json!({});

    let engine = PolicyEngine::new(&registry, &global_allowlist);
    let request = PolicyRequest {
        actor: Some(&actor),
        invocation_id: &invocation_id,
        tool_id: &mutate_tool,
        arguments: &arguments,
        run_budget_remaining: 10,
        tool_budget_remaining: 10,
        active_project: None,
        requested_path: None,
        path_authority: None,
        skill_scope: Some(&skill_scope),
        approval: None,
    };

    let decision = engine.authorize(&request);
    assert_eq!(
        decision,
        PolicyDecision::Deny(PolicyDenial::OutsideSkillScope(mutate_tool))
    );
}

// ── Boundary-condition coverage for `ManifestError`/`DiscoveryError` ──────
//
// Every constructor in `manifest.rs` bails on the FIRST failing check in a
// fixed order (see that module's own doc comments), so each test below
// builds an input that is valid along every dimension except the one under
// test -- otherwise a test meant to prove variant X could actually be
// proving an earlier-checked variant Y instead. Two kinds of check exist
// here:
//
//   - Field-level checks (name/description/version/tool-id/path length and
//     content, and the two list-length caps) run AFTER the frontmatter has
//     already deserialized into a well-formed `RawFrontmatter`, so the
//     "valid" (N) side of each boundary pair is a genuinely well-formed
//     `SKILL.md` document that parses all the way to `Ok(_)`.
//   - Whole-document/frontmatter *byte-size* checks (`DocumentTooLarge`,
//     `FrontmatterTooLarge`, and discovery's `FileTooLarge`) run BEFORE the
//     YAML is even parsed, so their "valid" (N) side only proves that
//     specific size check passes at exactly the limit; the filler content
//     used to hit an exact byte count is not a well-formed manifest, so the
//     valid side is asserted against the specific *next* error the document
//     legitimately fails on, never against `Ok(_)`.
#[cfg(test)]
mod boundary_tests {
    use super::{skill_document, write_skill};
    use crate::skills::discovery::{discover_user, DiscoveryError};
    use crate::skills::manifest::{
        parse_skill_document, ManifestError, MAX_ALLOWED_PATHS, MAX_ALLOWED_TOOLS,
        MAX_DESCRIPTION_BYTES, MAX_DOCUMENT_BYTES, MAX_FRONTMATTER_BYTES, MAX_NAME_BYTES,
        MAX_PATH_BYTES, MAX_VERSION_BYTES,
    };

    // ── Document/frontmatter byte-size ceilings ─────────────────────────

    #[test]
    fn document_too_large_boundary() {
        // `DocumentTooLarge` runs on `raw.len()` before any `---`-delimiter
        // split, so filler content need not be a valid document -- only its
        // byte length matters. At exactly the limit, the size check passes
        // and the (still-malformed) filler legitimately fails the very next
        // check instead (`MissingFrontmatterDelimiters`, since a single
        // line of "a"s never starts with "---").
        let at_limit = "a".repeat(MAX_DOCUMENT_BYTES);
        assert_eq!(at_limit.len(), MAX_DOCUMENT_BYTES);
        let err = parse_skill_document(&at_limit).expect_err("filler is not a valid document");
        assert!(
            matches!(err, ManifestError::MissingFrontmatterDelimiters),
            "expected the size check to pass at the exact limit: {err:?}"
        );

        let over_limit = "a".repeat(MAX_DOCUMENT_BYTES + 1);
        let err =
            parse_skill_document(&over_limit).expect_err("oversized document must be rejected");
        assert!(
            matches!(err, ManifestError::DocumentTooLarge(limit) if limit == MAX_DOCUMENT_BYTES),
            "{err:?}"
        );
    }

    #[test]
    fn missing_frontmatter_delimiters_rejected() {
        // No opening delimiter at all.
        let no_opening = "name: charting\ndescription: Build charts\nversion: 1.0.0\n";
        let err = parse_skill_document(no_opening)
            .expect_err("missing opening delimiter must be rejected");
        assert!(
            matches!(err, ManifestError::MissingFrontmatterDelimiters),
            "{err:?}"
        );

        // Opens but is never closed.
        let never_closes = "---\nname: charting\ndescription: Build charts\nversion: 1.0.0\n";
        let err = parse_skill_document(never_closes)
            .expect_err("unterminated frontmatter must be rejected");
        assert!(
            matches!(err, ManifestError::MissingFrontmatterDelimiters),
            "{err:?}"
        );
    }

    #[test]
    fn frontmatter_too_large_boundary() {
        // `FrontmatterTooLarge` runs on the split-out frontmatter's byte
        // length *before* `serde_saphyr::from_str` ever sees it, so (as
        // above) filler content that is not valid YAML for `RawFrontmatter`
        // is fine for pinning the size boundary itself.
        let build = |frontmatter_len: usize| -> String {
            format!("---\n{}\n---\nBody.\n", "a".repeat(frontmatter_len))
        };

        let at_limit = build(MAX_FRONTMATTER_BYTES);
        let err =
            parse_skill_document(&at_limit).expect_err("filler frontmatter is not valid YAML");
        assert!(
            matches!(err, ManifestError::MalformedYaml(_)),
            "expected the size check to pass at the exact limit: {err:?}"
        );

        let over_limit = build(MAX_FRONTMATTER_BYTES + 1);
        let err =
            parse_skill_document(&over_limit).expect_err("oversized frontmatter must be rejected");
        assert!(
            matches!(err, ManifestError::FrontmatterTooLarge(limit) if limit == MAX_FRONTMATTER_BYTES),
            "{err:?}"
        );
    }

    // ── `SkillName` ──────────────────────────────────────────────────────

    #[test]
    fn empty_name_rejected() {
        // The plain (unquoted) `skill_document` helper cannot express an
        // empty scalar (`name: ` parses as YAML null, not an empty string),
        // so this needs an explicitly quoted `""` in raw YAML.
        let raw = "---\nname: \"\"\ndescription: Valid description.\nversion: 1.0.0\n---\nBody.\n";
        let err = parse_skill_document(raw).expect_err("empty name must be rejected");
        assert!(matches!(err, ManifestError::EmptyName), "{err:?}");
    }

    #[test]
    fn name_too_long_boundary() {
        let at_limit = "a".repeat(MAX_NAME_BYTES);
        let raw = skill_document(&at_limit, "Valid description.", "1.0.0", &[], &[], "Body.");
        let parsed = parse_skill_document(&raw).expect("name at the exact limit must parse");
        assert_eq!(parsed.manifest.name().as_str(), at_limit);

        let over_limit = "a".repeat(MAX_NAME_BYTES + 1);
        let raw = skill_document(
            &over_limit,
            "Valid description.",
            "1.0.0",
            &[],
            &[],
            "Body.",
        );
        let err = parse_skill_document(&raw).expect_err("over-limit name must be rejected");
        assert!(
            matches!(err, ManifestError::NameTooLong(limit) if limit == MAX_NAME_BYTES),
            "{err:?}"
        );
    }

    #[test]
    fn a_name_is_normalized_to_the_slug_charset() {
        // A capital letter or a space is a formatting difference, not a
        // reason to refuse a user's skill -- and this is also the directory
        // it is stored under, so normalizing here is what keeps the declared
        // name and the stored path the same string.
        for (declared, expected) in [
            ("Chart", "chart"),
            ("chart_ing", "chart-ing"),
            ("-leading", "leading"),
            ("trailing-", "trailing"),
            ("double--hyphen", "double-hyphen"),
            ("My Skill", "my-skill"),
        ] {
            let raw = skill_document(declared, "Valid description.", "1.0.0", &[], &[], "Body.");
            let parsed = parse_skill_document(&raw)
                .unwrap_or_else(|err| panic!("{declared:?} should normalize, got {err:?}"));
            assert_eq!(parsed.manifest.name().as_str(), expected, "{declared:?}");
        }
    }

    #[test]
    fn a_name_with_nothing_to_slugify_is_still_rejected() {
        // There is no directory name to store it under. Asserted against the
        // constructor rather than a document, because YAML resolves a
        // punctuation-only scalar before this type ever sees it.
        let err = super::super::manifest::SkillName::new("!!!")
            .expect_err("punctuation-only name must be rejected");
        assert!(matches!(err, ManifestError::InvalidNameCharset), "{err:?}");
    }

    // ── `description` ────────────────────────────────────────────────────

    #[test]
    fn empty_description_rejected() {
        let raw = "---\nname: boundary\ndescription: \"\"\nversion: 1.0.0\n---\nBody.\n";
        let err = parse_skill_document(raw).expect_err("empty description must be rejected");
        assert!(matches!(err, ManifestError::EmptyDescription), "{err:?}");
    }

    #[test]
    fn description_too_long_boundary() {
        let at_limit = "a".repeat(MAX_DESCRIPTION_BYTES);
        let raw = skill_document("boundary", &at_limit, "1.0.0", &[], &[], "Body.");
        let parsed = parse_skill_document(&raw).expect("description at the exact limit must parse");
        assert_eq!(parsed.manifest.description(), at_limit);

        let over_limit = "a".repeat(MAX_DESCRIPTION_BYTES + 1);
        let raw = skill_document("boundary", &over_limit, "1.0.0", &[], &[], "Body.");
        let err = parse_skill_document(&raw).expect_err("over-limit description must be rejected");
        assert!(
            matches!(err, ManifestError::DescriptionTooLong(limit) if limit == MAX_DESCRIPTION_BYTES),
            "{err:?}"
        );
    }

    // ── `SkillVersion` ───────────────────────────────────────────────────

    #[test]
    fn empty_version_rejected() {
        let raw =
            "---\nname: boundary\ndescription: Valid description.\nversion: \"\"\n---\nBody.\n";
        let err = parse_skill_document(raw).expect_err("empty version must be rejected");
        assert!(matches!(err, ManifestError::EmptyVersion), "{err:?}");
    }

    #[test]
    fn version_too_long_boundary() {
        // `MAJOR.MINOR.PATCH`, all-digit parts, padded to land exactly on
        // the byte boundary: "1.0." (4 bytes) + N zeros.
        let at_limit = format!("1.0.{}", "0".repeat(MAX_VERSION_BYTES - 4));
        assert_eq!(at_limit.len(), MAX_VERSION_BYTES);
        let raw = skill_document(
            "boundary",
            "Valid description.",
            &at_limit,
            &[],
            &[],
            "Body.",
        );
        let parsed = parse_skill_document(&raw).expect("version at the exact limit must parse");
        assert_eq!(parsed.manifest.version().as_str(), at_limit);

        // One byte over: still a syntactically valid `MAJOR.MINOR.PATCH`
        // triple, proving this is genuinely the length check firing, not
        // `InvalidVersion`.
        let over_limit = format!("1.0.{}", "0".repeat(MAX_VERSION_BYTES - 3));
        let raw = skill_document(
            "boundary",
            "Valid description.",
            &over_limit,
            &[],
            &[],
            "Body.",
        );
        let err = parse_skill_document(&raw).expect_err("over-limit version must be rejected");
        assert!(
            matches!(err, ManifestError::VersionTooLong(limit) if limit == MAX_VERSION_BYTES),
            "{err:?}"
        );
    }

    #[test]
    fn invalid_version_rejected() {
        for bad_version in ["1.0", "1.0.0.0", "1.a.0", "1..0", "v1.0.0"] {
            let raw = skill_document(
                "boundary",
                "Valid description.",
                bad_version,
                &[],
                &[],
                "Body.",
            );
            let err = parse_skill_document(&raw).unwrap_err();
            match err {
                ManifestError::InvalidVersion(ref v) => assert_eq!(v.as_str(), bad_version),
                other => panic!("{bad_version:?} -> expected InvalidVersion, got {other:?}"),
            }
        }
    }

    // ── `allowed_tools` ──────────────────────────────────────────────────

    #[test]
    fn too_many_allowed_tools_boundary() {
        let at_limit: Vec<String> = (0..MAX_ALLOWED_TOOLS)
            .map(|i| format!("tool-{i}"))
            .collect();
        let at_limit_refs: Vec<&str> = at_limit.iter().map(String::as_str).collect();
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &at_limit_refs,
            &[],
            "Body.",
        );
        let parsed = parse_skill_document(&raw).expect("exactly the max tool count must parse");
        assert_eq!(parsed.manifest.allowed_tools().len(), MAX_ALLOWED_TOOLS);

        let over_limit: Vec<String> = (0..=MAX_ALLOWED_TOOLS)
            .map(|i| format!("tool-{i}"))
            .collect();
        let over_limit_refs: Vec<&str> = over_limit.iter().map(String::as_str).collect();
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &over_limit_refs,
            &[],
            "Body.",
        );
        let err = parse_skill_document(&raw).expect_err("over-limit tool count must be rejected");
        assert!(
            matches!(err, ManifestError::TooManyAllowedTools(limit) if limit == MAX_ALLOWED_TOOLS),
            "{err:?}"
        );
    }

    #[test]
    fn empty_tool_id_rejected() {
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &[""],
            &[],
            "Body.",
        );
        let err = parse_skill_document(&raw).expect_err("empty tool id must be rejected");
        assert!(matches!(err, ManifestError::EmptyToolId), "{err:?}");
    }

    #[test]
    fn tool_id_too_long_boundary() {
        let at_limit = "a".repeat(MAX_NAME_BYTES);
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &[at_limit.as_str()],
            &[],
            "Body.",
        );
        let parsed = parse_skill_document(&raw).expect("tool id at the exact limit must parse");
        assert_eq!(parsed.manifest.allowed_tools().len(), 1);

        let over_limit = "a".repeat(MAX_NAME_BYTES + 1);
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &[over_limit.as_str()],
            &[],
            "Body.",
        );
        let err = parse_skill_document(&raw).expect_err("over-limit tool id must be rejected");
        assert!(
            matches!(err, ManifestError::ToolIdTooLong(limit) if limit == MAX_NAME_BYTES),
            "{err:?}"
        );
    }

    // ── `allowed_project_paths` ──────────────────────────────────────────

    #[test]
    fn too_many_allowed_paths_boundary() {
        let at_limit: Vec<String> = (0..MAX_ALLOWED_PATHS)
            .map(|i| format!("path-{i}"))
            .collect();
        let at_limit_refs: Vec<&str> = at_limit.iter().map(String::as_str).collect();
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &[],
            &at_limit_refs,
            "Body.",
        );
        let parsed = parse_skill_document(&raw).expect("exactly the max path count must parse");
        assert_eq!(
            parsed.manifest.allowed_project_paths().len(),
            MAX_ALLOWED_PATHS
        );

        let over_limit: Vec<String> = (0..=MAX_ALLOWED_PATHS)
            .map(|i| format!("path-{i}"))
            .collect();
        let over_limit_refs: Vec<&str> = over_limit.iter().map(String::as_str).collect();
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &[],
            &over_limit_refs,
            "Body.",
        );
        let err = parse_skill_document(&raw).expect_err("over-limit path count must be rejected");
        assert!(
            matches!(err, ManifestError::TooManyAllowedPaths(limit) if limit == MAX_ALLOWED_PATHS),
            "{err:?}"
        );
    }

    #[test]
    fn empty_path_rejected() {
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &[],
            &[""],
            "Body.",
        );
        let err = parse_skill_document(&raw).expect_err("empty project path must be rejected");
        assert!(matches!(err, ManifestError::EmptyPath), "{err:?}");
    }

    #[test]
    fn path_too_long_boundary() {
        let at_limit = "a".repeat(MAX_PATH_BYTES);
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &[],
            &[at_limit.as_str()],
            "Body.",
        );
        let parsed = parse_skill_document(&raw).expect("path at the exact limit must parse");
        assert_eq!(parsed.manifest.allowed_project_paths().len(), 1);

        let over_limit = "a".repeat(MAX_PATH_BYTES + 1);
        let raw = skill_document(
            "boundary",
            "Valid description.",
            "1.0.0",
            &[],
            &[over_limit.as_str()],
            "Body.",
        );
        let err = parse_skill_document(&raw).expect_err("over-limit path must be rejected");
        assert!(
            matches!(err, ManifestError::PathTooLong(limit) if limit == MAX_PATH_BYTES),
            "{err:?}"
        );
    }

    #[test]
    fn absolute_path_rejected() {
        for bad_path in ["/etc/passwd", "C:\\Users\\attacker", "\\\\server\\share"] {
            let raw = skill_document(
                "boundary",
                "Valid description.",
                "1.0.0",
                &[],
                &[bad_path],
                "Body.",
            );
            let err = parse_skill_document(&raw).unwrap_err();
            match err {
                ManifestError::AbsolutePath(ref p) => assert_eq!(p.as_str(), bad_path),
                other => panic!("{bad_path:?} -> expected AbsolutePath, got {other:?}"),
            }
        }
    }

    // ── Discovery: per-file and per-source byte/count ceilings ──────────

    #[test]
    fn file_too_large_boundary() {
        // Mirrors `discovery::MAX_SKILL_FILE_BYTES` (256 * 1024): private to
        // that module (not `pub`), so hardcoded here with this comment as
        // the cross-reference.
        const MAX_SKILL_FILE_BYTES: u64 = 256 * 1024;

        let root = tempfile::tempdir().expect("skills root");
        write_skill(
            root.path(),
            "at-limit",
            &"a".repeat(MAX_SKILL_FILE_BYTES as usize),
        );
        // The size check runs on file metadata *before* the bytes are read
        // and parsed, so filler content that is not a valid `SKILL.md`
        // document is fine for pinning the boundary: at exactly the limit
        // it is read and legitimately rejected as an unparseable manifest
        // instead (this filler is also far larger than `MAX_DOCUMENT_BYTES`,
        // so the wrapped error is `ManifestError::DocumentTooLarge`, but the
        // discovery-level variant under test here is `Manifest`, not
        // `FileTooLarge`).
        let err = discover_user(root.path()).expect_err("filler skill is not a valid manifest");
        assert!(matches!(err, DiscoveryError::Manifest { .. }), "{err:?}");

        let root = tempfile::tempdir().expect("skills root");
        write_skill(
            root.path(),
            "over-limit",
            &"a".repeat(MAX_SKILL_FILE_BYTES as usize + 1),
        );
        let err = discover_user(root.path()).expect_err("oversized skill file must be rejected");
        assert!(
            matches!(err, DiscoveryError::FileTooLarge { limit, .. } if limit == MAX_SKILL_FILE_BYTES),
            "{err:?}"
        );
    }

    #[test]
    fn too_many_skills_boundary() {
        // Mirrors `discovery::MAX_SKILLS_PER_SOURCE` (256): private to that
        // module (not `pub`), so hardcoded here with this comment as the
        // cross-reference. The count check runs on the subdirectory listing
        // *before* any `SKILL.md` is opened, so empty directories (no
        // manifest file at all) are enough to pin the boundary.
        const MAX_SKILLS_PER_SOURCE: usize = 256;

        let root = tempfile::tempdir().expect("skills root");
        for i in 0..MAX_SKILLS_PER_SOURCE {
            std::fs::create_dir_all(root.path().join(format!("skill-{i}")))
                .expect("create skill dir");
        }
        let discovered =
            discover_user(root.path()).expect("exactly the max skill count must not be rejected");
        assert!(
            discovered.is_empty(),
            "no SKILL.md files were ever written, so nothing should be cataloged"
        );

        std::fs::create_dir_all(root.path().join(format!("skill-{MAX_SKILLS_PER_SOURCE}")))
            .expect("create one more skill dir");
        let err = discover_user(root.path()).expect_err("over-limit skill count must be rejected");
        assert!(
            matches!(err, DiscoveryError::TooManySkills(limit) if limit == MAX_SKILLS_PER_SOURCE),
            "{err:?}"
        );
    }
}

// ── store: the imported skill library (spec: user-skills) ────────────────

mod store_tests {
    use std::path::PathBuf;

    use super::super::store::{SkillStore, StoredSkill};

    /// The shape every skill in the `data-analyst` repository has: a name, a
    /// description, and nothing else.
    const IDA_SHAPED: &str =
        "---\nname: ida-dashboard\ndescription: \"Build dashboards.\"\n---\n# Dashboard\n\nBody.\n";

    fn account() -> crate::security::account_cleanup::ValidatedAccountId {
        crate::security::account_cleanup::ValidatedAccountId::parse("skills-account")
            .expect("account")
    }

    fn store() -> (tempfile::TempDir, SkillStore) {
        let root = tempfile::tempdir().expect("app local data");
        let store = SkillStore::open(root.path(), &account());
        (root, store)
    }

    fn chosen(root: &tempfile::TempDir, name: &str, contents: &str) -> PathBuf {
        let path = root.path().join(name);
        std::fs::write(&path, contents).expect("seed chosen file");
        path
    }

    #[test]
    fn a_fresh_install_has_an_empty_catalog() {
        let (_root, store) = store();
        assert!(store.list().expect("list").is_empty());
        assert!(store.enabled().expect("enabled").is_empty());
    }

    #[test]
    fn an_ida_shaped_skill_imports_and_is_stored_verbatim() {
        let (root, store) = store();
        let source = chosen(&root, "ida-dashboard.md", IDA_SHAPED);

        let outcome = store.import(&[source]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(outcome.imported.len(), 1);
        let skill = &outcome.imported[0];
        assert_eq!(skill.name(), "ida-dashboard");
        assert_eq!(skill.discovered.manifest().version().as_str(), "0.0.0");
        assert!(skill.enabled);

        // The stored file is the file the user chose. Re-serializing our
        // reading of it would drop the keys this parser ignores and make the
        // content hash describe our rendering instead of their document.
        let stored = std::fs::read_to_string(store.root().join("ida-dashboard").join("SKILL.md"))
            .expect("stored file");
        assert_eq!(stored, IDA_SHAPED);
    }

    #[test]
    fn a_declared_name_is_stored_under_its_slug() {
        let (root, store) = store();
        let source = chosen(
            &root,
            "mixed.md",
            "---\nname: My Skill\ndescription: \"Mine.\"\n---\nBody.\n",
        );
        let outcome = store.import(&[source]);
        assert_eq!(outcome.imported[0].name(), "my-skill");
        assert!(store.root().join("my-skill").join("SKILL.md").is_file());
    }

    #[test]
    fn a_malformed_file_is_reported_and_its_siblings_still_import() {
        let (root, store) = store();
        let good = chosen(&root, "good.md", IDA_SHAPED);
        let bad = chosen(&root, "bad.md", "no frontmatter here at all\n");

        let outcome = store.import(&[good, bad.clone()]);
        assert_eq!(outcome.imported.len(), 1);
        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].path, bad);
        assert!(
            outcome.failed[0].reason.contains("delimiters"),
            "the reason must name the actual problem: {:?}",
            outcome.failed[0].reason
        );
    }

    #[test]
    fn re_importing_replaces_the_content_and_keeps_the_disabled_state() {
        let (root, store) = store();
        let first = chosen(&root, "first.md", IDA_SHAPED);
        store.import(&[first]);
        store.set_enabled("ida-dashboard", false).expect("disable");

        let edited = IDA_SHAPED.replace("Body.", "Edited body.");
        let second = chosen(&root, "second.md", &edited);
        let outcome = store.import(&[second]);

        assert_eq!(outcome.imported.len(), 1);
        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 1, "name is identity: one entry, not two");
        assert!(
            !listed[0].enabled,
            "the user's decision survives an update to the content"
        );
        let stored = std::fs::read_to_string(store.root().join("ida-dashboard").join("SKILL.md"))
            .expect("stored file");
        assert_eq!(stored, edited);
    }

    #[test]
    fn a_disabled_skill_reaches_no_run_and_cannot_be_fetched() {
        let (root, store) = store();
        let source = chosen(&root, "ida-dashboard.md", IDA_SHAPED);
        store.import(&[source]);

        store.set_enabled("ida-dashboard", false).expect("disable");
        assert!(store.enabled().expect("enabled").is_empty());
        assert_eq!(
            store.list().expect("list").len(),
            1,
            "disabled is not deleted"
        );
        store
            .instructions_and_files("ida-dashboard")
            .expect_err("a skill the model was never told about must not be readable");

        // Re-read through a second store proves it persisted, not cached.
        let reopened = SkillStore::open(root.path(), &account());
        assert!(reopened.enabled().expect("enabled").is_empty());

        reopened.set_enabled("ida-dashboard", true).expect("enable");
        assert_eq!(reopened.enabled().expect("enabled").len(), 1);
        assert!(reopened
            .instructions_and_files("ida-dashboard")
            .expect("body")
            .0
            .contains("Body."));
    }

    #[test]
    fn a_body_is_fetched_by_a_name_normalized_the_way_import_normalized_it() {
        // The model reads `ida-dashboard` out of <active_skills>, but a user
        // typing `IDA Dashboard` means the same skill.
        let (root, store) = store();
        let source = chosen(&root, "ida-dashboard.md", IDA_SHAPED);
        store.import(&[source]);
        assert!(store.instructions_and_files("IDA Dashboard").is_ok());
    }

    #[test]
    fn an_unknown_skill_is_refused_rather_than_answered_empty() {
        let (_root, store) = store();
        store
            .instructions_and_files("nothing-like-this")
            .expect_err("an empty body would read to the model as a skill that says nothing");
    }

    #[test]
    fn deleting_removes_the_directory_and_forgets_the_disabled_state() {
        let (root, store) = store();
        let source = chosen(&root, "ida-dashboard.md", IDA_SHAPED);
        store.import(&[source]);
        store.set_enabled("ida-dashboard", false).expect("disable");

        store.delete("ida-dashboard").expect("delete");
        assert!(store.list().expect("list").is_empty());
        assert!(!store.root().join("ida-dashboard").exists());

        // A later import of the same name starts enabled.
        let again = chosen(&root, "again.md", IDA_SHAPED);
        let outcome = store.import(&[again]);
        assert!(outcome.imported[0].enabled);
    }

    #[test]
    fn a_chosen_folder_is_searched_including_its_subfolders() {
        // A skill library is a folder of skill folders, so importing one file
        // at a time is the wrong unit.
        let (root, store) = store();
        let library = root.path().join("library");
        std::fs::create_dir_all(library.join("nested").join("deeper")).expect("tree");
        std::fs::write(library.join("top.md"), IDA_SHAPED).expect("top");
        std::fs::write(
            library.join("nested").join("SKILL.md"),
            IDA_SHAPED.replace("ida-dashboard", "ida-explore"),
        )
        .expect("nested");
        std::fs::write(
            library.join("nested").join("deeper").join("deep.skill"),
            IDA_SHAPED.replace("ida-dashboard", "ida-stats"),
        )
        .expect("deep");
        // Neither of these is a skill, and neither is an error: they were
        // found, not chosen.
        std::fs::write(library.join("README.md"), "# Not a skill\n").expect("readme");
        std::fs::write(library.join("notes.txt"), "ignored").expect("txt");

        let outcome = store.import(&[library]);
        let mut names: Vec<&str> = outcome.imported.iter().map(StoredSkill::name).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["ida-dashboard", "ida-explore", "ida-stats"]);
        assert!(
            outcome.failed.is_empty(),
            "a README beside three skills is not a failure: {:?}",
            outcome.failed
        );
    }

    #[test]
    fn a_folder_holding_no_skill_says_so_rather_than_looking_successful() {
        let (root, store) = store();
        let empty = root.path().join("empty");
        std::fs::create_dir_all(&empty).expect("dir");
        std::fs::write(empty.join("README.md"), "# Nothing here\n").expect("readme");

        let outcome = store.import(std::slice::from_ref(&empty));
        assert!(outcome.imported.is_empty());
        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].path, empty);
    }

    #[test]
    fn a_zip_yields_every_skill_file_it_holds_at_any_depth() {
        let (root, store) = store();
        let archive_path = root.path().join("library.zip");
        {
            let file = std::fs::File::create(&archive_path).expect("archive");
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for (name, body) in [
                ("skills/ida-dashboard/SKILL.md", IDA_SHAPED.to_owned()),
                (
                    "skills/ida-explore/Skill.md",
                    IDA_SHAPED.replace("ida-dashboard", "ida-explore"),
                ),
                ("skills/README.md", "# Not a skill\n".to_owned()),
                ("skills/logo.png", "binary".to_owned()),
            ] {
                std::io::Write::write_all(
                    {
                        writer.start_file(name, options).expect("entry");
                        &mut writer
                    },
                    body.as_bytes(),
                )
                .expect("write entry");
            }
            writer.finish().expect("finish");
        }

        let outcome = store.import(&[archive_path]);
        let mut names: Vec<&str> = outcome.imported.iter().map(StoredSkill::name).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["ida-dashboard", "ida-explore"]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

        // Stored under the manifest's own slug, never under the entry's path:
        // an archive cannot choose where its contents land.
        assert!(store
            .root()
            .join("ida-dashboard")
            .join("SKILL.md")
            .is_file());
        assert!(!store.root().join("skills").exists());
    }

    #[test]
    fn a_zip_entry_that_would_escape_the_root_lands_under_its_slug_like_any_other() {
        let (root, store) = store();
        let archive_path = root.path().join("hostile.zip");
        {
            let file = std::fs::File::create(&archive_path).expect("archive");
            let mut writer = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            writer
                .start_file("../../../../evil.md", options)
                .expect("entry");
            std::io::Write::write_all(&mut writer, IDA_SHAPED.as_bytes()).expect("write");
            writer.finish().expect("finish");
        }

        let outcome = store.import(&[archive_path]);
        assert_eq!(outcome.imported.len(), 1);
        assert_eq!(outcome.imported[0].name(), "ida-dashboard");
        assert!(store
            .root()
            .join("ida-dashboard")
            .join("SKILL.md")
            .is_file());
        assert!(!root.path().join("evil.md").exists());
    }

    #[test]
    fn two_files_declaring_one_name_import_as_one_skill() {
        // Name is identity, so the second file replaced the first. Counting
        // both would tell the user two skills arrived where one did.
        let (root, store) = store();
        let library = root.path().join("library");
        std::fs::create_dir_all(library.join("v2")).expect("tree");
        std::fs::write(library.join("first.md"), IDA_SHAPED).expect("first");
        std::fs::write(
            library.join("v2").join("second.md"),
            IDA_SHAPED.replace("Build dashboards.", "Build better dashboards."),
        )
        .expect("second");

        let outcome = store.import(&[library]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(
            outcome.imported.len(),
            1,
            "one name is one skill: {:?}",
            outcome
                .imported
                .iter()
                .map(StoredSkill::name)
                .collect::<Vec<_>>()
        );
        assert_eq!(outcome.imported[0].name(), "ida-dashboard");
        // The reported entry describes the file that is actually on disk --
        // the last one stored, not the first one seen.
        let stored = std::fs::read_to_string(store.root().join("ida-dashboard").join("SKILL.md"))
            .expect("stored file");
        assert!(stored.contains("Build better dashboards."));
        assert_eq!(
            outcome.imported[0].description(),
            "Build better dashboards."
        );
        assert_eq!(store.list().expect("list").len(), 1);
    }

    #[test]
    fn a_file_the_user_named_still_reports_why_it_failed() {
        // The distinction the reporting rests on: a named file is owed its
        // reason, a found one is not.
        let (root, store) = store();
        let bad = chosen(&root, "bad.md", "no frontmatter here at all\n");
        let outcome = store.import(std::slice::from_ref(&bad));
        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].path, bad);
        assert!(outcome.failed[0].reason.contains("delimiters"));
    }

    #[test]
    fn each_account_sees_and_runs_with_only_its_own_skills() {
        let (root, first) = store();
        let second_account =
            crate::security::account_cleanup::ValidatedAccountId::parse("another-account")
                .expect("account");
        let second = SkillStore::open(root.path(), &second_account);

        let source = chosen(&root, "ida-dashboard.md", IDA_SHAPED);
        assert_eq!(first.import(&[source]).imported.len(), 1);

        assert_eq!(first.list().expect("list").len(), 1);
        assert!(second.list().expect("list").is_empty());
        assert!(second.enabled().expect("enabled").is_empty());
        assert!(
            second.instructions_and_files("ida-dashboard").is_err(),
            "another account's skill must not be fetchable by name"
        );
        assert_ne!(first.root(), second.root());
        assert!(first.root().starts_with(root.path().join("accounts")));
    }

    #[test]
    fn the_device_wide_library_moves_whole_into_the_first_account_to_open_one() {
        // The layout every earlier version wrote: `<app_local_data>/skills`.
        let root = tempfile::tempdir().expect("app local data");
        let device = root.path().join("skills");
        std::fs::create_dir_all(device.join("ida-dashboard")).expect("legacy skill");
        std::fs::write(device.join("ida-dashboard").join("SKILL.md"), IDA_SHAPED)
            .expect("legacy manifest");
        std::fs::write(
            device.join("state.json"),
            r#"{"disabled":["ida-dashboard"]}"#,
        )
        .expect("legacy state");

        let first = SkillStore::open(root.path(), &account());
        let listed = first.list().expect("list");
        assert_eq!(listed.len(), 1, "the library arrived");
        assert!(
            !listed[0].enabled,
            "and the user's decisions arrived with it"
        );
        assert!(!device.exists(), "moved, not copied");

        let second_account =
            crate::security::account_cleanup::ValidatedAccountId::parse("another-account")
                .expect("account");
        let second = SkillStore::open(root.path(), &second_account);
        assert!(
            second.list().expect("list").is_empty(),
            "only one account inherits the shared library"
        );
    }

    #[test]
    fn a_corrupt_state_file_hides_no_skill() {
        let (root, store) = store();
        let source = chosen(&root, "ida-dashboard.md", IDA_SHAPED);
        store.import(&[source]);
        std::fs::write(store.root().join("state.json"), b"{ not json").expect("corrupt");
        assert_eq!(store.enabled().expect("enabled").len(), 1);
    }

    // ── .skill packages and bundled files ────────────────────────────────

    const VIZ_SHAPED: &str = "---\nname: ida-viz\ndescription: \"Chart guidance.\"\n---\n# Viz\n\nRead `references/palette.md` first.\n";

    /// Writes a zip holding `entries`, the way the named packager would.
    fn zip_at(path: &std::path::Path, entries: &[(&str, &[u8])], method: zip::CompressionMethod) {
        let file = std::fs::File::create(path).expect("archive");
        let mut writer = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default().compression_method(method);
        for (name, body) in entries {
            if name.ends_with('/') {
                writer
                    .add_directory(*name, options)
                    .expect("directory entry");
                continue;
            }
            writer.start_file(*name, options).expect("entry");
            std::io::Write::write_all(&mut writer, body).expect("write entry");
        }
        writer.finish().expect("finish");
    }

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let directory = tempfile::tempdir().expect("scratch");
        let path = directory.path().join("inner.zip");
        zip_at(&path, entries, zip::CompressionMethod::Deflated);
        std::fs::read(path).expect("zip bytes")
    }

    fn stored(store: &SkillStore, relative: &str) -> Option<Vec<u8>> {
        std::fs::read(store.root().join(relative)).ok()
    }

    #[test]
    fn a_skill_creator_package_imports_with_every_bundled_file() {
        // `package_skill.py`'s layout: one top-level folder named for the
        // skill, DEFLATE, files only. The file that used to be read as text
        // and rejected for having no frontmatter delimiters.
        let (root, store) = store();
        let package = root.path().join("ida-viz.skill");
        zip_at(
            &package,
            &[
                ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                ("ida-viz/references/palette.md", b"# Palette\n"),
                ("ida-viz/scripts/render.py", b"print('chart')\n"),
                (
                    "ida-viz/assets/logo.png",
                    &[0x89, b'P', b'N', b'G', 0, 0xFF],
                ),
            ],
            zip::CompressionMethod::Deflated,
        );

        let outcome = store.import(std::slice::from_ref(&package));
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(outcome.imported.len(), 1);
        assert_eq!(outcome.imported[0].name(), "ida-viz");
        assert_eq!(
            stored(&store, "ida-viz/SKILL.md").as_deref(),
            Some(VIZ_SHAPED.as_bytes())
        );
        assert_eq!(
            stored(&store, "ida-viz/references/palette.md").as_deref(),
            Some(&b"# Palette\n"[..])
        );
        assert_eq!(
            stored(&store, "ida-viz/assets/logo.png").as_deref(),
            Some(&[0x89, b'P', b'N', b'G', 0, 0xFF][..]),
            "a binary asset is stored byte for byte"
        );
        let files: Vec<String> = store
            .bundled_files(&outcome.imported[0])
            .expect("files")
            .into_iter()
            .map(|file| file.path)
            .collect();
        assert_eq!(
            files,
            vec![
                "assets/logo.png",
                "references/palette.md",
                "scripts/render.py"
            ]
        );
        assert!(
            store
                .instructions_and_files("ida-viz")
                .expect("body")
                .0
                .contains("references/palette.md"),
            "the instructions still come from SKILL.md alone"
        );
    }

    #[test]
    fn a_package_with_skill_md_at_its_root_imports_the_same_way() {
        // The other layout in circulation (and the one the Claude API accepts
        // besides the folder form), stored uncompressed with directory
        // entries, as a desktop zip tool writes it.
        let (root, store) = store();
        let package = root.path().join("falcon-branding.skill");
        zip_at(
            &package,
            &[
                ("references/", b""),
                ("SKILL.md", VIZ_SHAPED.as_bytes()),
                ("references/palette.md", b"# Palette\n"),
            ],
            zip::CompressionMethod::Stored,
        );

        let outcome = store.import(&[package]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(outcome.imported.len(), 1);
        assert!(stored(&store, "ida-viz/references/palette.md").is_some());
    }

    #[test]
    fn a_plain_text_skill_file_still_imports() {
        // The older `.skill` spelling: Markdown with frontmatter, not a zip.
        let (root, store) = store();
        let source = chosen(&root, "ida-dashboard.skill", IDA_SHAPED);
        let outcome = store.import(&[source]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(outcome.imported[0].name(), "ida-dashboard");
        assert!(store
            .bundled_files(&outcome.imported[0])
            .expect("files")
            .is_empty());
    }

    #[test]
    fn a_broken_package_names_its_own_reason() {
        // A `.skill` is one skill by definition, so its SKILL.md failing is
        // owed its reason rather than a generic "nothing here parsed".
        let (root, store) = store();
        let package = root.path().join("broken.skill");
        zip_at(
            &package,
            &[("broken/SKILL.md", b"no frontmatter here at all\n")],
            zip::CompressionMethod::Deflated,
        );
        let outcome = store.import(&[package]);
        assert!(outcome.imported.is_empty());
        assert_eq!(outcome.failed.len(), 1, "{:?}", outcome.failed);
        assert!(outcome.failed[0]
            .path
            .to_string_lossy()
            .ends_with("broken.skill!broken/SKILL.md"));
        assert!(
            outcome.failed[0].reason.contains("delimiters"),
            "{:?}",
            outcome.failed[0]
        );
    }

    #[test]
    fn a_file_named_skill_that_is_not_a_zip_or_a_skill_is_reported() {
        let (root, store) = store();
        let source = root.path().join("junk.skill");
        std::fs::write(&source, [0_u8, 159, 146, 150]).expect("seed");
        let outcome = store.import(std::slice::from_ref(&source));
        assert!(outcome.imported.is_empty());
        assert_eq!(outcome.failed.len(), 1);
        assert_eq!(outcome.failed[0].path, source);
    }

    #[test]
    fn a_zip_that_is_not_a_zip_is_reported() {
        let (root, store) = store();
        let source = chosen(&root, "library.zip", "not an archive");
        let outcome = store.import(std::slice::from_ref(&source));
        assert_eq!(outcome.failed.len(), 1);
        assert!(
            outcome.failed[0].reason.contains("zip"),
            "{:?}",
            outcome.failed[0]
        );
    }

    #[test]
    fn a_macos_zip_leaves_its_resource_forks_behind() {
        // Finder's "Compress" writes `__MACOSX/._*` twins and `.DS_Store`.
        // Neither is skill content, and `._SKILL.md` must not be read as a
        // second manifest.
        let (root, store) = store();
        let archive = root.path().join("ida-viz.zip");
        zip_at(
            &archive,
            &[
                ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                ("ida-viz/.DS_Store", b"\0\0\0\x01Bud1"),
                ("ida-viz/references/palette.md", b"# Palette\n"),
                ("__MACOSX/ida-viz/._SKILL.md", b"\0\x05\x16\x07"),
                (
                    "__MACOSX/ida-viz/references/._palette.md",
                    b"\0\x05\x16\x07",
                ),
            ],
            zip::CompressionMethod::Deflated,
        );
        let outcome = store.import(&[archive]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        let files: Vec<String> = store
            .bundled_files(&outcome.imported[0])
            .expect("files")
            .into_iter()
            .map(|file| file.path)
            .collect();
        assert_eq!(files, vec!["references/palette.md"]);
    }

    #[test]
    fn a_zip_of_packages_imports_every_package_with_its_files() {
        // A skill pack: `.skill` files zipped together. One level of nesting
        // is read; each package keeps its own bundled files.
        let (root, store) = store();
        let viz = zip_bytes(&[
            ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
            ("ida-viz/references/palette.md", b"# Palette\n"),
        ]);
        let dashboard = zip_bytes(&[("ida-dashboard/SKILL.md", IDA_SHAPED.as_bytes())]);
        let pack = root.path().join("pack.zip");
        zip_at(
            &pack,
            &[("ida-viz.skill", &viz), ("ida-dashboard.skill", &dashboard)],
            zip::CompressionMethod::Stored,
        );
        let outcome = store.import(&[pack]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        let mut names: Vec<&str> = outcome.imported.iter().map(StoredSkill::name).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["ida-dashboard", "ida-viz"]);
        assert!(stored(&store, "ida-viz/references/palette.md").is_some());
    }

    #[test]
    fn a_skill_folder_imports_with_its_files_and_its_references_are_not_skills() {
        // A reference document with frontmatter of its own used to import as
        // a separate skill. It belongs to the skill whose folder holds it.
        let (root, store) = store();
        let library = root.path().join("library");
        let viz = library.join("ida-viz");
        std::fs::create_dir_all(viz.join("references")).expect("tree");
        std::fs::create_dir_all(viz.join("evals")).expect("evals");
        std::fs::create_dir_all(viz.join("scripts").join("__pycache__")).expect("cache");
        std::fs::write(viz.join("SKILL.md"), VIZ_SHAPED).expect("manifest");
        std::fs::write(
            viz.join("references").join("palette.md"),
            "---\nname: palette\ndescription: \"Colors.\"\n---\nBody.\n",
        )
        .expect("reference");
        std::fs::write(viz.join("scripts").join("render.py"), "print()\n").expect("script");
        std::fs::write(viz.join("scripts").join("render.pyc"), [0_u8; 4]).expect("bytecode");
        std::fs::write(
            viz.join("scripts").join("__pycache__").join("x.pyc"),
            [0_u8; 4],
        )
        .expect("cache");
        std::fs::write(viz.join("evals").join("evals.json"), "{}").expect("evals");
        std::fs::write(library.join("README.md"), "# Library\n").expect("readme");
        std::fs::write(library.join("loose.md"), IDA_SHAPED).expect("loose");

        let outcome = store.import(&[library]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        let mut names: Vec<&str> = outcome.imported.iter().map(StoredSkill::name).collect();
        names.sort_unstable();
        assert_eq!(names, vec!["ida-dashboard", "ida-viz"]);
        let viz = outcome
            .imported
            .iter()
            .find(|skill| skill.name() == "ida-viz")
            .expect("viz");
        let files: Vec<String> = store
            .bundled_files(viz)
            .expect("files")
            .into_iter()
            .map(|file| file.path)
            .collect();
        // What `package_skill.py` would have packaged: no bytecode, no
        // caches, no root `evals/`.
        assert_eq!(files, vec!["references/palette.md", "scripts/render.py"]);
    }

    #[test]
    fn a_named_skill_md_brings_its_folder_when_the_folder_is_the_skill() {
        let (root, store) = store();
        let viz = root.path().join("ida-viz");
        std::fs::create_dir_all(viz.join("references")).expect("tree");
        std::fs::write(viz.join("SKILL.md"), VIZ_SHAPED).expect("manifest");
        std::fs::write(viz.join("references").join("palette.md"), "# Palette\n").expect("ref");

        let outcome = store.import(&[viz.join("SKILL.md")]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert!(stored(&store, "ida-viz/references/palette.md").is_some());
    }

    #[test]
    fn a_named_skill_md_in_an_unrelated_folder_imports_alone() {
        // A SKILL.md saved into Downloads: its neighbours are not its files.
        let (root, store) = store();
        let downloads = root.path().join("Downloads");
        std::fs::create_dir_all(&downloads).expect("dir");
        std::fs::write(downloads.join("SKILL.md"), VIZ_SHAPED).expect("manifest");
        std::fs::write(downloads.join("tax-return.pdf"), [0_u8; 16]).expect("neighbour");

        let outcome = store.import(&[downloads.join("SKILL.md")]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert!(store
            .bundled_files(&outcome.imported[0])
            .expect("files")
            .is_empty());
        assert!(stored(&store, "ida-viz/tax-return.pdf").is_none());
    }

    #[test]
    fn re_importing_a_package_drops_the_files_its_new_version_removed() {
        let (root, store) = store();
        let first = root.path().join("v1.skill");
        zip_at(
            &first,
            &[
                ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                ("ida-viz/references/old.md", b"old\n"),
            ],
            zip::CompressionMethod::Deflated,
        );
        store.import(&[first]);
        store.set_enabled("ida-viz", false).expect("disable");

        let second = root.path().join("v2.skill");
        zip_at(
            &second,
            &[
                ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                ("ida-viz/references/new.md", b"new\n"),
            ],
            zip::CompressionMethod::Deflated,
        );
        let outcome = store.import(&[second]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert!(stored(&store, "ida-viz/references/old.md").is_none());
        assert!(stored(&store, "ida-viz/references/new.md").is_some());
        assert!(!store.list().expect("list")[0].enabled, "still disabled");
        assert!(
            std::fs::read_dir(store.root().with_file_name(".skills-staging"))
                .map(|entries| entries.count() == 0)
                .unwrap_or(true),
            "a finished import leaves nothing staged"
        );
    }

    #[test]
    fn a_bundled_file_that_climbs_out_of_the_skill_fails_the_skill() {
        let (root, store) = store();
        let package = root.path().join("hostile.skill");
        zip_at(
            &package,
            &[
                ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                ("ida-viz/references/CON.md", b"device\n"),
            ],
            zip::CompressionMethod::Deflated,
        );
        let outcome = store.import(&[package]);
        assert!(outcome.imported.is_empty());
        assert_eq!(outcome.failed.len(), 1, "{:?}", outcome.failed);
        assert!(
            outcome.failed[0].reason.contains("CON.md"),
            "{:?}",
            outcome.failed[0]
        );
        assert!(store.list().expect("list").is_empty());

        // A traversal-shaped entry is never placed in a skill's folder: it
        // is at most a loose document, stored under its own slug.
        let traversal = root.path().join("traversal.skill");
        zip_at(
            &traversal,
            &[
                ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                ("ida-viz/../../escape.md", b"escaped\n"),
            ],
            zip::CompressionMethod::Deflated,
        );
        let outcome = store.import(&[traversal]);
        assert_eq!(outcome.imported.len(), 1, "{:?}", outcome.failed);
        assert!(store
            .bundled_files(&outcome.imported[0])
            .expect("files")
            .is_empty());
        assert!(!root.path().join("escape.md").exists());
        assert!(!store.root().join("escape.md").exists());
    }

    #[test]
    fn a_bundle_whose_names_clash_on_a_case_blind_disk_is_refused() {
        // On Windows and macOS a bundled `skill.md` would overwrite the
        // manifest this import just parsed, and `Guide.md` would overwrite
        // `guide.md`. Refused on every platform, so a skill imports the same
        // way everywhere.
        let (root, store) = store();
        for (name, entries) in [
            (
                "manifest.skill",
                vec![
                    ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                    (
                        "ida-viz/skill.md",
                        &b"---\nname: other\ndescription: x\n---\n"[..],
                    ),
                ],
            ),
            (
                "twins.skill",
                vec![
                    ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                    ("ida-viz/references/Guide.md", &b"one"[..]),
                    ("ida-viz/references/guide.md", &b"two"[..]),
                ],
            ),
            (
                "file-and-folder.skill",
                vec![
                    ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                    ("ida-viz/references", &b"a file"[..]),
                    ("ida-viz/references/guide.md", &b"a file in a folder"[..]),
                ],
            ),
        ] {
            let package = root.path().join(name);
            zip_at(&package, &entries, zip::CompressionMethod::Deflated);
            let outcome = store.import(&[package]);
            assert!(outcome.imported.is_empty(), "{name}");
            assert_eq!(outcome.failed.len(), 1, "{name}: {:?}", outcome.failed);
            assert!(
                outcome.failed[0].reason.contains("letter case"),
                "{name}: {:?}",
                outcome.failed[0]
            );
        }
        assert!(store.list().expect("list").is_empty());
    }

    #[test]
    fn a_bundle_larger_than_a_skill_may_be_is_refused_whole() {
        // Refused, not trimmed: a skill missing a file its instructions name
        // is a skill that silently does not work.
        let (root, store) = store();
        let package = root.path().join("huge.skill");
        let big = vec![b'a'; 33 * 1024 * 1024];
        zip_at(
            &package,
            &[
                ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                ("ida-viz/assets/data.csv", &big),
            ],
            zip::CompressionMethod::Deflated,
        );
        let outcome = store.import(&[package]);
        assert!(outcome.imported.is_empty());
        assert_eq!(outcome.failed.len(), 1, "{:?}", outcome.failed);
        assert!(
            outcome.failed[0].reason.contains("MB"),
            "{:?}",
            outcome.failed[0]
        );
        assert!(store.list().expect("list").is_empty());
    }

    #[test]
    fn a_byte_order_mark_does_not_hide_the_frontmatter() {
        let (root, store) = store();
        let source = root.path().join("ida-dashboard.md");
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend_from_slice(IDA_SHAPED.replace('\n', "\r\n").as_bytes());
        std::fs::write(&source, &bytes).expect("seed");
        let outcome = store.import(&[source]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        assert_eq!(
            stored(&store, "ida-dashboard/SKILL.md"),
            Some(bytes),
            "stored verbatim"
        );
    }

    #[test]
    fn a_bundled_file_is_read_by_the_path_its_instructions_use() {
        let (root, store) = store();
        let package = root.path().join("ida-viz.skill");
        zip_at(
            &package,
            &[
                ("ida-viz/SKILL.md", VIZ_SHAPED.as_bytes()),
                ("ida-viz/references/palette.md", b"# Palette\n"),
                ("ida-viz/assets/logo.png", &[0xFF, 0xD8, 0xFF, 0x00]),
            ],
            zip::CompressionMethod::Deflated,
        );
        store.import(&[package]);

        for spelling in [
            "references/palette.md",
            "./references/palette.md",
            "references\\palette.md",
        ] {
            assert_eq!(
                store
                    .read_file("ida-viz", spelling, 0, 1024)
                    .expect(spelling)
                    .content,
                "# Palette\n"
            );
        }
        for outside in [
            "../ida-viz/SKILL.md",
            "/etc/passwd",
            "C:/Windows/win.ini",
            "SKILL.md",
            "missing.md",
        ] {
            assert!(
                matches!(
                    store.read_file("ida-viz", outside, 0, 1024),
                    Err(super::super::store::SkillFileError::UnknownFile { .. })
                ),
                "{outside} must not be readable"
            );
        }
        assert!(matches!(
            store.read_file("ida-viz", "assets/logo.png", 0, 1024),
            Err(super::super::store::SkillFileError::Binary { bytes: 4, .. })
        ));

        store.set_enabled("ida-viz", false).expect("disable");
        assert!(
            store
                .read_file("ida-viz", "references/palette.md", 0, 1024)
                .is_err(),
            "a disabled skill's files are as unreachable as its body"
        );
    }
}

mod manifest_spelling_tests {
    use super::super::manifest::parse_skill_document;

    #[test]
    fn allowed_tools_is_read_in_every_spelling_skills_use() {
        for (spelled, expected) in [
            (
                "allowed-tools: Read Grep Glob",
                vec!["Read", "Grep", "Glob"],
            ),
            (
                "allowed-tools: Read, Bash(git add:*)",
                vec!["Read", "Bash(git add:*)"],
            ),
            ("allowed-tools: [\"Read\", \"Bash\"]", vec!["Read", "Bash"]),
            ("allowed-tools:\n  - Read", vec!["Read"]),
            ("allowed-tools:", vec![]),
        ] {
            let document =
                format!("---\nname: tools\ndescription: \"Tools.\"\n{spelled}\n---\nBody.\n");
            let parsed = parse_skill_document(&document).expect(spelled);
            let tools: Vec<&str> = parsed
                .manifest
                .allowed_tools()
                .iter()
                .map(|tool| tool.as_str())
                .collect();
            assert_eq!(tools, expected, "{spelled}");
        }
    }
}

mod dispatch_tests {
    use std::sync::Arc;

    use serde_json::{json, Value};
    use tokio_util::sync::CancellationToken;
    use xfalcon_contracts::tools::ToolId;

    use super::super::dispatch::SkillToolDispatcher;
    use super::super::store::SkillStore;
    use crate::agent::tool_loop::{ToolExecutionError, ToolExecutorPort};
    use crate::tools::registry::GET_SKILL_TOOL_ID;

    fn library_with_bundle(reference: &[u8]) -> (tempfile::TempDir, Arc<SkillStore>) {
        let root = tempfile::tempdir().expect("app local data");
        let skill = root.path().join("source").join("ida-viz");
        std::fs::create_dir_all(skill.join("references")).expect("tree");
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: ida-viz\ndescription: \"Charts.\"\n---\nRead references/guide.md.\n",
        )
        .expect("manifest");
        std::fs::write(skill.join("references").join("guide.md"), reference).expect("guide");
        std::fs::write(skill.join("logo.png"), [0xFF_u8, 0xD8, 0x00]).expect("logo");
        let account =
            crate::security::account_cleanup::ValidatedAccountId::parse("dispatch-account")
                .expect("account");
        let store = Arc::new(SkillStore::open(root.path(), &account));
        let outcome = store.import(&[skill]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);
        (root, store)
    }

    async fn call(store: &Arc<SkillStore>, arguments: Value) -> Result<Value, ToolExecutionError> {
        SkillToolDispatcher::new(store.clone(), crate::tools::ToolResultBudget::floor())
            .execute(
                &ToolId::from(GET_SKILL_TOOL_ID),
                &arguments,
                CancellationToken::new(),
            )
            .await
            .map(|result| result.structured().cloned().expect("structured result"))
    }

    /// A model that fills in every field of a schema writes `null` or an empty
    /// string for the ones it does not need. That is a call for the skill's
    /// instructions, not a failed call.
    #[tokio::test]
    async fn null_or_empty_optional_arguments_read_the_skills_instructions() {
        let (_root, store) = library_with_bundle(b"# Guide\n");
        let descriptor = crate::tools::registry::get_skill_tool_descriptor().expect("descriptor");
        let validator = jsonschema::validator_for(descriptor.input_schema()).expect("schema");

        for arguments in [
            json!({ "name": "ida-viz", "file": null, "offset": null }),
            json!({ "name": "ida-viz", "file": "", "offset": 0 }),
            json!({ "name": "ida-viz", "file": "  " }),
            json!({ "name": "ida-viz", "offset": null }),
        ] {
            assert!(validator.is_valid(&arguments), "{arguments}");
            let skill = call(&store, arguments.clone()).await.expect("instructions");
            assert_eq!(
                skill["instructions"],
                json!("Read references/guide.md."),
                "{arguments}"
            );
        }
        // A real mistake is still refused.
        assert!(!validator.is_valid(&json!({ "name": "ida-viz", "offset": -1 })));
        assert!(!validator.is_valid(&json!({ "name": "ida-viz", "file": 7 })));
    }

    #[tokio::test]
    async fn the_model_sees_the_bundled_files_beside_the_instructions() {
        let (_root, store) = library_with_bundle(b"# Guide\n");
        let skill = call(&store, json!({ "name": "ida-viz" }))
            .await
            .expect("skill");
        assert_eq!(skill["instructions"], json!("Read references/guide.md."));
        assert_eq!(skill["truncated"], json!(false));
        assert_eq!(
            skill["files"],
            json!([
                { "path": "logo.png", "bytes": 3 },
                { "path": "references/guide.md", "bytes": 8 }
            ])
        );
        assert!(skill["files_note"]
            .as_str()
            .is_some_and(|note| note.contains("file")));

        let file = call(
            &store,
            json!({ "name": "ida-viz", "file": "references/guide.md" }),
        )
        .await
        .expect("file");
        assert_eq!(file["content"], json!("# Guide\n"));
        assert_eq!(file["file"], json!("references/guide.md"));
        assert_eq!(file["truncated"], json!(false));
    }

    #[tokio::test]
    async fn a_long_bundled_file_is_read_in_pages_that_join_exactly() {
        // Multi-byte text so a page boundary can land inside a character.
        let text = "é".repeat(40 * 1024);
        let (_root, store) = library_with_bundle(text.as_bytes());
        let mut offset = 0_u64;
        let mut joined = String::new();
        loop {
            let page = call(
                &store,
                json!({ "name": "ida-viz", "file": "references/guide.md", "offset": offset }),
            )
            .await
            .expect("page");
            joined.push_str(page["content"].as_str().expect("content"));
            match page["next_offset"].as_u64() {
                Some(next) => {
                    assert_eq!(page["truncated"], json!(true));
                    offset = next;
                }
                None => break,
            }
        }
        assert_eq!(joined, text);
    }

    #[tokio::test]
    async fn a_first_page_beside_a_long_file_list_still_arrives_whole() {
        // Instructions longer than a page, and a file list that alone would
        // take most of one: the list is charged against the page, so neither
        // is cut on the way to the model, and the pages still join exactly.
        let root = tempfile::tempdir().expect("app local data");
        let skill = root.path().join("source").join("ida-viz");
        let references = skill.join("references");
        std::fs::create_dir_all(&references).expect("tree");
        let body = "Step by step, \"quoted\"\tand tabbed.\n".repeat(2_000);
        std::fs::write(
            skill.join("SKILL.md"),
            format!("---\nname: ida-viz\ndescription: \"Charts.\"\n---\n{body}"),
        )
        .expect("manifest");
        let stem = "a-long-reference-name".repeat(4);
        for index in 0..150 {
            std::fs::write(references.join(format!("{stem}-{index:03}.md")), b"x")
                .expect("reference");
        }
        let account =
            crate::security::account_cleanup::ValidatedAccountId::parse("dispatch-account")
                .expect("account");
        let store = Arc::new(SkillStore::open(root.path(), &account));
        let outcome = store.import(&[skill]);
        assert!(outcome.failed.is_empty(), "{:?}", outcome.failed);

        let budget = crate::tools::ToolResultBudget::floor();
        let dispatcher = SkillToolDispatcher::new(store.clone(), budget);
        let mut offset = 0_u64;
        let mut joined = String::new();
        loop {
            let result = dispatcher
                .execute(
                    &ToolId::from(GET_SKILL_TOOL_ID),
                    &json!({ "name": "ida-viz", "offset": offset }),
                    CancellationToken::new(),
                )
                .await
                .expect("page");
            assert!(
                result.model_preview(budget).complete(),
                "a page is never cut on the way to the model"
            );
            let page = result.structured().cloned().expect("structured result");
            if offset == 0 {
                let listed = page["files"].as_array().expect("files").len();
                let unlisted = page["files_not_listed"].as_u64().unwrap_or(0) as usize;
                assert!(
                    listed > 0 && unlisted > 0,
                    "{listed} listed, {unlisted} not"
                );
                assert_eq!(listed + unlisted, 150);
            } else {
                assert!(page.get("files").is_none(), "files are listed once");
            }
            joined.push_str(page["instructions"].as_str().expect("instructions"));
            match page["next_offset"].as_u64() {
                Some(next) => offset = next,
                None => break,
            }
        }
        assert_eq!(joined, body.trim());
    }

    #[tokio::test]
    async fn every_miss_is_named_rather_than_answered_empty() {
        let (_root, store) = library_with_bundle(b"# Guide\n");
        for (arguments, expected) in [
            (json!({ "name": "ida-viz", "file": "logo.png" }), "binary"),
            (
                json!({ "name": "ida-viz", "file": "../../secrets.md" }),
                "bundles no file",
            ),
            (json!({ "name": "ida-viz", "file": 7 }), "files list"),
            (json!({ "name": "ida-viz", "offset": -1 }), "offset"),
            (
                json!({ "name": "ida-viz", "offset": 1_000_000 }),
                "past the end",
            ),
            (json!({ "name": "nothing" }), "No skill named"),
        ] {
            match call(&store, arguments.clone()).await {
                Err(ToolExecutionError::Failed(message)) => {
                    assert!(message.contains(expected), "{arguments}: {message}");
                }
                other => panic!("{arguments} must fail, got {other:?}"),
            }
        }
    }
}
