//! Data-only `SKILL.md` frontmatter parsing.
//!
//! A `SKILL.md` file is untrusted content (`docs/threat-model.md`
//! TM-006: "a malicious skill" is an explicit abuse case). The structural
//! guarantees are therefore hard: fixed frontmatter delimiters, size ceilings
//! checked before any allocation, a validated name, and an instructions body
//! treated as inert text — never evaluated, executed, or used to load an
//! adjacent script or binary.
//!
//! What is deliberately NOT strict is the field set. This parser was
//! originally written with `#[serde(deny_unknown_fields)]` and a required
//! `version`, on the reasoning that a partially-recovered parse could hide a
//! policy field. Measured against the files it now has to read, that shape
//! rejects every skill that exists: all twelve skills in the `data-analyst`
//! repository declare `name` and `description` and nothing else, and skills
//! written for other agents carry `license`, `metadata`, or a hyphenated
//! `allowed-tools`. A parser that refuses the entire corpus is not fail-closed,
//! it is fail-useless.
//!
//! The security property is kept where it belongs instead: this contract has
//! exactly two policy fields, both matched here under both spellings, and
//! neither can *widen* a run's tools (see `super::policy` — a skill scope is
//! intersected with the effective allowlist, never unioned). An unrecognized
//! key therefore cannot be a policy field of this contract, so ignoring it
//! hides nothing. See
//! `docs/decisions.md` for the
//! reference implementations this shape was compared against.

use serde::Deserialize;

use xfalcon_contracts::tools::ToolId;

/// Hard ceiling on the whole `SKILL.md` document (frontmatter + instructions)
/// checked before any line-splitting work, so a caller that hands this
/// function a raw string directly (rather than pre-checking file size, as
/// `super::discovery` does) still gets a bounded parse.
pub const MAX_DOCUMENT_BYTES: usize = MAX_FRONTMATTER_BYTES + MAX_INSTRUCTIONS_BYTES;

/// Frontmatter block size cap. `source-examples/grok-build`'s equivalent
/// (`xai-grok-tools::implementations::skills::discovery::MAX_FRONTMATTER_BYTES`)
/// uses 4096; this is doubled to comfortably fit a bounded `allowed_tools`/
/// `allowed_project_paths` list plus a description, while staying far below
/// `MAX_INSTRUCTIONS_BYTES`.
pub const MAX_FRONTMATTER_BYTES: usize = 8 * 1024;

/// Instructions body size cap ("oversized instructions body rejected" in the
/// plan). Not a prompt-size bound: a body reaches a run only when the model
/// fetches it, one bounded page at a time (`super::dispatch`). Sized so the
/// whole document stays inside discovery's 256 KiB file ceiling, and above
/// the largest published skills (Anthropic's `claude-api` is 86 KB).
pub const MAX_INSTRUCTIONS_BYTES: usize = 240 * 1024;

/// Matches `source-examples/grok-build`'s `MAX_NAME_LEN` for the equivalent
/// field.
pub const MAX_NAME_BYTES: usize = 64;

/// Four times the Agent Skills specification's 1024 characters, because
/// published skills exceed it — Anthropic's own `claude-api` declares 1068
/// characters, and Claude Code loads it. Refusing those costs the user the
/// skill and buys nothing: a run's prompt shows at most the first 300
/// characters of any description (`crate::agent::prompt`).
pub const MAX_DESCRIPTION_BYTES: usize = 4 * 1024;

pub const MAX_VERSION_BYTES: usize = 32;
pub const MAX_ALLOWED_TOOLS: usize = 32;
pub const MAX_ALLOWED_PATHS: usize = 32;
pub const MAX_PATH_BYTES: usize = 256;

/// A skill identifier: lowercase ASCII letters, digits, and single internal
/// hyphens. A declared name is *normalized* into that charset rather than
/// rejected for missing it — `My Skill` and `My_Skill` both become
/// `my-skill`. A capital letter is a formatting difference, not a reason to
/// refuse a user's skill, and this is also the directory name the skill is
/// stored under, so normalizing here is what keeps the declared name and the
/// stored path the same string. Matches
/// `source-examples/grok-build`'s normalization and xfalcon-web's
/// `generateSlug`, which the imported files were written against.
///
/// A name that normalizes to nothing — punctuation only — is still an error:
/// there is no slug to store it under.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SkillName(String);

