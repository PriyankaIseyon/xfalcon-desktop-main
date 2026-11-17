//! Reading what the user chose into skills to store.
//!
//! A skill is a folder: `SKILL.md` plus whatever it bundles beside it —
//! `references/`, `scripts/`, `assets/`, or any other file (the Agent Skills
//! layout). A `.skill` file is that folder zipped: skill-creator's
//! `package_skill.py` writes `<name>/SKILL.md` and its siblings into a
//! DEFLATE zip, and other packagers put `SKILL.md` at the archive root. A
//! loose `.md` with frontmatter is a skill with nothing bundled.
//!
//! Everything a user can hand over — a file, a `.skill` or `.zip` archive, a
//! folder — is read here as one tree of entries, and the tree is split into
//! skills the same way whatever it came from:
//!
//! - Every directory holding a `SKILL.md` is a skill. Each file under it
//!   belongs to the nearest such directory above it, so a library of skill
//!   folders splits into its skills and a skill's `references/*.md` are its
//!   own files rather than candidate skills of their own.
//! - A `.skill` file anywhere is a package of its own, never a bundled file:
//!   the extension names a distributable skill.
//! - A `.md` outside every skill folder is a single-file candidate.
//!
//! # What is never trusted
//!
//! An entry's path decides only where a bundled file sits *inside* its
//! skill's folder; the folder itself is always named by the parsed manifest's
//! slug (`super::store`). A bundled path must be a plain relative path of
//! portable names, so an archive cannot aim a write outside the skill, at a
//! device name, or at a name another platform cannot hold. Symlinks are never
//! followed and never stored. Every read is bounded before it allocates:
//! entry count, per-skill file count and bytes, per-source bytes, depth, and
//! nesting.

use std::collections::HashSet;
use std::fs;
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};

use super::discovery;

pub(super) const SKILL_FILE: &str = "SKILL.md";

/// How deep a chosen folder is searched. A skill library is a folder of skill
/// folders; eight levels covers a repository checkout without walking a whole
/// home directory if one is chosen by mistake.
const MAX_SEARCH_DEPTH: usize = 8;
/// How many files one chosen folder or archive may list before the read
/// stops. Directories skipped as noise (`.git`, `node_modules`) do not count.
const MAX_ENTRIES_PER_SOURCE: usize = 4096;
/// How many skills one chosen folder or archive may yield.
const MAX_CANDIDATES_PER_SOURCE: usize = 512;
/// How many files one skill may bundle beside its `SKILL.md`. Above
/// claude.ai's reported 200-file upload ceiling, so a skill made for Claude
/// always fits.
pub(super) const MAX_BUNDLED_FILES: usize = 512;
/// Total bytes one skill may bundle, `SKILL.md` included — the Claude API's
/// 30 MB uncompressed skill ceiling, rounded up to a binary unit.
pub(super) const MAX_BUNDLE_BYTES: u64 = 32 * 1024 * 1024;
/// Total bytes one chosen folder or archive may expand to — the bound that
/// makes a compression bomb or an enormous folder a bounded read — and the
/// largest archive file that may be chosen.
const MAX_SOURCE_BYTES: u64 = 256 * 1024 * 1024;
/// How many folders deep a bundled file may sit inside its skill. The
/// specification keeps references one level deep; real skills (fonts under
/// `assets/fonts/<family>/`) go a few more.
const MAX_BUNDLED_PATH_DEPTH: usize = 16;
/// How many archives deep a read goes. One level admits the real case, a zip
/// of `.skill` packages; a package inside that is read as text.
const MAX_ARCHIVE_NESTING: usize = 1;

/// Directories that are never skill content: version control, dependency
/// caches, bytecode caches, and the resource forks macOS writes into zips.
const IGNORED_DIRECTORIES: [&str; 4] = [".git", "node_modules", "__pycache__", "__MACOSX"];

