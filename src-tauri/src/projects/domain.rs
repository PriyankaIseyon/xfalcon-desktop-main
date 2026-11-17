//! `ProjectId`, `TrustedRoot`, and the containment API domain callers use.
//!
//! `TrustedRoot` is the only way project code ever touches a trusted
//! project's filesystem contents. There are no raw path joins anywhere in
//! this module or in `super::trust`: every resolution goes through
//! [`TrustedRoot::resolve_existing`]/[`TrustedRoot::resolve_new`], which
//! revalidate the root's stable identity (`super::platform::RootIdentity`)
//! and walk the requested relative path one component at a time through
//! `cap-std`, rejecting any symlink or reparse point encountered anywhere
//! along the way (not only an escaping one -- see the decision record's
//! "containment-check design" section for why this is deliberately
//! stricter than the plan's literal "reject outside link/reparse/mount
//! targets" wording).

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use cap_std::fs::{Dir, OpenOptions};
use uuid::Uuid;

use super::path_policy::RelativeProjectPath;
use super::platform::{self, PlatformError, RootIdentity};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProjectId(Uuid);

impl ProjectId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn from_uuid(id: Uuid) -> Self {
        Self(id)
    }

    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for ProjectId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("project id is not a valid identifier")]
pub struct ProjectIdParseError;

impl FromStr for ProjectId {
    type Err = ProjectIdParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| ProjectIdParseError)
    }
}

impl From<ProjectId> for xfalcon_contracts::projects::ProjectId {
    fn from(value: ProjectId) -> Self {
        value.to_string().into()
    }
}

/// Hard ceiling on the number of entries [`TrustedRoot::list_entries`]/
/// [`TrustedRoot::list_root_entries`] will return from a single directory --
/// defense against an unbounded/hostile directory listing exhausting memory
/// before a caller ever sees a result, mirroring
/// `crate::skills::discovery::MAX_SKILLS_PER_SOURCE`'s identical rationale.
///
/// Sized for a person browsing a real data folder — an export split into tens
/// of thousands of part files — while still refusing a directory that would
/// exhaust memory. The agent's listing pages through whatever this returns.
pub const MAX_DIRECTORY_ENTRIES: usize = 100_000;

#[derive(Debug, thiserror::Error)]
pub enum ContainmentError {
    #[error("the trusted root is not a plain directory")]
    NotADirectory,
    #[error("the trusted root's identity no longer matches the identity recorded at trust time (it may have been deleted and recreated, or a different volume mounted at the same path)")]
    RootIdentityChanged,
    #[error("the resolved path passes through a symlink or reparse point")]
    LinkOrReparse,
    #[error("the resolved path does not exist")]
    NotFound,
    #[error("the requested relative path is structurally invalid")]
    InvalidComponent,
    #[error("a resolved path from a different trusted project was presented to this one")]
    MismatchedRoot,
    #[error("directory listing exceeds {0} entries")]
    TooManyEntries(usize),
    #[error("expected a plain file but the path names a directory")]
    TargetIsDirectory,
    #[error(transparent)]
    Platform(#[from] PlatformError),
    #[error("filesystem operation failed: {0}")]
    Io(#[source] io::Error),
}

fn map_io(error: io::Error) -> ContainmentError {
    if error.kind() == io::ErrorKind::NotFound {
        ContainmentError::NotFound
    } else {
        ContainmentError::Io(error)
    }
}

/// One project-relative path, proven to resolve (or, for
/// [`TrustedRoot::resolve_new`], to be safely creatable) inside a specific
/// [`TrustedRoot`] at the moment it was produced. Carries no open file
/// handle -- Task 5/6's actual reads/writes go through `TrustedRoot`'s own
/// `cap_std::fs::Dir` using the relative path this wraps, revalidating
/// identity fresh at that time too (this value is not a standing capability
/// that skips a later revalidation).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedProjectPath {
    project_id: ProjectId,
    relative: RelativeProjectPath,
}

impl ResolvedProjectPath {
    #[must_use]
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    #[must_use]
    pub fn relative(&self) -> &RelativeProjectPath {
        &self.relative
    }
}

/// A live, capability-scoped handle onto a trusted project's root directory.
///
/// # Security contract
///
/// Every method that touches the filesystem MUST re-derive the root's
/// current stable identity from the OS and compare it against
/// [`Self::expected_identity`] before doing anything else -- there is no
/// "check once, trust forever" state. This is a disclosed, non-blocking
/// TOCTOU window (identical in kind to `crate::skills::discovery`'s):
/// between a successful `resolve_existing`/`resolve_new` and a caller's
/// later, separate filesystem operation using the returned relative path,
/// an adversary with concurrent local write access to the same filesystem
/// could still swap something. Closing that window completely would
/// require holding the resolved file open for the entire caller operation
/// (a Task 5/6 concern, since no read/write exists yet in this task). This
/// mirrors this repository's threat model (`docs/threat-model.md`,
/// TM-007); see the decision record for the full rationale.
#[derive(Debug)]
pub struct TrustedRoot {
    project_id: ProjectId,
    canonical_path: PathBuf,
    dir: Dir,
    expected_identity: RootIdentity,
}

impl TrustedRoot {
    /// Canonicalizes `user_path`, opens it through `cap-std`, and records its
    /// current stable identity as the identity to require on every future
    /// revalidation. Used once, at trust-establishment time
    /// (`project_create`) -- see `super::trust::TrustStore::create`.
    pub fn establish(project_id: ProjectId, user_path: &Path) -> Result<Self, ContainmentError> {
        let canonical_path = std::fs::canonicalize(user_path).map_err(map_io)?;
        let (dir, identity) = open_and_check(&canonical_path)?;
        Ok(Self {
            project_id,
            canonical_path,
            dir,
            expected_identity: identity,
        })
    }

