//! One account's imported skill library, on this device.
//!
//! One directory per account, `<app_local_data>/accounts/acct-<sha256>/skills`,
//! holding one `<slug>/` folder per skill — its `SKILL.md` and every file it
//! bundles — plus a `state.json` naming the ones the user turned off. It sits
//! beside the account's other local data and under the same directory name
//! (`account_directory_name`), so a second person signing in to this device
//! neither sees nor runs with the first person's skills, and a skill's
//! instructions — which steer every run that uses it — are only ever the
//! signed-in person's own.
//!
//! # The device-wide library earlier versions kept
//!
//! Before skills were per account they lived in `<app_local_data>/skills`,
//! shared by everyone who signed in. [`SkillStore::open`] moves that library,
//! whole and once, into the first account that opens its own library while
//! it has none. Every account could already see those skills, so handing
//! them to one account exposes nothing new; leaving them behind would lose
//! them, and copying them to every account would keep them shared forever.
//!
//! # Why the files are stored verbatim
//!
//! Import writes exactly the bytes it read, for `SKILL.md` and for every
//! bundled file. Re-serializing the parsed manifest would make the stored
//! file differ from the one the user chose, make the content hash describe
//! our rendering rather than their document, and quietly drop the frontmatter
//! keys this parser ignores. The file is the artifact; the manifest is only
//! our reading of it.
//!
//! # A re-import replaces the whole folder
//!
//! A skill's folder is written complete in the account's `.skills-staging`
//! and then renamed into place, so a run never reads a half-written skill and
//! an updated skill never keeps a bundled file its new version dropped.
//!
//! # Disabled is not deleted
//!
//! `state.json` holds slugs, not skills, so a disabled skill still has its
//! file and a re-import of the same name stays disabled — the user's decision
//! survives an update to the content. Deleting forgets the slug, so importing
//! it again later starts enabled.

use std::collections::BTreeSet;
use std::fs;
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use super::discovery::{self, DiscoveredSkill, DiscoveryError, SkillSource};
use super::manifest::SkillName;
use super::package::{self, Candidate, Found, MAX_BUNDLED_FILES, SKILL_FILE};
use crate::security::account_cleanup::{account_directory_name, ValidatedAccountId};

/// Where every account's library sat before libraries were per account.
pub(crate) const DEVICE_LIBRARY: &str = "skills";

/// The account's library folder and its staging folder, both directly under
/// the account directory. Named here so account cleanup removes them.
pub(crate) const ACCOUNT_LIBRARY_DIR: &str = "skills";
pub(crate) const ACCOUNT_STAGING_DIR: &str = ".skills-staging";

const STATE_FILE: &str = "state.json";
/// A disposable staging folder older than this was left by an import that
/// crashed; one younger may belong to an import still running in another window.
const ABANDONED_STAGING_AGE: Duration = Duration::from_secs(60 * 60);
const STAGING_ID_HEX_CHARS: usize = 32;

/// A cataloged skill plus whether runs are told about it.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredSkill {
    pub discovered: DiscoveredSkill,
    pub enabled: bool,
}

impl StoredSkill {
    #[must_use]
    pub fn name(&self) -> &str {
        self.discovered.manifest().name().as_str()
    }

    #[must_use]
    pub fn description(&self) -> &str {
        self.discovered.manifest().description()
    }
}

/// One chosen file that did not import, and the reason to show for it.
#[derive(Clone, Debug, PartialEq)]
pub struct SkillImportFailure {
    pub path: PathBuf,
    pub reason: String,
}

/// The outcome of one import. Never an `Err` for a bad file: a selection of
/// five where one is malformed must import four, and the caller needs both
/// halves of that answer.
#[derive(Debug, Default)]
pub struct SkillImportOutcome {
    pub imported: Vec<StoredSkill>,
    pub failed: Vec<SkillImportFailure>,
}