/// A skill read out of a chosen path, ready to store.
pub(super) struct Candidate {
    /// What to show if it fails.
    pub label: PathBuf,
    /// The `SKILL.md` (or loose `.md`) bytes, exactly as read.
    pub document: Vec<u8>,
    /// Files bundled beside the document, by `/`-separated path relative to
    /// the skill folder, sorted.
    pub files: Vec<(String, Vec<u8>)>,
    /// Whether a parse failure is reported. True for a file the user named
    /// and for any `SKILL.md`: the name declares it a skill, so a broken one
    /// is a skill that did not import. False for a loose `.md` found in a
    /// folder, which is usually a README.
    pub owed_reason: bool,
}

/// One result of reading a chosen path.
pub(super) enum Found {
    Candidate(Candidate),
    /// A skill that could not be read at all — too large, unreadable, or
    /// holding a file name that cannot be stored. Always reported.
    Failed {
        label: PathBuf,
        reason: String,
    },
}

/// What a finished read says about the chosen path as a whole.
#[derive(Debug, Default)]
pub(super) struct Summary {
    /// A folder or archive, whose contents were found rather than named.
    pub searched: bool,
    /// How many candidates or failures the read yielded.
    pub yielded: usize,
    /// Why the read stopped before the end, when a bound stopped it.
    pub stopped: Option<String>,
}

/// Reads one chosen path, handing each skill it holds to `visit` as soon as
/// it is read, so only one skill's bytes are held at a time.
///
/// `Err` is the chosen path itself being unusable — unreadable, or a `.zip`
/// that is not a zip — and is reported against that path.
pub(super) fn read(path: &Path, visit: &mut dyn FnMut(Found)) -> Result<Summary, String> {
    let metadata = fs::metadata(path).map_err(|err| format!("could not be read: {err}"))?;
    let mut reader = Reader {
        visit,
        summary: Summary::default(),
        expanded: 0,
    };
    if metadata.is_dir() {
        reader.summary.searched = true;
        let tree = Tree::directory(path, &mut reader.summary)?;
        reader.read_tree(&tree, path, 0);
        return Ok(reader.summary);
    }

    // Sniffed from the header, so a named archive is read from disk entry by
    // entry — never held whole — and a named Markdown file is held to the
    // skill-file bound.
    if starts_like_zip(path)? {
        if metadata.len() > MAX_SOURCE_BYTES {
            return Err(size_refusal(MAX_SOURCE_BYTES));
        }
        let file = fs::File::open(path).map_err(|err| format!("could not be read: {err}"))?;
        reader.summary.searched = true;
        let tree = Tree::archive(Box::new(file), &mut reader.summary)?;
        reader.read_tree(&tree, path, 0);
        return Ok(reader.summary);
    }
    if has_extension(path, "zip") {
        return Err("is not a readable zip".to_owned());
    }
    let bytes = read_file_bounded(path, discovery::MAX_CANDIDATE_BYTES)?;

    // A named `SKILL.md` in a folder that carries the skill's own name is
    // that skill's folder — the Agent Skills rule that a skill's directory
    // matches its name. Anything else (a `SKILL.md` saved into Downloads) is
    // the document alone, since its neighbours are not its files.
    if is_skill_file(path) {
        if let Some(folder) = path.parent().filter(|folder| names_skill(folder, &bytes)) {
            let tree = Tree::directory(folder, &mut reader.summary)?;
            reader.read_named_skill(&tree, path);
            return Ok(reader.summary);
        }
    }
    reader.yield_found(Found::Candidate(Candidate {
        label: path.to_path_buf(),
        document: bytes,
        files: Vec::new(),
        owed_reason: true,
    }));
    Ok(reader.summary)
}

struct Reader<'a> {
    visit: &'a mut dyn FnMut(Found),
    summary: Summary,
    expanded: u64,
}

