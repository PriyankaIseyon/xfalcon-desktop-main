//! Filesystem discovery for imported and (trust-gated) project skills.
//!
//! Imported skills live at `<account-dir>/skills/*/SKILL.md`, one library per
//! account, written there by `super::store` from files the user chose. This app ships none of
//! its own: the catalog is exactly what was imported. This module only ever
//! receives an already-resolved directory path, never an `AppHandle`, so it
//! stays unit-testable without a running app (mirroring `crate::mcp`'s
//! "independently testable" discipline).
//!
//! Project skills come from `<trusted-root>/.xfalcon/skills/*/SKILL.md`.
//! [`TrustState`] is the narrow seam every other item in this module only
//! ever sees: a plain two-variant enum, never a `crate::projects` type
//! directly. `crate::projects` (Task 4) now exists, and its
//! `xfalcon_contracts::projects::ProjectTrustStatus` converts into
//! [`TrustState`] via the `impl From<ProjectTrustStatus> for TrustState`
//! below — but nothing calls [`discover_project`] from production code yet.
//! The real call site that resolves a project's current trust status and
//! passes the converted [`TrustState`] in is Task 7's agent-loop
//! skill-invocation integration; see
//! `docs/decisions.md` for
//! that disclosure. Until that caller exists, [`discover_project`] performs
//! **zero filesystem access** when untrusted — the directory is not even
//! opened — so an untrusted project's skills remain completely inert, per
//! `docs/threat-model.md` TM-006 ("content is data, never
//! authority") and the plan's "project skill before trust" requirement. This
//! is a deliberately stricter posture than `source-examples/grok-build`'s
//! `TrustStore`/`PluginRegistry`, which still discovers and lists untrusted
//! project plugins' skills (metadata-only) and only blocks their executable
//! surfaces (hooks, MCP servers); see the decision record for the
//! rationale.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use xfalcon_contracts::projects::ProjectTrustStatus;

use super::manifest::{self, ManifestError, ParsedSkillDocument, SkillManifest};

/// Hard ceiling on a single `SKILL.md` file's size, checked via metadata
/// *before* its bytes are read — an oversized or hostile file is rejected
/// without ever loading its content into memory.
const MAX_SKILL_FILE_BYTES: u64 = 256 * 1024;

/// Hard ceiling on how many skill directories a single discovery pass may
/// enumerate — defense against an unbounded/hostile directory listing
/// (mirrors `crate::mcp::catalog`'s `MAX_TOOLS` bound for the same reason).
pub(super) const MAX_SKILLS_PER_SOURCE: usize = 256;

/// Where a discovered skill came from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkillSource {
    /// Imported by the user through Settings.
    User,
    Project,
}

/// Whether the project a project-scoped skill would come from is currently
/// trusted. `crate::projects`'s
/// `xfalcon_contracts::projects::ProjectTrustStatus` (the real
/// trusted-root/session state Task 4 produces) converts into this
/// two-variant enum via the `From` impl below; see the module doc comment
/// for why no production caller wires that conversion into
/// [`discover_project`] yet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrustState {
    Untrusted,
    Trusted,
}

/// Maps a project's revalidated trust status onto the narrow two-variant
/// seam this module consumes: only [`ProjectTrustStatus::Trusted`] yields
/// [`TrustState::Trusted`] — [`ProjectTrustStatus::Unavailable`] (root
/// missing, identity changed, or any other revalidation failure) fails
/// closed to [`TrustState::Untrusted`], never panics or defaults
/// optimistically.
impl From<ProjectTrustStatus> for TrustState {
    fn from(status: ProjectTrustStatus) -> Self {
        match status {
            ProjectTrustStatus::Trusted => Self::Trusted,
            ProjectTrustStatus::Unavailable => Self::Untrusted,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("failed to read skill directory {path:?}: {source}")]
    ReadDir {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read skill file {path:?}: {source}")]
    ReadFile {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to read metadata for {path:?}: {source}")]
    Metadata {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("skill file {path:?} exceeds {limit} bytes")]
    FileTooLarge { path: PathBuf, limit: u64 },
    #[error("skill path {path:?} escapes its root via a symlink or reparse point")]
    SymlinkEscape { path: PathBuf },
    #[error("skill manifest at {path:?} is invalid: {source}")]
    Manifest {
        path: PathBuf,
        source: ManifestError,
    },
    #[error("discovered more than {0} skills from a single source")]
    TooManySkills(usize),
}

impl std::fmt::Display for SkillSource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => formatter.write_str("imported"),
            Self::Project => formatter.write_str("project"),
        }
    }
}