impl SkillName {
    pub fn new(name: impl Into<String>) -> Result<Self, ManifestError> {
        let name = name.into();
        if name.is_empty() {
            return Err(ManifestError::EmptyName);
        }
        if name.len() > MAX_NAME_BYTES {
            return Err(ManifestError::NameTooLong(MAX_NAME_BYTES));
        }
        let mut slug = String::with_capacity(name.len());
        for character in name.chars() {
            if character.is_ascii_alphanumeric() {
                slug.push(character.to_ascii_lowercase());
            } else if !slug.ends_with('-') {
                slug.push('-');
            }
        }
        let slug = slug.trim_matches('-').to_owned();
        if slug.is_empty() {
            return Err(ManifestError::InvalidNameCharset);
        }
        Ok(Self(slug))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated skill version. Deliberately a simplified `MAJOR.MINOR.PATCH`
/// grammar (three non-empty ASCII-digit components), not full SemVer 2.0
/// (pre-release/build metadata) — the `semver` crate is not already a
/// workspace dependency and this contract's own example (`version: 1.0.0`)
/// never uses pre-release syntax, so adding it would be unjustified
/// complexity for a closed, enumerable need.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillVersion(String);

impl SkillVersion {
    /// What a skill that declares no `version` carries. Every skill file this
    /// app is expected to read is in that state, so this is the common case,
    /// not a fallback for a malformed one.
    pub const UNVERSIONED: &'static str = "0.0.0";

    #[must_use]
    pub fn unversioned() -> Self {
        Self(Self::UNVERSIONED.to_owned())
    }

    pub fn new(version: impl Into<String>) -> Result<Self, ManifestError> {
        let version = version.into();
        if version.is_empty() {
            return Err(ManifestError::EmptyVersion);
        }
        if version.len() > MAX_VERSION_BYTES {
            return Err(ManifestError::VersionTooLong(MAX_VERSION_BYTES));
        }
        let parts: Vec<&str> = version.split('.').collect();
        let is_valid = parts.len() == 3
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
        if !is_valid {
            return Err(ManifestError::InvalidVersion(version));
        }
        Ok(Self(version))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A validated, relative project-path scope entry declared in
/// `allowed_project_paths`. This is a lexical check only — non-empty,
/// bounded, relative (no leading `/`, `\`, or drive-letter prefix), and free
/// of `..` traversal segments and NUL bytes. It is deliberately NOT a
/// containment guarantee: Task 4's `TrustedRoot`/`PathAuthority` (see
/// `crate::tools::policy::PathAuthority`'s security contract) is what
/// canonicalizes and revalidates a real path against a real trusted root.
/// This type only keeps a manifest from declaring an obviously-hostile
/// pattern in the first place — defense in depth, not the containment
/// boundary itself.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillProjectPath(String);

impl SkillProjectPath {
    pub fn new(path: impl Into<String>) -> Result<Self, ManifestError> {
        let path = path.into();
        if path.is_empty() {
            return Err(ManifestError::EmptyPath);
        }
        if path.len() > MAX_PATH_BYTES {
            return Err(ManifestError::PathTooLong(MAX_PATH_BYTES));
        }
        if path.contains('\0') {
            return Err(ManifestError::PathTraversal(path));
        }
        let looks_absolute = path.starts_with('/')
            || path.starts_with('\\')
            || path
                .as_bytes()
                .get(1)
                .is_some_and(|&b| b == b':' && path.as_bytes()[0].is_ascii_alphabetic());
        if looks_absolute {
            return Err(ManifestError::AbsolutePath(path));
        }
        if path.split(['/', '\\']).any(|component| component == "..") {
            return Err(ManifestError::PathTraversal(path));
        }
        Ok(Self(path))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Deserialization target for the frontmatter YAML block.
///
/// `name` and `description` are the only required fields, because they are
/// the only two every real skill file declares. `version` is absent from all
/// of them and defaults to [`SkillVersion::UNVERSIONED`]. Unknown keys are
/// ignored — see the module doc comment for why that costs no security here.
///
/// Both policy fields accept the hyphenated spelling other agents use, so a
/// skill that *did* mean to narrow its tools is honored rather than silently
/// read as declaring none.
#[derive(Deserialize)]
struct RawFrontmatter {
    name: String,
    description: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default, alias = "allowed-tools")]
    allowed_tools: ToolList,
    #[serde(default, alias = "allowed-project-paths")]
    allowed_project_paths: Vec<String>,
}

/// `allowed-tools` as skills actually spell it: a YAML list, or the single
/// string the Agent Skills specification defines (`Read Grep Glob`), which
/// Claude Code skills also write comma-separated (`Read, Bash(git:*)`).
/// Refusing the string form rejected every skill written to the spec.
#[derive(Default, Deserialize)]
#[serde(untagged)]
enum ToolList {
    #[default]
    Absent,
    Listed(Vec<String>),
    Spelled(String),
}

impl ToolList {
    /// Splits the string form on whitespace and commas outside parentheses,
    /// so `Bash(git add:*)` stays one entry.
    fn into_entries(self) -> Vec<String> {
        let spelled = match self {
            Self::Absent => return Vec::new(),
            Self::Listed(entries) => return entries,
            Self::Spelled(spelled) => spelled,
        };
        let mut entries = Vec::new();
        let mut current = String::new();
        let mut depth = 0_usize;
        for character in spelled.chars() {
            match character {
                '(' => depth += 1,
                ')' => depth = depth.saturating_sub(1),
                ',' | ' ' | '\t' | '\n' | '\r' if depth == 0 => {
                    if !current.is_empty() {
                        entries.push(std::mem::take(&mut current));
                    }
                    continue;
                }
                _ => {}
            }
            current.push(character);
        }
        if !current.is_empty() {
            entries.push(current);
        }
        entries
    }
}

/// A validated `SKILL.md` frontmatter block.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillManifest {
    name: SkillName,
    description: String,
    version: SkillVersion,
    allowed_tools: Vec<ToolId>,
    allowed_project_paths: Vec<SkillProjectPath>,
}

impl SkillManifest {
    #[must_use]
    pub fn name(&self) -> &SkillName {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub fn version(&self) -> &SkillVersion {
        &self.version
    }

    #[must_use]
    pub fn allowed_tools(&self) -> &[ToolId] {
        &self.allowed_tools
    }

    #[must_use]
    pub fn allowed_project_paths(&self) -> &[SkillProjectPath] {
        &self.allowed_project_paths
    }
}

/// A fully parsed `SKILL.md` document: the validated manifest plus the
/// instructions body (everything after the closing frontmatter delimiter).
/// Callers that only need the catalog-visible metadata (`super::discovery`)
/// discard `instructions` rather than retaining it — see that module's
/// "lazy relevance selection" doc comment.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedSkillDocument {
    pub manifest: SkillManifest,
    pub instructions: String,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("SKILL.md document exceeds {0} bytes")]
    DocumentTooLarge(usize),
    #[error("frontmatter delimiters not found (expected a leading and a closing `---` line)")]
    MissingFrontmatterDelimiters,
    #[error("frontmatter exceeds {0} bytes")]
    FrontmatterTooLarge(usize),
    #[error("instructions body exceeds {0} bytes")]
    InstructionsTooLarge(usize),
    /// Covers both YAML syntax errors and schema violations (e.g. an
    /// unrecognized field via `deny_unknown_fields`) — see the module doc
    /// comment and `RawFrontmatter`'s doc comment for why these are one
    /// taxonomy rather than split further: both mean "the YAML does not
    /// parse into this manifest's strict, closed shape."
    #[error("frontmatter is not valid YAML for the SKILL.md contract: {0}")]
    MalformedYaml(String),
    #[error("skill name must not be empty")]
    EmptyName,
    #[error("skill name exceeds {0} bytes")]
    NameTooLong(usize),
    #[error("skill name contains no letters or digits to form an identifier from")]
    InvalidNameCharset,
    #[error("skill description must not be empty")]
    EmptyDescription,
    #[error("skill description exceeds {0} bytes")]
    DescriptionTooLong(usize),
    #[error("skill version must not be empty")]
    EmptyVersion,
    #[error("skill version exceeds {0} bytes")]
    VersionTooLong(usize),
    #[error("skill version {0:?} is not a valid MAJOR.MINOR.PATCH version")]
    InvalidVersion(String),
    #[error("too many allowed tools (maximum {0})")]
    TooManyAllowedTools(usize),
    #[error("allowed tool id must not be empty")]
    EmptyToolId,
    #[error("allowed tool id exceeds {0} bytes")]
    ToolIdTooLong(usize),
    #[error("too many allowed project paths (maximum {0})")]
    TooManyAllowedPaths(usize),
    #[error("allowed project path must not be empty")]
    EmptyPath,
    #[error("allowed project path exceeds {0} bytes")]
    PathTooLong(usize),
    #[error("allowed project path must be relative (found {0:?})")]
    AbsolutePath(String),
    #[error("allowed project path must not contain `..` traversal segments (found {0:?})")]
    PathTraversal(String),
}

/// Splits a raw `SKILL.md` document into its frontmatter YAML and
/// instructions body. The opening delimiter must be the document's first
/// line, exactly `---`; the closing delimiter must be a later line that is
/// also exactly `---`. `str::lines` strips both `\n` and `\r\n` terminators,
/// so this comparison is CRLF-safe without separate handling. This is
/// intentionally stricter than `source-examples/grok-build`'s
/// `after_first.find("\n---")` substring search (which would also match a
/// line like `---itude` immediately following an empty line) — a
/// security-relevant manifest should not accept an ambiguous delimiter
/// boundary.
fn split_frontmatter(raw: &str) -> Result<(String, String), ManifestError> {
    let mut lines = raw.lines();
    let first = lines.next().unwrap_or_default();
    if first != "---" {
        return Err(ManifestError::MissingFrontmatterDelimiters);
    }

    let mut frontmatter_lines = Vec::new();
    let mut closed = false;
    for line in lines.by_ref() {
        if line == "---" {
            closed = true;
            break;
        }
        frontmatter_lines.push(line);
    }
    if !closed {
        return Err(ManifestError::MissingFrontmatterDelimiters);
    }

    let instructions: Vec<&str> = lines.collect();
    Ok((frontmatter_lines.join("\n"), instructions.join("\n")))
}

fn build_tool_ids(raw: Vec<String>) -> Result<Vec<ToolId>, ManifestError> {
    if raw.len() > MAX_ALLOWED_TOOLS {
        return Err(ManifestError::TooManyAllowedTools(MAX_ALLOWED_TOOLS));
    }
    raw.into_iter()
        .map(|tool_id| {
            if tool_id.is_empty() {
                return Err(ManifestError::EmptyToolId);
            }
            if tool_id.len() > MAX_NAME_BYTES {
                return Err(ManifestError::ToolIdTooLong(MAX_NAME_BYTES));
            }
            Ok(ToolId::from(tool_id))
        })
        .collect()
}

fn build_project_paths(raw: Vec<String>) -> Result<Vec<SkillProjectPath>, ManifestError> {
    if raw.len() > MAX_ALLOWED_PATHS {
        return Err(ManifestError::TooManyAllowedPaths(MAX_ALLOWED_PATHS));
    }
    raw.into_iter().map(SkillProjectPath::new).collect()
}

/// Parses and validates a full `SKILL.md` document: strict frontmatter
/// (never executed, never used to load an adjacent script or binary) plus
/// the instructions body. This is the single entry point every discovery
/// path (`super::discovery`) must go through.
pub fn parse_skill_document(raw: &str) -> Result<ParsedSkillDocument, ManifestError> {
    if raw.len() > MAX_DOCUMENT_BYTES {
        return Err(ManifestError::DocumentTooLarge(MAX_DOCUMENT_BYTES));
    }

    let (frontmatter, instructions) = split_frontmatter(raw)?;

    if frontmatter.len() > MAX_FRONTMATTER_BYTES {
        return Err(ManifestError::FrontmatterTooLarge(MAX_FRONTMATTER_BYTES));
    }
    if instructions.len() > MAX_INSTRUCTIONS_BYTES {
        return Err(ManifestError::InstructionsTooLarge(MAX_INSTRUCTIONS_BYTES));
    }

    // `serde-saphyr` is a pure-Rust YAML 1.2 parser with default budgets on
    // nesting depth, aliases and anchors, so an untrusted frontmatter cannot
    // expand or nest without bound. It replaced `serde_yaml_ng`, whose
    // `unsafe-libyaml` backend is unmaintained (RUSTSEC-2024-0449) and is the
    // C-memory-safety surface this parse path should not carry. The size caps
    // above still run first. YAML 1.2 reads `yes`, `no` and `on` as plain
    // strings rather than booleans, which only matters to a field typed as a
    // string, and every field here is.
    let raw_frontmatter: RawFrontmatter = serde_saphyr::from_str(&frontmatter)
        .map_err(|err| ManifestError::MalformedYaml(err.to_string()))?;

    let name = SkillName::new(raw_frontmatter.name)?;
    if raw_frontmatter.description.is_empty() {
        return Err(ManifestError::EmptyDescription);
    }
    if raw_frontmatter.description.len() > MAX_DESCRIPTION_BYTES {
        return Err(ManifestError::DescriptionTooLong(MAX_DESCRIPTION_BYTES));
    }
    let version = match raw_frontmatter.version {
        Some(declared) => SkillVersion::new(declared)?,
        None => SkillVersion::unversioned(),
    };
    let allowed_tools = build_tool_ids(raw_frontmatter.allowed_tools.into_entries())?;
    let allowed_project_paths = build_project_paths(raw_frontmatter.allowed_project_paths)?;

    Ok(ParsedSkillDocument {
        manifest: SkillManifest {
            name,
            description: raw_frontmatter.description,
            version,
            allowed_tools,
            allowed_project_paths,
        },
        instructions,
    })
}