#[derive(Debug, thiserror::Error)]
pub enum SkillStoreError {
    #[error(transparent)]
    Discovery(#[from] DiscoveryError),
    #[error("no skill named {0:?} is imported")]
    UnknownSkill(String),
    #[error("skill {0:?} is disabled")]
    DisabledSkill(String),
    #[error("skills directory {path:?} is unusable: {source}")]
    Storage {
        path: PathBuf,
        source: std::io::Error,
    },
}

impl SkillStoreError {
    /// A category safe to log. The `Display` text carries this device's
    /// app-data path, and with it the user's account name, which the
    /// diagnostics log must not (`local-diagnostics`).
    #[must_use]
    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Discovery(_) => "discovery",
            Self::UnknownSkill(_) => "unknown_skill",
            Self::DisabledSkill(_) => "disabled_skill",
            Self::Storage { source, .. } => match source.kind() {
                std::io::ErrorKind::NotFound => "storage_not_found",
                std::io::ErrorKind::PermissionDenied => "storage_permission_denied",
                _ => "storage_io",
            },
        }
    }
}

/// One file a skill bundles beside its `SKILL.md`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundledFile {
    /// `/`-separated, relative to the skill's folder — the spelling a
    /// `SKILL.md` uses when it points at `references/guide.md`.
    pub path: String,
    pub bytes: u64,
}

/// Why a bundled file could not be read for a run.
#[derive(Debug, thiserror::Error)]
pub enum SkillFileError {
    #[error(transparent)]
    Store(#[from] SkillStoreError),
    #[error("skill {skill:?} bundles no file {path:?}")]
    UnknownFile { skill: String, path: String },
    #[error("{path:?} is a binary file of {bytes} bytes")]
    Binary { path: String, bytes: u64 },
    #[error("offset {offset} is past the end of {path:?} ({bytes} bytes)")]
    OffsetPastEnd {
        path: String,
        offset: u64,
        bytes: u64,
    },
}

/// One page of a bundled text file.
#[derive(Debug, Eq, PartialEq)]
pub struct FilePage {
    pub content: String,
    /// The whole file's size.
    pub bytes: u64,
    /// Where the next page starts, when text remains.
    pub next_offset: Option<u64>,
}

/// Renames a directory, retrying briefly when Windows refuses because a file
/// inside it is open. Antivirus scanners and the search indexer open files
/// the moment they are written, and a directory holding an open file cannot
/// be renamed there. Every other error is returned at once.
fn rename_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt: u32 = 0;
    loop {
        match fs::rename(from, to) {
            Err(err)
                if cfg!(windows)
                    && err.kind() == std::io::ErrorKind::PermissionDenied
                    && attempt < 5 =>
            {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(40 * u64::from(attempt)));
            }
            other => return other,
        }
    }
}