impl Reader<'_> {
    fn yield_found(&mut self, found: Found) {
        self.summary.yielded += 1;
        (self.visit)(found);
    }

    fn stop(&mut self, reason: String) {
        if self.summary.stopped.is_none() {
            self.summary.stopped = Some(reason);
        }
    }

    fn exhausted(&self) -> bool {
        self.summary.stopped.is_some()
    }

    /// Charges bytes against the source's total. `false` once the total is
    /// spent, which stops the whole read rather than skipping one skill.
    fn charge(&mut self, bytes: u64) -> bool {
        self.expanded = self.expanded.saturating_add(bytes);
        if self.expanded > MAX_SOURCE_BYTES {
            self.stop(format!(
                "stopped after {} MB; choose a smaller folder or archive",
                MAX_SOURCE_BYTES / (1024 * 1024)
            ));
            return false;
        }
        true
    }

    fn read_tree(&mut self, tree: &Tree, label: &Path, nesting: usize) {
        for unit in plan(&tree.entries) {
            if self.exhausted() {
                return;
            }
            if self.summary.yielded >= MAX_CANDIDATES_PER_SOURCE {
                self.stop(format!(
                    "stopped after {MAX_CANDIDATES_PER_SOURCE} skills; choose a smaller folder or archive"
                ));
                return;
            }
            match unit {
                Unit::Skill { manifest, files } => {
                    let found = self.load_skill(tree, label, manifest, &files);
                    if let Some(found) = found {
                        self.yield_found(found);
                    }
                }
                Unit::Document(entry) => {
                    // Found, not named: one too large to be a skill is
                    // skipped, not fatal, and must not end the search.
                    let Ok(document) = tree.read(entry, discovery::MAX_CANDIDATE_BYTES) else {
                        continue;
                    };
                    if !self.charge(document.len() as u64) {
                        return;
                    }
                    self.yield_found(Found::Candidate(Candidate {
                        label: tree.label(label, entry),
                        document,
                        files: Vec::new(),
                        owed_reason: false,
                    }));
                }
                Unit::Package(entry) => self.read_package(tree, label, entry, nesting),
            }
        }
    }

    /// A `.skill` found inside a folder or archive: a zip read as a tree of
    /// its own, or — the older plain-text form — a single document.
    fn read_package(&mut self, tree: &Tree, label: &Path, entry: &Entry, nesting: usize) {
        let package_label = tree.label(label, entry);
        let bytes = match tree.read(entry, MAX_BUNDLE_BYTES) {
            Ok(bytes) => bytes,
            Err(reason) => {
                self.yield_found(Found::Failed {
                    label: package_label,
                    reason,
                });
                return;
            }
        };
        if !self.charge(bytes.len() as u64) {
            return;
        }
        if is_zip(&bytes) && nesting < MAX_ARCHIVE_NESTING {
            match Tree::archive(Box::new(Cursor::new(bytes)), &mut self.summary) {
                Ok(inner) => self.read_tree(&inner, &package_label, nesting + 1),
                Err(reason) => self.yield_found(Found::Failed {
                    label: package_label,
                    reason,
                }),
            }
            return;
        }
        if bytes.len() as u64 > discovery::MAX_CANDIDATE_BYTES {
            return;
        }
        self.yield_found(Found::Candidate(Candidate {
            label: package_label,
            document: bytes,
            files: Vec::new(),
            owed_reason: false,
        }));
    }

    /// The one skill a named `SKILL.md` stands for: its folder's root skill,
    /// and none of the other skills that folder may hold.
    fn read_named_skill(&mut self, tree: &Tree, label: &Path) {
        let root = plan(&tree.entries).into_iter().find_map(|unit| match unit {
            Unit::Skill { manifest, files } if manifest.parent().is_empty() => {
                Some((manifest, files))
            }
            _ => None,
        });
        let Some((manifest, files)) = root else {
            self.yield_found(Found::Failed {
                label: label.to_path_buf(),
                reason: "could not be read from its folder".to_owned(),
            });
            return;
        };
        if let Some(found) = self.load_skill(tree, label, manifest, &files) {
            self.yield_found(found);
        }
    }

    /// Reads one skill folder's document and bundled files. `None` when the
    /// source's byte budget ran out partway, which [`Self::charge`] already
    /// recorded as the reason the read stopped.
    fn load_skill(
        &mut self,
        tree: &Tree,
        label: &Path,
        manifest: &Entry,
        files: &[(String, &Entry)],
    ) -> Option<Found> {
        let skill_label = tree.label(label, manifest);
        let failed = |reason: String| {
            Some(Found::Failed {
                label: skill_label.clone(),
                reason,
            })
        };
        if files.len() > MAX_BUNDLED_FILES {
            return failed(format!(
                "bundles {} files; a skill may bundle at most {MAX_BUNDLED_FILES}",
                files.len()
            ));
        }
        let declared = files.iter().fold(manifest.size, |total, (_, entry)| {
            total.saturating_add(entry.size)
        });
        if declared > MAX_BUNDLE_BYTES {
            return failed(bundle_too_large());
        }
        if let Some((path, _)) = files.iter().find(|(path, _)| !is_portable_path(path)) {
            return failed(format!(
                "bundles {path:?}, a path that cannot be stored on every platform"
            ));
        }
        if let Some(path) = colliding_path(files) {
            return failed(format!(
                "bundles {path:?}, which clashes with another file on a disk that ignores letter case"
            ));
        }

        let document = match tree.read(manifest, discovery::MAX_CANDIDATE_BYTES) {
            Ok(document) => document,
            Err(reason) => return failed(reason),
        };
        if !self.charge(document.len() as u64) {
            return None;
        }
        let mut remaining = MAX_BUNDLE_BYTES.saturating_sub(document.len() as u64);
        let mut bundled = Vec::with_capacity(files.len());
        for (path, entry) in files {
            // Bounded by what is left of the skill's budget, not by the
            // entry's declared size: a zip header is attacker-controlled and
            // may understate what it decompresses to.
            let bytes = match tree.read(entry, remaining) {
                Ok(bytes) => bytes,
                Err(reason) => return failed(format!("bundled file {path:?} {reason}")),
            };
            if !self.charge(bytes.len() as u64) {
                return None;
            }
            remaining -= bytes.len() as u64;
            bundled.push((path.clone(), bytes));
        }
        Some(Found::Candidate(Candidate {
            label: skill_label,
            document,
            files: bundled,
            owed_reason: true,
        }))
    }
}