/// A cataloged skill: metadata visible for lazy-relevance selection, plus
/// provenance (source, content hash) and the manifest itself. Deliberately
/// does NOT carry the parsed instructions body: only a selected skill's body
/// is ever loaded, via [`load_instructions`] (wrapped by
/// `super::registry::SkillRegistry::select`). Cataloging a skill (this type)
/// and loading its full instructions are separate operations on purpose —
/// see the plan's "lazy relevance selection" requirement.
#[derive(Clone, Debug, PartialEq)]
pub struct DiscoveredSkill {
    manifest: SkillManifest,
    source: SkillSource,
    content_hash: String,
    path: PathBuf,
}

impl DiscoveredSkill {
    /// The catalog entry for a skill `super::store` has just written, built
    /// from the parse it already did. Avoids re-scanning the whole directory
    /// after every imported file purely to learn what was just stored.
    #[must_use]
    pub(super) fn stored(
        manifest: SkillManifest,
        source: SkillSource,
        content_hash: String,
        path: PathBuf,
    ) -> Self {
        Self {
            manifest,
            source,
            content_hash,
            path,
        }
    }

    #[must_use]
    pub fn manifest(&self) -> &SkillManifest {
        &self.manifest
    }

    #[must_use]
    pub fn source(&self) -> SkillSource {
        self.source
    }

    /// Lowercase hex SHA-256 of the raw `SKILL.md` file bytes at discovery
    /// time (`format!("{:x}", Sha256::digest(bytes))`, matching
    /// `crate::chat::local_projection::hash_bytes`'s idiom).
    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Discovers imported skills from `skills_root/*/SKILL.md`, where
/// `skills_root` is an account's `skills` folder — see the module doc comment.
/// An absent directory is an empty catalog, not an error: it is what a fresh
/// install looks like.
pub fn discover_user(skills_root: &Path) -> Result<Vec<DiscoveredSkill>, DiscoveryError> {
    discover_skill_dirs(skills_root, SkillSource::User)
}

/// Discovers project skills from `<trusted_root>/.xfalcon/skills/*/SKILL.md`.
///
/// Returns an empty catalog with **no filesystem access at all** when `trust`
/// is [`TrustState::Untrusted`] — see the module doc comment.
pub fn discover_project(
    trusted_root: &Path,
    trust: TrustState,
) -> Result<Vec<DiscoveredSkill>, DiscoveryError> {
    if trust == TrustState::Untrusted {
        return Ok(Vec::new());
    }
    let skills_dir = trusted_root.join(".xfalcon").join("skills");
    discover_skill_dirs(&skills_dir, SkillSource::Project)
}

fn discover_skill_dirs(
    root: &Path,
    source: SkillSource,
) -> Result<Vec<DiscoveredSkill>, DiscoveryError> {
    let root_canonical = match fs::canonicalize(root) {
        Ok(path) => path,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(DiscoveryError::ReadDir {
                path: root.to_path_buf(),
                source: err,
            })
        }
    };

    let entries = match fs::read_dir(&root_canonical) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(DiscoveryError::ReadDir {
                path: root_canonical.clone(),
                source: err,
            })
        }
    };

    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| DiscoveryError::ReadDir {
            path: root_canonical.clone(),
            source: err,
        })?;
        // `Path::is_dir` follows symlinks, so a symlinked directory is
        // included here and only rejected below once its canonical target
        // is known to escape `root_canonical` — never silently skipped.
        if entry.path().is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    if dirs.len() > MAX_SKILLS_PER_SOURCE {
        return Err(DiscoveryError::TooManySkills(MAX_SKILLS_PER_SOURCE));
    }

    let mut skills = Vec::with_capacity(dirs.len());
    for dir in dirs {
        let skill_md = dir.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }

        // Symlink-escape guard (plan requirement: "a project skill's
        // SKILL.md or its directory must not symlink outside the trusted
        // root"). `fs::canonicalize` resolves the full symlink/junction
        // chain for both the containing directory and the file itself, so
        // either one pointing outside `root_canonical` is caught here. This
        // is an interim, lexical-canonicalization boundary — Task 4's
        // `TrustedRoot`/`PathAuthority` (see
        // `crate::tools::policy::PathAuthority`'s security contract) is the
        // real, revalidated-at-use-time containment API for actual project
        // operations.
        let dir_canonical = fs::canonicalize(&dir).map_err(|err| DiscoveryError::ReadDir {
            path: dir.clone(),
            source: err,
        })?;
        if !dir_canonical.starts_with(&root_canonical) {
            return Err(DiscoveryError::SymlinkEscape { path: dir });
        }
        let skill_md_canonical =
            fs::canonicalize(&skill_md).map_err(|err| DiscoveryError::ReadFile {
                path: skill_md.clone(),
                source: err,
            })?;
        if !skill_md_canonical.starts_with(&root_canonical) {
            return Err(DiscoveryError::SymlinkEscape { path: skill_md });
        }

        let (parsed, content_hash) = load_and_parse(&skill_md_canonical)?;
        skills.push(DiscoveredSkill {
            manifest: parsed.manifest,
            source,
            content_hash,
            path: skill_md_canonical,
        });
    }
    Ok(skills)
}