fn is_disposable_staging_name(name: &std::ffi::OsStr) -> bool {
    let Some(name) = name.to_str() else {
        return false;
    };
    let Some((kind, id)) = name.split_once('-') else {
        return false;
    };
    matches!(kind, "import" | "obsolete")
        && id.len() == STAGING_ID_HEX_CHARS
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_continuation_byte(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

/// Writes a skill's folder in `staging`: `SKILL.md` and every bundled file,
/// each synced, so the rename that publishes it publishes complete files.
fn write_folder(staging: &Path, candidate: &Candidate) -> std::io::Result<()> {
    fs::create_dir_all(staging)?;
    write_synced(&staging.join(SKILL_FILE), &candidate.document)?;
    for (relative, bytes) in &candidate.files {
        // Portable, relative, and `..`-free by construction in `package`;
        // joined component by component so no separator is reinterpreted.
        let destination = relative
            .split('/')
            .fold(staging.to_path_buf(), |path, component| {
                path.join(component)
            });
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        write_synced(&destination, bytes)?;
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = fs::File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

#[cfg(test)]
mod staging_cleanup_tests {
    use super::*;

    fn store() -> (tempfile::TempDir, SkillStore) {
        let root = tempfile::tempdir().expect("app data");
        let account = ValidatedAccountId::parse("staging-cleanup-test").expect("account");
        let store = SkillStore::open(root.path(), &account);
        (root, store)
    }

    fn abandoned_now() -> SystemTime {
        SystemTime::now() + ABANDONED_STAGING_AGE + Duration::from_secs(1)
    }

    fn staging_dir(store: &SkillStore, name: &str) -> PathBuf {
        store.staging.join(name)
    }

    #[test]
    fn cleanup_removes_only_old_disposable_folders_and_preserves_recovery_data() {
        let (_root, store) = store();
        let stale_import = staging_dir(&store, "import-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let stale_obsolete = staging_dir(&store, "obsolete-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
        let recovery_copy = staging_dir(&store, "replaced-cccccccccccccccccccccccccccccccc");
        let unrelated = staging_dir(&store, "user-data");
        for path in [&stale_import, &stale_obsolete, &recovery_copy, &unrelated] {
            fs::create_dir_all(path).expect("create staging entry");
            fs::write(path.join("SKILL.md"), b"previous skill bytes").expect("write data");
        }

        store.clear_abandoned_staging_at(abandoned_now(), |path| fs::remove_dir_all(path));

        assert!(!stale_import.exists(), "abandoned imports are disposable");
        assert!(!stale_obsolete.exists(), "published backups are disposable");
        assert_eq!(
            fs::read(recovery_copy.join("SKILL.md")).expect("recovery copy survives"),
            b"previous skill bytes"
        );
        assert!(unrelated.join("SKILL.md").exists(), "unknown data survives");
    }

    #[test]
    fn failed_restore_keeps_the_only_previous_skill_copy_across_cleanup() {
        use super::super::package::Candidate;

        let (_root, store) = store();
        let folder = store.root.join("ida-viz");
        fs::create_dir_all(&folder).expect("previous skill folder");
        fs::write(folder.join(SKILL_FILE), b"previous skill bytes").expect("previous skill");
        let candidate = Candidate {
            label: PathBuf::from("replacement.skill"),
            document: b"replacement skill bytes".to_vec(),
            files: Vec::new(),
            owed_reason: true,
        };
        let mut rename_count = 0;

        let result = store.publish_with_rename(&folder, &candidate, |from, to| {
            rename_count += 1;
            if (2..=3).contains(&rename_count) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "injected rename failure",
                ));
            }
            fs::rename(from, to)
        });

        assert!(result.is_err(), "the replacement does not publish");
        assert!(!folder.exists(), "restoring the prior location failed");
        let backup = fs::read_dir(&store.staging)
            .expect("staging directory")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("replaced-"))
            })
            .expect("recovery backup remains");
        store.clear_abandoned_staging_at(abandoned_now(), |path| fs::remove_dir_all(path));

        assert_eq!(
            fs::read(backup.join(SKILL_FILE)).expect("previous skill survives cleanup"),
            b"previous skill bytes"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_does_not_follow_staging_symlinks() {
        use std::os::unix::fs::symlink;

        let (root, store) = store();
        let target = root.path().join("outside");
        fs::create_dir_all(&target).expect("outside target");
        fs::write(target.join("keep.txt"), b"keep").expect("outside data");
        fs::create_dir_all(&store.staging).expect("staging directory");
        let link = staging_dir(&store, "import-dddddddddddddddddddddddddddddddd");
        symlink(&target, &link).expect("staging symlink");

        store.clear_abandoned_staging_at(abandoned_now(), |path| fs::remove_dir_all(path));

        assert!(fs::symlink_metadata(&link)
            .expect("symlink remains")
            .file_type()
            .is_symlink());
        assert_eq!(
            fs::read(target.join("keep.txt")).expect("target survives"),
            b"keep"
        );
    }

    #[test]
    fn cleanup_keeps_going_when_removal_fails() {
        let (_root, store) = store();
        let stale = staging_dir(&store, "obsolete-eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee");
        fs::create_dir_all(&stale).expect("stale directory");
        fs::write(stale.join("old.txt"), b"old").expect("stale data");
        let mut attempted = Vec::new();

        store.clear_abandoned_staging_at(abandoned_now(), |path| {
            attempted.push(path.to_path_buf());
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "injected removal failure",
            ))
        });

        assert_eq!(attempted, vec![stale.clone()]);
        assert!(stale.join("old.txt").exists());
    }
}