fn bundle_too_large() -> String {
    format!(
        "is larger than the {} MB a skill may bundle",
        MAX_BUNDLE_BYTES / (1024 * 1024)
    )
}

/// What a tree of entries splits into.
enum Unit<'a> {
    /// A folder holding `SKILL.md`, with every file it bundles.
    Skill {
        manifest: &'a Entry,
        files: Vec<(String, &'a Entry)>,
    },
    /// A loose `.md` outside every skill folder.
    Document(&'a Entry),
    /// A `.skill` file.
    Package(&'a Entry),
}

/// Splits entries into skills — see the module doc comment for the rules.
/// Units come back in entry order (entries are sorted), so a name declared
/// twice in one source resolves the same way on every platform.
fn plan(entries: &[Entry]) -> Vec<Unit<'_>> {
    let mut roots: Vec<(&[String], &Entry)> = Vec::new();
    for entry in entries {
        if entry.plain && is_skill_file_name(entry.leaf()) {
            let parent = entry.parent();
            match roots.iter_mut().find(|(root, _)| *root == parent) {
                // Two case spellings in one folder: the specified `SKILL.md`
                // is the manifest and the other is an ordinary bundled file.
                Some(held) if entry.leaf() == SKILL_FILE && held.1.leaf() != SKILL_FILE => {
                    held.1 = entry;
                }
                Some(_) => {}
                None => roots.push((parent, entry)),
            }
        }
    }

    // One per root, in `roots` order; `position` is where its manifest sits
    // in `entries`, which is where the skill takes its turn.
    let mut skills: Vec<SkillUnit<'_>> = roots
        .iter()
        .map(|(_, manifest)| SkillUnit {
            position: 0,
            manifest,
            files: Vec::new(),
        })
        .collect();
    let mut units: Vec<(usize, Unit<'_>)> = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        if has_extension_str(entry.leaf(), "skill") {
            units.push((position, Unit::Package(entry)));
            continue;
        }
        let owner = if entry.plain {
            roots
                .iter()
                .enumerate()
                .filter(|(_, (root, _))| entry.path.starts_with(root))
                .max_by_key(|(_, (root, _))| root.len())
        } else {
            None
        };
        match owner {
            Some((slot, (root, manifest))) => {
                let skill = &mut skills[slot];
                if std::ptr::eq(entry, *manifest) {
                    skill.position = position;
                } else if !is_packaging_noise(&entry.path[root.len()..]) {
                    skill
                        .files
                        .push((entry.path[root.len()..].join("/"), entry));
                }
            }
            None if has_extension_str(entry.leaf(), "md") => {
                units.push((position, Unit::Document(entry)));
            }
            None => {}
        }
    }
    units.extend(skills.into_iter().map(|skill| {
        (
            skill.position,
            Unit::Skill {
                manifest: skill.manifest,
                files: skill.files,
            },
        )
    }));
    units.sort_by_key(|(position, _)| *position);
    units.into_iter().map(|(_, unit)| unit).collect()
}