    /// Re-opens an already-trusted root from its stored canonical path and
    /// requires its current identity to match `expected_identity` (loaded
    /// from encrypted storage). Used every time a previously trusted project
    /// is revalidated: session start, `project_activate`, and before any
    /// future mutation.
    pub fn reopen(
        project_id: ProjectId,
        canonical_path: PathBuf,
        expected_identity: RootIdentity,
    ) -> Result<Self, ContainmentError> {
        let (dir, identity) = open_and_check(&canonical_path)?;
        if identity != expected_identity {
            return Err(ContainmentError::RootIdentityChanged);
        }
        Ok(Self {
            project_id,
            canonical_path,
            dir,
            expected_identity,
        })
    }

    #[must_use]
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    #[must_use]
    pub fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    #[must_use]
    pub fn identity(&self) -> RootIdentity {
        self.expected_identity
    }

    fn revalidate(&self) -> Result<(), ContainmentError> {
        // Reopen the stored path so deleting and recreating a root cannot be
        // hidden by the original directory handle, which continues to refer
        // to the unlinked inode on Unix.
        let (_, identity) = open_and_check(&self.canonical_path)?;
        if identity != self.expected_identity {
            return Err(ContainmentError::RootIdentityChanged);
        }
        Ok(())
    }

    /// Resolves `path`, which MUST already exist, inside this trusted root.
    /// Rejects a changed root identity, a missing path, and a symlink or
    /// reparse point anywhere along the resolved chain (mid-path or final
    /// component).
    pub fn resolve_existing(
        &self,
        path: RelativeProjectPath,
    ) -> Result<ResolvedProjectPath, ContainmentError> {
        self.revalidate()?;
        self.walk_reject_links(path.as_path(), false)?;
        Ok(ResolvedProjectPath {
            project_id: self.project_id,
            relative: path,
        })
    }

    /// Resolves `path` for a not-yet-existing file. Ancestor directories need
    /// not exist yet -- writing a new file into a new directory is an ordinary
    /// create, not a special case -- but every component that *does* exist
    /// must be a plain directory, and none of them may be a symlink or reparse
    /// point. Rejects a changed root identity.
    ///
    /// Containment is not weakened by allowing a missing tail: once a
    /// component is absent, nothing below it exists to follow, so there is no
    /// link for a walk to traverse. The absent components are still required
    /// to be legal path components, and
    /// [`Self::create_missing_parents`] -- the only code that materializes
    /// them -- re-checks for a link at every step as it goes.
    pub fn resolve_new(
        &self,
        path: RelativeProjectPath,
    ) -> Result<ResolvedProjectPath, ContainmentError> {
        self.revalidate()?;
        self.walk_reject_links_allow_missing_tail(path.as_path())?;
        Ok(ResolvedProjectPath {
            project_id: self.project_id,
            relative: path,
        })
    }