/// Every file under a stored skill's folder except its `SKILL.md`, sorted.
/// Symlinks are skipped: import never writes one, so one found here was put
/// there by something else and does not belong to the skill.
fn list_folder(folder: &Path) -> std::io::Result<Vec<BundledFile>> {
    fn visit(directory: &Path, prefix: &str, found: &mut Vec<BundledFile>) -> std::io::Result<()> {
        for child in fs::read_dir(directory)? {
            // Checked before each entry, at every depth, so a folder that
            // somehow outgrew the import bound stops the walk everywhere.
            if found.len() > MAX_BUNDLED_FILES {
                return Ok(());
            }
            let child = child?;
            let kind = child.file_type()?;
            let Ok(name) = child.file_name().into_string() else {
                continue;
            };
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if kind.is_dir() {
                visit(&child.path(), &path, found)?;
            } else if kind.is_file() && path != SKILL_FILE {
                found.push(BundledFile {
                    path,
                    bytes: child.metadata()?.len(),
                });
            }
        }
        Ok(())
    }
    let mut found = Vec::new();
    visit(folder, "", &mut found)?;
    found.sort_by(|left, right| left.path.cmp(&right.path));
    found.truncate(MAX_BUNDLED_FILES);
    Ok(found)
}

/// A path as a model or a `SKILL.md` might spell it — `./references/a.md`,
/// `references\a.md` — in the `/`-separated form [`list_folder`] reports.
/// `None` for anything that climbs out (`..`) or is absolute.
fn normalize_requested(requested: &str) -> Option<String> {
    let trimmed = requested.trim();
    if trimmed.starts_with(['/', '\\']) || trimmed.contains(':') {
        return None;
    }
    let mut components = Vec::new();
    for component in trimmed.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => return None,
            other => components.push(other),
        }
    }
    (!components.is_empty()).then(|| components.join("/"))
}

pub struct SkillStore {
    root: PathBuf,
    /// Outside `root`, so a folder being written is never discovered as a
    /// skill, and on the same volume, so publishing it is a rename.
    staging: PathBuf,
}

impl SkillStore {
    /// The library of `account`, adopting the device-wide library of earlier
    /// versions if this account is the first to open one (see the module doc
    /// comment).
    #[must_use]
    pub fn open(app_local_data: &Path, account: &ValidatedAccountId) -> Self {
        let account_root = app_local_data
            .join("accounts")
            .join(account_directory_name(account.as_str()));
        let store = Self {
            root: account_root.join(ACCOUNT_LIBRARY_DIR),
            staging: account_root.join(ACCOUNT_STAGING_DIR),
        };
        store.adopt_device_library(&app_local_data.join(DEVICE_LIBRARY));
        store
    }