struct SkillUnit<'a> {
    position: usize,
    manifest: &'a Entry,
    files: Vec<(String, &'a Entry)>,
}

/// A file inside a chosen folder or archive.
struct Entry {
    /// Path components below the tree's root.
    path: Vec<String>,
    /// Size from metadata or the zip header. Advisory for an archive: reads
    /// are bounded again as they happen.
    size: u64,
    /// Whether `path` is a plain relative path. A zip entry naming `..` is
    /// not, and may only ever be read as a loose document, which is stored by
    /// its manifest's slug and so has nothing to traverse.
    plain: bool,
    at: Locator,
}

impl Entry {
    fn leaf(&self) -> &str {
        self.path.last().map_or("", String::as_str)
    }

    fn parent(&self) -> &[String] {
        &self.path[..self.path.len().saturating_sub(1)]
    }
}

enum Locator {
    File(PathBuf),
    Zip(usize),
}

trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// A chosen folder or archive, listed but not yet read.
struct Tree {
    entries: Vec<Entry>,
    archive: Option<std::cell::RefCell<zip::ZipArchive<Box<dyn ReadSeek>>>>,
}

impl Tree {
    fn directory(root: &Path, summary: &mut Summary) -> Result<Self, String> {
        let mut entries = Vec::new();
        walk(root, &mut Vec::new(), &mut entries, summary)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self {
            entries,
            archive: None,
        })
    }

    fn archive(reader: Box<dyn ReadSeek>, summary: &mut Summary) -> Result<Self, String> {
        let mut archive =
            zip::ZipArchive::new(reader).map_err(|err| format!("is not a readable zip: {err}"))?;
        let mut entries = Vec::new();
        for index in 0..archive.len() {
            if entries.len() >= MAX_ENTRIES_PER_SOURCE {
                summary.stopped = Some(format!(
                    "stopped after {MAX_ENTRIES_PER_SOURCE} files; choose a smaller archive"
                ));
                break;
            }
            // Raw: listing reads headers only, never decompresses.
            let Ok(file) = archive.by_index_raw(index) else {
                continue;
            };
            if !file.is_file() {
                continue;
            }
            let name = file
                .name()
                .map_err(|error| format!("archive contains an invalid entry name: {error}"))?;
            let (path, plain) = split_entry_name(name.as_ref());
            if path.is_empty() || is_ignored(&path) {
                continue;
            }
            entries.push(Entry {
                path,
                size: file.size(),
                plain,
                at: Locator::Zip(index),
            });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(Self {
            entries,
            archive: Some(std::cell::RefCell::new(archive)),
        })
    }

    /// How a failure inside this tree is labelled: the file's own path for a
    /// folder, `archive!entry` for an archive.
    fn label(&self, source: &Path, entry: &Entry) -> PathBuf {
        match &entry.at {
            Locator::File(path) => path.clone(),
            Locator::Zip(_) => {
                PathBuf::from(format!("{}!{}", source.display(), entry.path.join("/")))
            }
        }
    }

    /// Reads one entry, refusing — without holding more than `limit + 1`
    /// bytes — one that is larger than `limit`.
    fn read(&self, entry: &Entry, limit: u64) -> Result<Vec<u8>, String> {
        match &entry.at {
            Locator::File(path) => read_file_bounded(path, limit),
            Locator::Zip(index) => {
                let archive = self
                    .archive
                    .as_ref()
                    .ok_or_else(|| "could not be read".to_owned())?;
                let mut archive = archive.borrow_mut();
                let file = archive
                    .by_index(*index)
                    .map_err(|err| format!("could not be read from the archive: {err}"))?;
                read_limited(file, limit)
            }
        }
    }
}