    /// Creates every missing directory on `resolved`'s parent chain, each one
    /// through its own parent's capability handle (never a multi-component
    /// `create_dir_all`), re-checking for a symlink or reparse point at every
    /// step so a link planted mid-walk still fails closed.
    ///
    /// Returns the project-relative paths it actually created, in creation
    /// order, so `super::writer` can remove them again if the plan it is
    /// staging rolls back. `pub(super)`: creating a directory is a mutation,
    /// and `super::writer` is the only module allowed to mutate.
    pub(super) fn create_missing_parents(
        &self,
        resolved: &ResolvedProjectPath,
    ) -> Result<Vec<PathBuf>, ContainmentError> {
        self.check_owner(resolved)?;
        self.revalidate()?;
        let components: Vec<_> = resolved.relative().as_path().components().collect();
        if components.is_empty() {
            return Err(ContainmentError::InvalidComponent);
        }

        let mut current = self.dir.try_clone().map_err(ContainmentError::Io)?;
        let mut created: Vec<PathBuf> = Vec::new();
        let mut prefix = PathBuf::new();
        // The final component is the file being written, never created here.
        for component in &components[..components.len() - 1] {
            let Component::Normal(name) = component else {
                return Err(ContainmentError::InvalidComponent);
            };
            prefix.push(name);
            match current.symlink_metadata(name) {
                Ok(metadata) => {
                    if platform::is_link_or_reparse(&metadata) {
                        return Err(ContainmentError::LinkOrReparse);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    current.create_dir(name).map_err(map_io)?;
                    created.push(prefix.clone());
                }
                Err(error) => return Err(map_io(error)),
            }
            // Fails if the component exists but is not a directory, which is
            // the only remaining way this chain can be unusable.
            current = current.open_dir(name).map_err(map_io)?;
        }
        Ok(created)
    }

    /// Best-effort removal of one directory a rolling-back plan created.
    /// Silently does nothing when the directory is missing or no longer empty:
    /// cleanup must never destroy something another process put there, and a
    /// leftover empty directory is not a failure worth propagating (mirroring
    /// [`WriteTarget::remove_temp_sibling`]'s own best-effort contract).
    pub(super) fn remove_created_dir(&self, relative: &Path) {
        if let Ok((parent, name)) = self.walk_to_parent(relative, false) {
            let _ = parent.remove_dir(&name);
        }
    }

    /// Convenience wrapper combining [`RelativeProjectPath::new`] and
    /// [`Self::resolve_existing`] for callers (the `PathAuthority` impl
    /// below, future command handlers) that start from a raw string.
    pub fn resolve_existing_str(&self, raw: &str) -> Result<ResolvedProjectPath, ContainmentError> {
        let relative =
            RelativeProjectPath::new(raw).map_err(|_| ContainmentError::InvalidComponent)?;
        self.resolve_existing(relative)
    }

    /// Opens `resolved` for reading. Revalidates root identity and re-walks
    /// the full path fresh, rejecting any symlink/reparse point encountered
    /// anywhere along it -- never trusts the earlier `resolve_existing` call
    /// that produced `resolved` (see [`ResolvedProjectPath`]'s doc comment:
    /// "not a standing capability that skips a later revalidation"). The
    /// final hop is a single-component open against an already
    /// link-checked parent (via [`Self::walk_to_parent`]), never a raw
    /// multi-component path, so `cap-std`'s own (more permissive)
    /// internal-symlink tolerance never applies to the final component
    /// either.
    pub fn open_file(
        &self,
        resolved: &ResolvedProjectPath,
    ) -> Result<cap_std::fs::File, ContainmentError> {
        self.check_owner(resolved)?;
        self.revalidate()?;
        let (parent, name) = self.walk_to_parent(resolved.relative().as_path(), false)?;
        parent.open(&name).map_err(map_io)
    }

    /// Returns `resolved`'s metadata via the same revalidated, link-free walk
    /// as [`Self::open_file`], without opening it for read.
    pub fn file_metadata(
        &self,
        resolved: &ResolvedProjectPath,
    ) -> Result<cap_std::fs::Metadata, ContainmentError> {
        self.check_owner(resolved)?;
        self.revalidate()?;
        let (parent, name) = self.walk_to_parent(resolved.relative().as_path(), false)?;
        parent.metadata(&name).map_err(map_io)
    }

    /// Reads a directory's stable filesystem identity through a freshly
    /// revalidated, link-free capability handle. Timestamps alone do not prove
    /// that a directory entry is still the one staged by the writer.
    pub(crate) fn directory_identity(
        &self,
        resolved: &ResolvedProjectPath,
    ) -> Result<RootIdentity, ContainmentError> {
        self.check_owner(resolved)?;
        self.revalidate()?;
        let (parent, name) = self.walk_to_parent(resolved.relative().as_path(), false)?;
        let metadata = parent.symlink_metadata(&name).map_err(map_io)?;
        if platform::is_link_or_reparse(&metadata) {
            return Err(ContainmentError::LinkOrReparse);
        }
        if !metadata.is_dir() {
            return Err(ContainmentError::NotADirectory);
        }
        let directory = parent.open_dir(&name).map_err(map_io)?;
        Ok(platform::root_identity(&directory)?)
    }

    /// Opens `resolved` (which must be a plain, link-free directory) and
    /// returns its immediate child entries as `(name, metadata)` pairs.
    /// Every child's metadata comes from a non-following read (see
    /// [`collect_entries`]), so a symlinked child is reported as a symlink,
    /// never silently resolved; this method never recurses and never opens a
    /// child itself. Bounded by [`MAX_DIRECTORY_ENTRIES`].
    pub fn list_entries(
        &self,
        resolved: &ResolvedProjectPath,
    ) -> Result<Vec<(OsString, cap_std::fs::Metadata)>, ContainmentError> {
        self.check_owner(resolved)?;
        self.revalidate()?;
        let (parent, name) = self.walk_to_parent(resolved.relative().as_path(), false)?;
        let metadata = parent.symlink_metadata(&name).map_err(map_io)?;
        if platform::is_link_or_reparse(&metadata) {
            return Err(ContainmentError::LinkOrReparse);
        }
        let directory = parent.open_dir(&name).map_err(map_io)?;
        collect_entries(&directory)
    }

    /// Lists the trusted root's own immediate children -- the project-root
    /// equivalent of [`Self::list_entries`], needed because
    /// [`super::path_policy::RelativeProjectPath`] cannot represent the
    /// empty ("this directory itself") path. Bounded by
    /// [`MAX_DIRECTORY_ENTRIES`].
    pub fn list_root_entries(
        &self,
    ) -> Result<Vec<(OsString, cap_std::fs::Metadata)>, ContainmentError> {
        self.revalidate()?;
        collect_entries(&self.dir)
    }

    /// [`Self::list_root_entries`] under a smaller bound, for the boundary test.
    #[cfg(test)]
    pub(crate) fn list_root_entries_bounded(
        &self,
        limit: usize,
    ) -> Result<Vec<(OsString, cap_std::fs::Metadata)>, ContainmentError> {
        self.revalidate()?;
        collect_entries_bounded(&self.dir, limit)
    }

    /// Defense-in-depth ownership check: a [`ResolvedProjectPath`] is only
    /// ever constructed by *some* `TrustedRoot`'s `resolve_existing`/
    /// `resolve_new`, but nothing at the type level stops a caller from
    /// presenting one root's resolved path to a *different* root. Doing so
    /// would not escape either root's own containment (each root still only
    /// ever resolves relative to itself), but it would silently operate on
    /// the wrong project, so every access method above checks this first.
    fn check_owner(&self, resolved: &ResolvedProjectPath) -> Result<(), ContainmentError> {
        if resolved.project_id() == &self.project_id {
            Ok(())
        } else {
            Err(ContainmentError::MismatchedRoot)
        }
    }

    /// Walks `relative` one path component at a time through this root's
    /// `cap_std::fs::Dir`, opening each intermediate directory (a single
    /// contained hop, never a raw multi-segment join) and checking every
    /// component -- including the final one -- is neither a symlink nor a
    /// (Windows) reparse point. `allow_missing_final` permits the last
    /// component to not exist yet (for `resolve_new`); every other
    /// component, and the final one when `allow_missing_final` is false,
    /// must exist.
    ///
    /// Returns the directory positioned at the final component's *parent*
    /// (this root itself, for a single-component path) plus the final
    /// component's own name, so callers that need to actually open, stat, or
    /// list that final component ([`Self::open_file`],
    /// [`Self::file_metadata`], [`Self::list_entries`]) can do so with one
    /// additional single-component operation against an already
    /// link-checked parent, rather than re-walking from the root or
    /// performing a raw multi-component open.
    fn walk_to_parent(
        &self,
        relative: &Path,
        allow_missing_final: bool,
    ) -> Result<(Dir, OsString), ContainmentError> {
        let components: Vec<_> = relative.components().collect();
        if components.is_empty() {
            return Err(ContainmentError::InvalidComponent);
        }
        let last_index = components.len() - 1;
        let mut current = self.dir.try_clone().map_err(ContainmentError::Io)?;
        let mut final_name = None;

        for (index, component) in components.into_iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err(ContainmentError::InvalidComponent);
            };
            let is_final = index == last_index;

            match current.symlink_metadata(name) {
                Ok(metadata) => {
                    if platform::is_link_or_reparse(&metadata) {
                        return Err(ContainmentError::LinkOrReparse);
                    }
                    if !is_final {
                        current = current.open_dir(name).map_err(map_io)?;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if !(is_final && allow_missing_final) {
                        return Err(ContainmentError::NotFound);
                    }
                }
                Err(error) => return Err(map_io(error)),
            }
            if is_final {
                final_name = Some(name.to_owned());
            }
        }
        // The `components.is_empty()` check above guarantees the loop runs
        // at least once, and `final_name` is unconditionally set on the last
        // (i.e. final) iteration, so this can never be `None`.
        Ok((
            current,
            final_name.expect("final component name is always set by the loop above"),
        ))
    }