    /// Moves the device-wide library into this account when this account has
    /// no library yet. One rename, so the library arrives whole or not at
    /// all. Best effort by design: a failure leaves the library where it was,
    /// logged, and the next open tries again — the account simply starts with
    /// what it has, which is never another account's skills.
    fn adopt_device_library(&self, device_library: &Path) {
        if !device_library.is_dir() || self.root.exists() {
            return;
        }
        let adopted = self
            .root
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| rename_dir(device_library, &self.root));
        match adopted {
            Ok(()) => tracing::info!(
                "skills: moved the device-wide skill library into the signed-in account"
            ),
            // Another open adopted it first, between the check and the rename.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => tracing::warn!(
                %error,
                "skills: the device-wide skill library could not be moved into the signed-in account; retrying on next open"
            ),
        }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn storage_error(&self, source: std::io::Error) -> SkillStoreError {
        SkillStoreError::Storage {
            path: self.root.clone(),
            source,
        }
    }

    /// The slugs the user turned off. A missing or malformed state file reads
    /// as "nothing disabled" — losing the file must not hide a skill, and the
    /// next toggle rewrites it.
    fn disabled(&self) -> BTreeSet<String> {
        fs::read(self.root.join(STATE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<StateDocument>(&bytes).ok())
            .map(|document| document.disabled)
            .unwrap_or_default()
    }

    fn write_disabled(&self, disabled: &BTreeSet<String>) -> Result<(), SkillStoreError> {
        let document = StateDocument {
            disabled: disabled.clone(),
        };
        let bytes = serde_json::to_vec_pretty(&document)
            .map_err(|err| self.storage_error(std::io::Error::other(err)))?;
        let path = self.root.join(STATE_FILE);
        let parent = path.parent().unwrap_or(&self.root).to_path_buf();
        fs::create_dir_all(&parent).map_err(|err| self.storage_error(err))?;
        let temporary = parent.join(".state.json.tmp");
        {
            let mut file = fs::File::create(&temporary).map_err(|err| self.storage_error(err))?;
            file.write_all(&bytes)
                .map_err(|err| self.storage_error(err))?;
            file.sync_all().map_err(|err| self.storage_error(err))?;
        }
        if path.exists() {
            fs::remove_file(&path).map_err(|err| self.storage_error(err))?;
        }
        fs::rename(&temporary, &path).map_err(|err| self.storage_error(err))
    }

    /// Every imported skill, metadata only, in discovery's deterministic
    /// order. Bodies are never read here.
    pub fn list(&self) -> Result<Vec<StoredSkill>, SkillStoreError> {
        let disabled = self.disabled();
        Ok(discovery::discover_user(&self.root)?
            .into_iter()
            .map(|discovered| StoredSkill {
                enabled: !disabled.contains(discovered.manifest().name().as_str()),
                discovered,
            })
            .collect())
    }

    /// The subset a run is told about. Separate from [`Self::list`] because
    /// Settings shows everything and a run must see only what is enabled —
    /// filtering at the one call site instead would make forgetting to filter
    /// a silent leak of a skill the user turned off.
    pub fn enabled(&self) -> Result<Vec<StoredSkill>, SkillStoreError> {
        Ok(self
            .list()?
            .into_iter()
            .filter(|skill| skill.enabled)
            .collect())
    }

    /// Imports whatever the user chose: a skill file, a `.skill` or `.zip`
    /// archive, or a folder to search (see `super::package` for how each is
    /// split into skills). A skill that parses is stored under its declared
    /// name's slug with every file it bundles, replacing whatever was there —
    /// name is identity, matching xfalcon-web's import-by-name behavior.
    ///
    /// # What gets reported
    ///
    /// A file the user **named** that would not import is reported with its
    /// reason, and so is any `SKILL.md` found in a folder or archive: both
    /// declare themselves skills. A loose `.md` **found** beside skills that
    /// would not parse is skipped in silence — a `README.md` is not an error,
    /// and reporting it would bury the reasons that matter. A folder or
    /// archive that yielded nothing is reported once, since otherwise choosing
    /// it would look like it worked, and so is a read a bound cut short.
    pub fn import(&self, paths: &[PathBuf]) -> SkillImportOutcome {
        self.clear_abandoned_staging();
        let mut outcome = SkillImportOutcome::default();
        let disabled = self.disabled();
        let mut library = self.stored_slugs();
        for path in paths {
            let mut stored_any = false;
            let mut reported_any = false;
            let read = package::read(path, &mut |found| match found {
                Found::Candidate(candidate) => {
                    match self.store_candidate(&candidate, &disabled, &mut library) {
                        Ok(skill) => {
                            stored_any = true;
                            outcome.imported.push(skill);
                        }
                        Err(reason) if candidate.owed_reason => {
                            reported_any = true;
                            outcome.failed.push(SkillImportFailure {
                                path: candidate.label,
                                reason,
                            });
                        }
                        Err(_) => {}
                    }
                }
                Found::Failed { label, reason } => {
                    reported_any = true;
                    outcome.failed.push(SkillImportFailure {
                        path: label,
                        reason,
                    });
                }
            });
            let summary = match read {
                Ok(summary) => summary,
                Err(reason) => {
                    outcome.failed.push(SkillImportFailure {
                        path: path.clone(),
                        reason,
                    });
                    continue;
                }
            };
            if summary.yielded == 0 {
                outcome.failed.push(SkillImportFailure {
                    path: path.clone(),
                    reason: "no skills were found here".to_owned(),
                });
            } else if summary.searched && !stored_any && !reported_any {
                outcome.failed.push(SkillImportFailure {
                    path: path.clone(),
                    reason: "nothing here parsed as a skill".to_owned(),
                });
            }
            if let Some(reason) = summary.stopped {
                outcome.failed.push(SkillImportFailure {
                    path: path.clone(),
                    reason,
                });
            }
        }
        outcome.imported = keep_last_per_name(outcome.imported);
        // Reasons only: they name what was wrong with a skill, never where on
        // this device it was chosen from (`local-diagnostics`).
        for failure in &outcome.failed {
            tracing::warn!(reason = %failure.reason, "skills: a chosen skill was not imported");
        }
        tracing::info!(
            sources = paths.len(),
            imported = outcome.imported.len(),
            failed = outcome.failed.len(),
            "skills: import finished"
        );
        outcome
    }

    /// The slugs the library holds now, by folder name. Read once per import
    /// so the library cap below does not rescan per skill.
    fn stored_slugs(&self) -> BTreeSet<String> {
        fs::read_dir(&self.root)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
                    .filter_map(|entry| entry.file_name().into_string().ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn store_candidate(
        &self,
        candidate: &Candidate,
        disabled: &BTreeSet<String>,
        library: &mut BTreeSet<String>,
    ) -> Result<StoredSkill, String> {
        let (parsed, content_hash) =
            discovery::parse_bytes(&candidate.document).map_err(|err| err.to_string())?;
        let name = parsed.manifest.name().clone();
        // Discovery refuses a library above its bound outright, so a skill
        // past it would not add one skill — it would hide all of them.
        if !library.contains(name.as_str()) && library.len() >= discovery::MAX_SKILLS_PER_SOURCE {
            return Err(format!(
                "was not added: the library already holds its limit of {} skills",
                discovery::MAX_SKILLS_PER_SOURCE
            ));
        }
        let folder = self.root.join(name.as_str());
        self.publish(&folder, candidate)
            .map_err(|err| format!("could not be stored: {err}"))?;
        library.insert(name.as_str().to_owned());
        Ok(StoredSkill {
            enabled: !disabled.contains(name.as_str()),
            discovered: DiscoveredSkill::stored(
                parsed.manifest,
                SkillSource::User,
                content_hash,
                folder.join(SKILL_FILE),
            ),
        })
    }

    /// Writes the skill's folder in staging, then swaps it in for `folder`.
    ///
    /// The old folder is moved aside rather than deleted first, so a failed
    /// swap puts it back and the skill the user had is never lost to an
    /// import that did not land. Windows `fs::rename` will not replace an
    /// existing directory, which is why the old one moves out of the way
    /// instead of being overwritten.
    fn publish(&self, folder: &Path, candidate: &Candidate) -> std::io::Result<()> {
        self.publish_with_rename(folder, candidate, rename_dir)
    }

    fn publish_with_rename(
        &self,
        folder: &Path,
        candidate: &Candidate,
        mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        let unique = uuid::Uuid::new_v4().simple().to_string();
        let staged = self.staging.join(format!("import-{unique}"));
        if let Err(err) = write_folder(&staged, candidate) {
            let _ = fs::remove_dir_all(&staged);
            return Err(err);
        }
        if let Err(err) = fs::create_dir_all(&self.root) {
            let _ = fs::remove_dir_all(&staged);
            return Err(err);
        }
        let replaced = self.staging.join(format!("replaced-{unique}"));
        let had_previous = match rename(folder, &replaced) {
            Ok(()) => true,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => false,
            Err(err) => {
                let _ = fs::remove_dir_all(&staged);
                return Err(err);
            }
        };
        if let Err(err) = rename(&staged, folder) {
            if had_previous {
                if let Err(restore) = rename(&replaced, folder) {
                    // Keep the only remaining copy under `replaced-*`. The
                    // age sweep deliberately preserves these uncertain backups.
                    tracing::error!(
                        error = %restore,
                        "skills: an import failed and the previous version of the skill could not be put back"
                    );
                }
            }
            let _ = fs::remove_dir_all(&staged);
            return Err(err);
        }
        if had_previous {
            // The new folder is already in place, so this backup is now
            // disposable. Mark it before deletion so a crash or failed delete
            // cannot make the sweep confuse it with a recovery copy.
            let obsolete = self.staging.join(format!("obsolete-{unique}"));
            match rename(&replaced, &obsolete) {
                Ok(()) => {
                    if let Err(error) = fs::remove_dir_all(&obsolete) {
                        tracing::debug!(%error, "skills: obsolete replaced skill folder is left for the next sweep");
                    }
                }
                Err(error) => {
                    tracing::debug!(%error, "skills: redundant replaced skill folder could not be marked obsolete");
                }
            }
        }
        Ok(())
    }

    /// Removes what a crashed import left in staging. Best effort by design:
    /// it is garbage collection, not state, and a folder it cannot remove
    /// now is retried by the next import.
    fn clear_abandoned_staging(&self) {
        self.clear_abandoned_staging_at(SystemTime::now(), |path| fs::remove_dir_all(path));
    }

    fn clear_abandoned_staging_at(
        &self,
        now: SystemTime,
        mut remove_dir_all: impl FnMut(&Path) -> std::io::Result<()>,
    ) {
        let Ok(entries) = fs::read_dir(&self.staging) else {
            return;
        };
        for entry in entries.filter_map(Result::ok) {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if !file_type.is_dir() || !is_disposable_staging_name(&entry.file_name()) {
                continue;
            }
            let abandoned = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > ABANDONED_STAGING_AGE);
            if abandoned {
                let _ = remove_dir_all(&entry.path());
            }
        }
    }

    pub fn set_enabled(&self, name: &str, enabled: bool) -> Result<StoredSkill, SkillStoreError> {
        let mut skill = self.find(name)?;
        let mut disabled = self.disabled();
        let changed = if enabled {
            disabled.remove(skill.name())
        } else {
            disabled.insert(skill.name().to_owned())
        };
        if changed {
            self.write_disabled(&disabled)?;
        }
        skill.enabled = enabled;
        Ok(skill)
    }

    /// Removes the skill's directory and forgets its disabled state, so a
    /// later import of the same name starts enabled.
    pub fn delete(&self, name: &str) -> Result<(), SkillStoreError> {
        let skill = self.find(name)?;
        let directory = skill
            .discovered
            .path()
            .parent()
            .ok_or_else(|| SkillStoreError::UnknownSkill(name.to_owned()))?
            .to_path_buf();
        fs::remove_dir_all(&directory).map_err(|source| SkillStoreError::Storage {
            path: directory,
            source,
        })?;
        let mut disabled = self.disabled();
        if disabled.remove(skill.name()) {
            self.write_disabled(&disabled)?;
        }
        Ok(())
    }

    /// The files a stored skill bundles beside its `SKILL.md`, whether or
    /// not it is enabled — Settings describes every skill it lists.
    pub fn bundled_files(&self, skill: &StoredSkill) -> Result<Vec<BundledFile>, SkillStoreError> {
        let folder = skill_folder(skill);
        list_folder(folder).map_err(|source| SkillStoreError::Storage {
            path: folder.to_path_buf(),
            source,
        })
    }

    /// One enabled skill's body and bundled files, from one lookup — the only
    /// path in this module that reads instructions, and the one
    /// `__internal__get_skill` serves.
    ///
    /// A disabled skill is refused rather than returned: a skill the model was
    /// never told about must not become reachable by guessing its name.
    pub fn instructions_and_files(
        &self,
        name: &str,
    ) -> Result<(String, Vec<BundledFile>), SkillStoreError> {
        let skill = self.find_enabled(name)?;
        let instructions = discovery::load_instructions(&skill.discovered)?;
        Ok((instructions, self.bundled_files(&skill)?))
    }

    /// One page of a bundled file of an enabled skill, as text: at most
    /// `page_bytes` starting at byte `offset`, cut on character boundaries.
    /// An offset inside a character moves back to that character's start.
    ///
    /// The requested path is matched against the skill's own listing rather
    /// than joined onto its folder, so no spelling of a path — `..`, an
    /// absolute path, a symlink planted in the folder — can reach a file the
    /// listing does not name. Only the page is read, so a large file costs a
    /// page of memory. Bytes that are not UTF-8, or that hold a NUL, are
    /// refused as binary: they mean nothing in a model's context.
    pub fn read_file(
        &self,
        name: &str,
        requested: &str,
        offset: u64,
        page_bytes: usize,
    ) -> Result<FilePage, SkillFileError> {
        let skill = self.find_enabled(name)?;
        let unknown = || SkillFileError::UnknownFile {
            skill: skill.name().to_owned(),
            path: requested.to_owned(),
        };
        let wanted = normalize_requested(requested).ok_or_else(unknown)?;
        let file = self
            .bundled_files(&skill)?
            .into_iter()
            .find(|file| file.path == wanted)
            .ok_or_else(unknown)?;
        if offset > file.bytes {
            return Err(SkillFileError::OffsetPastEnd {
                path: file.path,
                offset,
                bytes: file.bytes,
            });
        }
        let location = wanted
            .split('/')
            .fold(skill_folder(&skill).to_path_buf(), |path, component| {
                path.join(component)
            });
        let storage = |source: std::io::Error| {
            SkillFileError::Store(SkillStoreError::Storage {
                path: location.clone(),
                source,
            })
        };

        // A window that starts up to three bytes early, so a character the
        // offset lands inside can be read whole, and runs four bytes past
        // the page, so a character the page ends inside can be seen and cut.
        let window_start = offset.saturating_sub(3);
        let lead = usize::try_from(offset - window_start).unwrap_or(0);
        let wanted_bytes = lead + page_bytes + 4;
        let mut handle = fs::File::open(&location).map_err(storage)?;
        handle
            .seek(SeekFrom::Start(window_start))
            .map_err(storage)?;
        let mut window = Vec::with_capacity(wanted_bytes);
        handle
            .take(wanted_bytes as u64)
            .read_to_end(&mut window)
            .map_err(storage)?;

        let binary = || SkillFileError::Binary {
            path: file.path.clone(),
            bytes: file.bytes,
        };
        let mut start = lead.min(window.len());
        while start > 0 && start < window.len() && is_continuation_byte(window[start]) {
            start -= 1;
        }
        let mut end = (start + page_bytes).min(window.len());
        if end < window.len() {
            while end > start && is_continuation_byte(window[end]) {
                end -= 1;
            }
            // A page that backs off to nothing is not text: text makes
            // progress on every page, and an empty one would repeat forever.
            if end == start {
                return Err(binary());
            }
        }
        let slice = &window[start..end];
        if slice.contains(&0) {
            return Err(binary());
        }
        let content = std::str::from_utf8(slice).map_err(|_| binary())?.to_owned();
        Ok(FilePage {
            content,
            bytes: file.bytes,
            next_offset: (end < window.len()).then(|| window_start + end as u64),
        })
    }

    fn find_enabled(&self, name: &str) -> Result<StoredSkill, SkillStoreError> {
        let skill = self.find(name)?;
        if !skill.enabled {
            return Err(SkillStoreError::DisabledSkill(skill.name().to_owned()));
        }
        Ok(skill)
    }

    /// Looks a skill up by the name the caller supplied, normalized the same
    /// way import normalized it. A model that asks for `IDA Dashboard` and a
    /// user who imported `ida-dashboard` mean the same skill.
    fn find(&self, name: &str) -> Result<StoredSkill, SkillStoreError> {
        let slug =
            SkillName::new(name).map_err(|_| SkillStoreError::UnknownSkill(name.to_owned()))?;
        self.list()?
            .into_iter()
            .find(|skill| skill.name() == slug.as_str())
            .ok_or_else(|| SkillStoreError::UnknownSkill(name.to_owned()))
    }
}

/// Collapses an import's results to one entry per declared name — the last
/// file stored for that name, at the position the name was first seen.
///
/// Name is identity, so two files in one library declaring the same name are
/// one skill: the second overwrote the first, and the last one written is the
/// one that exists. Reporting both would tell the user "imported 2 skills"
/// about a library that gained one — the same class of lie as a folder that
/// yielded nothing reporting success.
///
/// Linear scan rather than a map: a source yields a bounded number of
/// candidates, and duplicate names are the rare case, so the index this would
/// need costs more to read than it saves.
fn keep_last_per_name(imported: Vec<StoredSkill>) -> Vec<StoredSkill> {
    let mut kept: Vec<StoredSkill> = Vec::with_capacity(imported.len());
    for skill in imported {
        match kept.iter().position(|held| held.name() == skill.name()) {
            Some(index) => kept[index] = skill,
            None => kept.push(skill),
        }
    }
    kept
}

#[derive(serde::Deserialize, serde::Serialize)]
struct StateDocument {
    #[serde(default)]
    disabled: BTreeSet<String>,
}

/// The folder a stored skill lives in: its `SKILL.md`'s parent, which
/// discovery canonicalized.
fn skill_folder(skill: &StoredSkill) -> &Path {
    skill
        .discovered
        .path()
        .parent()
        .unwrap_or_else(|| skill.discovered.path())
}