fn walk(
    directory: &Path,
    prefix: &mut Vec<String>,
    entries: &mut Vec<Entry>,
    summary: &mut Summary,
) -> Result<(), String> {
    let listing = fs::read_dir(directory).map_err(|err| format!("could not be read: {err}"))?;
    let mut children: Vec<(String, fs::FileType, PathBuf)> = listing
        .filter_map(Result::ok)
        .filter_map(|child| {
            let kind = child.file_type().ok()?;
            let name = child.file_name().into_string().ok()?;
            Some((name, kind, child.path()))
        })
        .collect();
    children.sort_by(|left, right| left.0.cmp(&right.0));

    for (name, kind, path) in children {
        if summary.stopped.is_some() {
            return Ok(());
        }
        // A symlink is skipped rather than followed: following one turns a
        // bounded walk into a potentially cyclic one, and would store a file
        // from outside the folder the user chose.
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if prefix.len() >= MAX_SEARCH_DEPTH || IGNORED_DIRECTORIES.contains(&name.as_str()) {
                continue;
            }
            prefix.push(name);
            let walked = walk(&path, prefix, entries, summary);
            prefix.pop();
            walked?;
            continue;
        }
        if !kind.is_file() {
            continue;
        }
        if entries.len() >= MAX_ENTRIES_PER_SOURCE {
            summary.stopped = Some(format!(
                "stopped after {MAX_ENTRIES_PER_SOURCE} files; choose a smaller folder"
            ));
            return Ok(());
        }
        let mut components = prefix.clone();
        components.push(name);
        if is_ignored(&components) {
            continue;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        entries.push(Entry {
            path: components,
            size: metadata.len(),
            plain: true,
            at: Locator::File(path),
        });
    }
    Ok(())
}

/// Splits a zip entry name on either separator. Empty and `.` components are
/// dropped; a `..` or a drive prefix makes the path not plain.
fn split_entry_name(name: &str) -> (Vec<String>, bool) {
    let mut plain = true;
    let mut path = Vec::new();
    for component in name.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                plain = false;
                path.push(component.to_owned());
            }
            _ => {
                if component.contains(':') {
                    plain = false;
                }
                path.push(component.to_owned());
            }
        }
    }
    (path, plain)
}

/// Entries that are never content anywhere: noise directories, and the
/// `.DS_Store` / `._*` / `Thumbs.db` files operating systems leave behind.
fn is_ignored(path: &[String]) -> bool {
    let Some((leaf, parents)) = path.split_last() else {
        return true;
    };
    parents
        .iter()
        .any(|part| IGNORED_DIRECTORIES.contains(&part.as_str()))
        || leaf == ".DS_Store"
        || leaf == "Thumbs.db"
        || leaf.starts_with("._")
}

/// What skill-creator's packager leaves out of a `.skill`, applied to a
/// skill's own files so importing a skill folder stores what packaging it
/// would have: `*.pyc`, and an `evals/` directory at the skill's root.
fn is_packaging_noise(relative: &[String]) -> bool {
    (relative.len() > 1 && relative[0] == "evals")
        || relative
            .last()
            .is_some_and(|leaf| has_extension_str(leaf, "pyc"))
}