    /// [`Self::walk_reject_links`] for a path whose ancestors may not exist
    /// yet. Every component that exists is link-checked exactly as usual;
    /// after the first absent one, the remainder cannot be a link (nothing is
    /// there to follow), so only its component form is still validated.
    fn walk_reject_links_allow_missing_tail(
        &self,
        relative: &Path,
    ) -> Result<(), ContainmentError> {
        let components: Vec<_> = relative.components().collect();
        if components.is_empty() {
            return Err(ContainmentError::InvalidComponent);
        }
        let last_index = components.len() - 1;
        let mut current = self.dir.try_clone().map_err(ContainmentError::Io)?;
        let mut missing = false;

        for (index, component) in components.into_iter().enumerate() {
            let Component::Normal(name) = component else {
                return Err(ContainmentError::InvalidComponent);
            };
            if missing {
                continue;
            }
            match current.symlink_metadata(name) {
                Ok(metadata) => {
                    if platform::is_link_or_reparse(&metadata) {
                        return Err(ContainmentError::LinkOrReparse);
                    }
                    if index != last_index {
                        current = current.open_dir(name).map_err(map_io)?;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => missing = true,
                Err(error) => return Err(map_io(error)),
            }
        }
        Ok(())
    }

    /// Walks `relative`, rejecting any symlink/reparse point anywhere along
    /// the chain, without opening or returning a handle to the final
    /// component. Used by [`Self::resolve_existing`], which only needs a
    /// yes/no containment decision.
    fn walk_reject_links(
        &self,
        relative: &Path,
        allow_missing_final: bool,
    ) -> Result<(), ContainmentError> {
        self.walk_to_parent(relative, allow_missing_final)
            .map(|_| ())
    }

    /// Revalidates identity and re-walks `resolved`'s full path fresh
    /// (rejecting any link/reparse point anywhere along it, exactly like
    /// every other accessor -- never trusting the earlier
    /// `resolve_existing`/`resolve_new` call that produced `resolved`),
    /// returning a same-parent [`WriteTarget`] for `super::writer`'s
    /// temp-file-then-atomic-rename sequence. This is the ONLY chokepoint
    /// `super::patch`/`super::writer` are allowed to obtain one from -- there
    /// is no raw path join anywhere in either module.
    pub(crate) fn write_target(
        &self,
        resolved: &ResolvedProjectPath,
    ) -> Result<WriteTarget, ContainmentError> {
        self.check_owner(resolved)?;
        self.revalidate()?;
        let (parent, final_name) = self.walk_to_parent(resolved.relative().as_path(), true)?;
        Ok(WriteTarget { parent, final_name })
    }
}

/// A parent directory handle plus a final-component name, both already
/// proven by the same link-rejecting walk every other [`TrustedRoot`]
/// accessor uses -- the only shape [`super::writer`]'s atomic-write sequence
/// is allowed to act on. Only ever constructed by [`TrustedRoot::write_target`].
pub(crate) struct WriteTarget {
    parent: Dir,
    final_name: OsString,
}

impl WriteTarget {
    /// Creates a fresh, exclusively-owned (`create_new`) temporary file next
    /// to this target's final component -- same parent directory, so the
    /// later atomic rename never crosses a directory (and therefore never a
    /// filesystem/volume boundary either; see [`Self::atomic_replace`]).
    /// Returns the open handle plus the generated name, so the caller can
    /// write/flush content into the handle before renaming the name into
    /// place.
    pub(crate) fn create_temp_sibling(
        &self,
    ) -> Result<(cap_std::fs::File, OsString), ContainmentError> {
        let temp_name = OsString::from(format!(".xfalcon-mut-{}.tmp", Uuid::new_v4()));
        let mut options = OpenOptions::new();
        options.create_new(true).read(true).write(true);
        let file = self
            .parent
            .open_with(&temp_name, &options)
            .map_err(map_io)?;
        Ok((file, temp_name))
    }

    /// The current permissions of this target's final component, or `None`
    /// when nothing is there yet.
    ///
    /// `None` is the create case, and a created file correctly takes the
    /// platform default for a new file in this directory -- there is no prior
    /// file to inherit from. Read through this target's own already
    /// link-checked parent handle, like every other decision here.
    pub(crate) fn existing_permissions(&self) -> Option<cap_std::fs::Permissions> {
        self.parent
            .metadata(&self.final_name)
            .ok()
            .map(|metadata| metadata.permissions())
    }

    /// Best-effort removal of a temp sibling this target created. Used on
    /// every failure path so no orphaned temp file survives (plan: "no
    /// orphaned temp files left behind on any failure path"). Swallows its
    /// own error: if cleanup itself fails there is nothing more to preserve
    /// or report here -- the caller's original mutation error is always what
    /// actually gets surfaced.
    pub(crate) fn remove_temp_sibling(&self, temp_name: &OsStr) {
        let _ = self.parent.remove_file(temp_name);
    }

    /// A cheap, same-parent-handle existence recheck of this target's final
    /// component -- used by `super::writer::apply` immediately before a
    /// `Create` operation's atomic swap, to narrow (not eliminate; no
    /// OS-level "fail if exists" rename primitive is used here -- see
    /// `Self::atomic_replace`'s doc comment) the window between that
    /// operation's staging-time existence check and its own apply during
    /// which an out-of-band write could otherwise be silently clobbered
    /// instead of triggering the rejection that check exists to guarantee.
    /// A non-following `symlink_metadata` check: a symlink/reparse point
    /// occupying the name still counts as "exists" here (this is a plain
    /// existence probe, not a containment decision).
    pub(crate) fn exists(&self) -> bool {
        self.parent.symlink_metadata(&self.final_name).is_ok()
    }

    /// The one and only place a `Create`/`Modify` mutation's new content
    /// actually replaces (or creates) live project content: an atomic
    /// same-directory rename of `temp_name` onto this target's final
    /// component, via `cap_std::fs::Dir::rename`. Confirmed by reading
    /// `cap-primitives-4.0.2`'s own unchecked implementations directly
    /// (`src/rustix/fs/rename_unchecked.rs`, `src/windows/fs/rename_unchecked.rs`):
    /// Unix reduces to a bare `renameat(2)` (atomic, and returns `EXDEV`
    /// rather than falling back to copy+delete if the two directories ever
    /// sit on different filesystems); Windows reduces to `std::fs::rename`,
    /// itself `MoveFileExW(..., MOVEFILE_REPLACE_EXISTING)` with no
    /// `MOVEFILE_COPY_ALLOWED` set (the flag that would otherwise let it
    /// silently fall back to a non-atomic copy across volumes) -- the same
    /// primitive/flag combination the already-depended-on `tempfile` crate's
    /// own `persist()` uses on Windows (`tempfile-3.27.0/src/file/imp/windows.rs`).
    /// Writing a second, hand-rolled FFI call to the exact same primitive
    /// `cap_std` already safely wraps would add unaudited surface for no
    /// behavioral gain (the same "no excess dependency/attack surface for no
    /// behavioral gain" bar Task 5's own decision record applied to
    /// rejecting `grep-searcher`). Never called on anything but a
    /// same-parent temp sibling this same [`WriteTarget`] created.
    pub(crate) fn atomic_replace(&self, temp_name: &OsStr) -> Result<(), ContainmentError> {
        self.parent
            .rename(temp_name, &self.parent, &self.final_name)
            .map_err(map_io)
    }

    /// Renames this target's final component directly onto a *different*
    /// [`WriteTarget`]'s final component -- the `Rename` operation's actual
    /// path to disk (no temp file or content copy at all, since the
    /// source's bytes already exist on disk exactly where they need to be).
    /// Fails cleanly (`io::ErrorKind`/raw OS error surfacing `EXDEV` on Unix,
    /// `ERROR_NOT_SAME_DEVICE` on Windows -- see [`super::writer`]'s error
    /// classification) rather than falling back to copy+delete if `self` and
    /// `destination` sit on different filesystems/volumes internally mounted
    /// under the same trusted root -- see [`Self::atomic_replace`]'s doc
    /// comment for why this primitive already guarantees that without any
    /// extra code here.
    pub(crate) fn atomic_rename_onto(&self, destination: &Self) -> Result<(), ContainmentError> {
        self.parent
            .rename(
                &self.final_name,
                &destination.parent,
                &destination.final_name,
            )
            .map_err(map_io)
    }

    /// Removes this target's final component as a directory -- the
    /// `DeleteDirectory` operation's only path to disk.
    ///
    /// Deliberately `remove_dir`, never a recursive removal: the OS refuses a
    /// non-empty directory as part of this call, which is what makes "delete
    /// this folder" verifiable without walking it first. A directory that
    /// gained an entry since the plan was proposed fails here instead of being
    /// destroyed.
    pub(crate) fn remove_directory(&self) -> Result<(), ContainmentError> {
        self.parent.remove_dir(&self.final_name).map_err(map_io)
    }

    /// Whether this target's final component is a plain directory (not a file
    /// and not a link), read through the same already link-checked parent
    /// handle as every other decision here.
    pub(crate) fn is_directory(&self) -> bool {
        self.parent
            .symlink_metadata(&self.final_name)
            .is_ok_and(|metadata| metadata.is_dir() && !platform::is_link_or_reparse(&metadata))
    }

    /// Removes this target's final component outright -- the `Delete`
    /// operation's only path to disk. There is no weaker "atomic delete"
    /// primitive to fail closed onto: a crash immediately before this call
    /// leaves the original file fully intact, and a crash immediately after
    /// leaves it fully gone; `unlink(2)`/`DeleteFile`-via-rename-replace
    /// admits no partially-deleted state in between for a caller to ever
    /// observe.
    pub(crate) fn remove(&self) -> Result<(), ContainmentError> {
        self.parent.remove_file(&self.final_name).map_err(map_io)
    }

    /// Fsyncs the parent directory this target's mutation just landed in.
    /// See `super::platform::fsync_dir_after_replace`'s doc comment for the
    /// full per-platform durability contract.
    pub(crate) fn fsync_parent(&self) -> Result<(), ContainmentError> {
        platform::fsync_dir_after_replace(&self.parent).map_err(ContainmentError::from)
    }
}

/// Shared by [`TrustedRoot::list_entries`] and
/// [`TrustedRoot::list_root_entries`]: reads `directory`'s immediate
/// children via `Dir::entries`, whose `DirEntry::metadata` corresponds to
/// `std::fs::DirEntry::metadata` -- documented by `std` to not traverse a
/// symlink if the entry itself is one, i.e. the same non-following
/// guarantee as `symlink_metadata` -- so a symlinked child is reported as a
/// symlink via `Metadata::is_symlink`/[`platform::is_link_or_reparse`],
/// never silently resolved. Bounded by [`MAX_DIRECTORY_ENTRIES`] rather than
/// collecting an unbounded `Vec` from a hostile/huge directory.
fn collect_entries(
    directory: &Dir,
) -> Result<Vec<(OsString, cap_std::fs::Metadata)>, ContainmentError> {
    collect_entries_bounded(directory, MAX_DIRECTORY_ENTRIES)
}

/// [`collect_entries`] with its bound as a parameter, so the boundary is
/// testable without writing a hundred thousand files.
fn collect_entries_bounded(
    directory: &Dir,
    limit: usize,
) -> Result<Vec<(OsString, cap_std::fs::Metadata)>, ContainmentError> {
    let mut entries = Vec::new();
    for entry in directory.entries().map_err(map_io)? {
        let entry = entry.map_err(map_io)?;
        let metadata = entry.metadata().map_err(map_io)?;
        entries.push((entry.file_name(), metadata));
        if entries.len() > limit {
            return Err(ContainmentError::TooManyEntries(limit));
        }
    }
    Ok(entries)
}

fn open_and_check(canonical_path: &Path) -> Result<(Dir, RootIdentity), ContainmentError> {
    let dir = platform::open_root(canonical_path)?;
    let metadata = dir.dir_metadata().map_err(map_io)?;
    if !metadata.is_dir() {
        return Err(ContainmentError::NotADirectory);
    }
    if platform::is_link_or_reparse(&metadata) {
        return Err(ContainmentError::LinkOrReparse);
    }
    let identity = platform::root_identity(&dir)?;
    Ok((dir, identity))
}

impl crate::tools::policy::PathAuthority for TrustedRoot {
    fn contains(&self, path: &Path) -> bool {
        let relative: &Path = if path.is_absolute() {
            match path.strip_prefix(&self.canonical_path) {
                Ok(stripped) => stripped,
                Err(_) => return false,
            }
        } else {
            path
        };
        // The empty relative path denotes the trusted root itself (an
        // absolute `path` exactly equal to `self.canonical_path`, or a
        // caller-supplied empty relative path — see
        // `crate::agent::tool_loop::requested_path_for`'s "whole project"
        // case for `project.search_files`/`project.search_text`, which have
        // no `path` argument at all). `RelativeProjectPath` cannot represent
        // this ("this directory itself") case at all (see
        // `Self::list_root_entries`'s own doc comment), so routing it
        // through `resolve_existing_str` would always fail with
        // `PathPolicyError::Empty` — silently turning "the whole trusted
        // root" into "not trusted" for every caller. `list_root_entries`
        // already establishes the correct pattern for this exact case:
        // revalidate identity and treat the root itself as contained,
        // without ever constructing a `RelativeProjectPath` for it. A
        // review finding caught an earlier version of this method routing
        // an empty path through `resolve_existing_str` (which always denies
        // it) instead.
        if relative.as_os_str().is_empty() {
            return self.revalidate().is_ok();
        }
        let Some(relative_str) = relative.to_str() else {
            return false;
        };
        self.resolve_existing_str(relative_str).is_ok()
    }
}