/// Reads and parses one `SKILL.md` file, returning its raw bytes, the full
/// parsed document (manifest + instructions), and the content hash.
///
/// The bytes come back because `super::store` writes exactly what it read:
/// an import that re-serialized the manifest would make the stored file
/// disagree with the file the user chose, and the content hash meaningless.
pub(super) fn read_and_parse(
    path: &Path,
) -> Result<(Vec<u8>, ParsedSkillDocument, String), DiscoveryError> {
    let bytes = read_bounded(path)?;
    let (parsed, content_hash) =
        parse_bytes(&bytes).map_err(|source| DiscoveryError::Manifest {
            path: path.to_path_buf(),
            source,
        })?;
    Ok((bytes, parsed, content_hash))
}

/// Reads one candidate skill file, checking its size from metadata *before*
/// its bytes are read — an oversized or hostile file is rejected without ever
/// loading its content into memory.
pub(super) fn read_bounded(path: &Path) -> Result<Vec<u8>, DiscoveryError> {
    let metadata = fs::metadata(path).map_err(|err| DiscoveryError::Metadata {
        path: path.to_path_buf(),
        source: err,
    })?;
    if metadata.len() > MAX_SKILL_FILE_BYTES {
        return Err(DiscoveryError::FileTooLarge {
            path: path.to_path_buf(),
            limit: MAX_SKILL_FILE_BYTES,
        });
    }
    fs::read(path).map_err(|err| DiscoveryError::ReadFile {
        path: path.to_path_buf(),
        source: err,
    })
}

/// Parses already-held bytes into a document plus their content hash.
///
/// Separate from [`read_and_parse`] because a candidate inside an archive has
/// no path to read from, and it must go through the exact same parse — the
/// import path and the discovery path must never disagree about what a valid
/// skill is.
///
/// A leading UTF-8 byte-order mark is skipped for the parse only (the hash
/// and the stored bytes keep it): Windows editors write one, and it would
/// otherwise hide the opening `---` delimiter.
pub(super) fn parse_bytes(bytes: &[u8]) -> Result<(ParsedSkillDocument, String), ManifestError> {
    let content_hash = format!("{:x}", Sha256::digest(bytes));
    let unmarked = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let raw = String::from_utf8_lossy(unmarked);
    Ok((manifest::parse_skill_document(&raw)?, content_hash))
}

/// The ceiling one candidate file may occupy, shared by the on-disk read
/// above and `super::package`'s reader.
pub(super) const MAX_CANDIDATE_BYTES: u64 = MAX_SKILL_FILE_BYTES;

/// [`read_and_parse`] without the raw bytes. Shared by discovery (which also
/// discards `instructions`, see [`DiscoveredSkill`]) and [`load_instructions`]
/// (lazy body load) so both go through the exact same bounded-read-then-parse
/// path.
fn load_and_parse(path: &Path) -> Result<(ParsedSkillDocument, String), DiscoveryError> {
    let (_bytes, parsed, content_hash) = read_and_parse(path)?;
    Ok((parsed, content_hash))
}

/// Lazily loads a discovered skill's instructions body by re-reading and
/// re-parsing its `SKILL.md` file from disk. Only called once a skill is
/// actually selected for a run (explicit slash command today; bounded
/// relevance selection is a documented future extension of the same seam —
/// see `super::registry::SkillSelection`) — never during discovery/cataloging.
pub fn load_instructions(skill: &DiscoveredSkill) -> Result<String, DiscoveryError> {
    let (parsed, _content_hash) = load_and_parse(skill.path())?;
    Ok(parsed.instructions)
}