/// Whether every component of a bundled path is a file name that Windows,
/// macOS, and Linux can all store — the skill library is per device, but a
/// skill written on one platform is imported on all three.
fn is_portable_path(path: &str) -> bool {
    if path.split('/').count() > MAX_BUNDLED_PATH_DEPTH {
        return false;
    }
    path.split('/').all(|component| {
        let stem = component.split('.').next().unwrap_or(component);
        !component.is_empty()
            && component != "."
            && component != ".."
            && component.len() <= 255
            && !component.ends_with(['.', ' '])
            && !component
                .chars()
                .any(|character| character.is_control() || "<>:\"|?*\\".contains(character))
            && !is_reserved_device_name(stem)
    })
}

/// The first bundled path that would land on the same file as another — or
/// on a folder another path needs — on a disk that ignores letter case, as
/// Windows and macOS disks do by default. `SKILL.md` is counted, so a bundled
/// `skill.md` cannot overwrite the manifest the import just parsed.
fn colliding_path<'a>(files: &'a [(String, &Entry)]) -> Option<&'a str> {
    let mut seen_files: HashSet<String> = HashSet::from([SKILL_FILE.to_lowercase()]);
    let mut seen_folders: HashSet<String> = HashSet::new();
    for (path, _) in files {
        let folded = path.to_lowercase();
        if seen_folders.contains(&folded) || !seen_files.insert(folded.clone()) {
            return Some(path);
        }
        for (separator, _) in folded.match_indices('/') {
            let folder = &folded[..separator];
            if seen_files.contains(folder) {
                return Some(path);
            }
            seen_folders.insert(folder.to_owned());
        }
    }
    None
}

fn is_reserved_device_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.as_bytes()[3].is_ascii_digit())
}

/// Whether a file name is the skill manifest. Case-insensitive: the
/// specification's reference loader accepts `skill.md`, claude.ai's help
/// pages spell it that way, and some libraries write `Skill.md`.
fn is_skill_file_name(name: &str) -> bool {
    name.eq_ignore_ascii_case(SKILL_FILE)
}

fn is_skill_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(is_skill_file_name)
}

/// Whether `folder` is named for the skill `document` declares — both
/// compared as slugs, so `My Skill/` and `name: my-skill` agree.
fn names_skill(folder: &Path, document: &[u8]) -> bool {
    let Some(folder_name) = folder.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let (Ok(folder_slug), Ok((parsed, _))) = (
        super::manifest::SkillName::new(folder_name),
        discovery::parse_bytes(document),
    ) else {
        return false;
    };
    folder_slug == *parsed.manifest.name()
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|found| found.eq_ignore_ascii_case(extension))
}

fn has_extension_str(name: &str, extension: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, found)| !stem.is_empty() && found.eq_ignore_ascii_case(extension))
}

/// A zip starts with a local file header, or — when it holds nothing — with
/// the end-of-central-directory record. Sniffed rather than trusted to the
/// extension: a `.skill` may be either a zip or plain Markdown.
fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04") || bytes.starts_with(b"PK\x05\x06")
}

fn starts_like_zip(path: &Path) -> Result<bool, String> {
    let mut header = [0_u8; 4];
    let mut file = fs::File::open(path).map_err(|err| format!("could not be read: {err}"))?;
    let mut filled = 0;
    while filled < header.len() {
        match file.read(&mut header[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
            Err(err) => return Err(format!("could not be read: {err}")),
        }
    }
    Ok(is_zip(&header[..filled]))
}

fn read_file_bounded(path: &Path, limit: u64) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|err| format!("could not be read: {err}"))?;
    let declared = file
        .metadata()
        .map_err(|err| format!("could not be read: {err}"))?
        .len();
    if declared > limit {
        return Err(size_refusal(limit));
    }
    read_limited(file, limit)
}

/// Reads at most `limit + 1` bytes, so a source that is larger than it
/// claimed is refused having allocated no more than the bound.
fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|err| format!("could not be read: {err}"))?;
    if bytes.len() as u64 > limit {
        return Err(size_refusal(limit));
    }
    Ok(bytes)
}

fn size_refusal(limit: u64) -> String {
    if limit >= 1024 * 1024 {
        format!(
            "is larger than the {} MB limit",
            limit.div_ceil(1024 * 1024)
        )
    } else {
        format!("is larger than the {} KB limit", limit.div_ceil(1024))
    }
}
