//! What changed in a file since one of its revisions (spec:
//! project-file-patch-inspector, "A revision's changes can be reviewed before
//! it is restored").
//!
//! A revision is compared with the file as it is now: the question a person
//! asks before restoring is "what would this undo". Both sides are bounded
//! before anything is diffed, and the result is refused rather than cut when
//! the edit is too large to read -- a truncated diff looks like the whole
//! change and is not.
//!
//! The same gates as the file view apply, fresh: the path is classified for
//! sensitivity before the file is touched, and [`TrustedRoot`] revalidates
//! containment and root identity on every open.

use std::io::Read;
use std::time::Duration;

use similar::{ChangeTag, TextDiff};
use xfalcon_contracts::RunId;

use super::domain::{ContainmentError, TrustedRoot};
use super::patch::MAX_DIFF_BYTES;
use super::path_policy::RelativeProjectPath;
use super::read::{classify_bytes, ContentKind};
use super::sensitive::{
    evaluate, SecretClass, SecretClassifier, SensitiveDecision, SensitiveReadScope,
};

/// The largest revision, and the largest current file, that is compared. A
/// line diff of anything larger is not something a person reads in a panel.
pub const MAX_REVISION_DIFF_FILE_BYTES: u64 = 1024 * 1024;

/// The most added plus removed lines one diff shows.
pub const MAX_REVISION_DIFF_CHANGED_LINES: usize = 2_000;

/// How long the line diff may search for the smallest answer. Past it the
/// algorithm settles for a larger, still correct, diff instead of spending
/// unbounded time on a pathological pair.
const DIFF_TIMEOUT: Duration = Duration::from_millis(500);

/// Why a revision's changes are not shown line by line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionDiffLimit {
    /// The revision or the current file is over [`MAX_REVISION_DIFF_FILE_BYTES`].
    FileSize,
    /// The edit is over [`MAX_REVISION_DIFF_CHANGED_LINES`], or its rendered
    /// diff is over [`MAX_DIFF_BYTES`].
    ChangeSize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RevisionDiff {
    /// A unified diff from the revision (`a/`) to the target (`b/`).
    Changes {
        unified_diff: String,
        added_lines: usize,
        removed_lines: usize,
    },
    Unchanged,
    /// One side is binary or not UTF-8.
    NotText,
    TooLarge(RevisionDiffLimit),
    Sensitive(SecretClass),
}

/// What a revision is compared with.
pub enum DiffTarget<L> {
    /// The file as it is now: what restoring the revision would undo.
    CurrentFile,
    /// A newer revision of the same file, by its stored size and a loader:
    /// with the next newer revision, what the one edit after it changed.
    Revision { bytes: u64, load: L },
}

/// Compares a revision of `path` with `target`.
///
/// `revision_bytes` and a target revision's `bytes` are stored sizes, checked
/// before either loader runs: the sensitivity gate and the size bound both
/// answer before a revision is decrypted and inflated, which for a large file
/// is most of the cost.
///
/// A current file that no longer exists compares as empty, so the diff shows
/// what the revision held. Anything else that stops a read is an error, not an
/// empty side: a diff against content that could not be read would claim a
/// change that did not happen.
pub fn revision_diff<E, L1, L2>(
    root: &TrustedRoot,
    classifier: &SecretClassifier,
    path: &RelativeProjectPath,
    run_id: RunId,
    revision_bytes: u64,
    load_revision: L1,
    target: DiffTarget<L2>,
) -> Result<RevisionDiff, E>
where
    E: From<ContainmentError>,
    L1: FnOnce() -> Result<Vec<u8>, E>,
    L2: FnOnce() -> Result<Vec<u8>, E>,
{
    let scope = SensitiveReadScope::new(*root.project_id(), path.clone(), run_id);
    if let SensitiveDecision::RequiresOverride(pending) = evaluate(classifier, scope, None) {
        return Ok(RevisionDiff::Sensitive(pending.class()));
    }
    let too_large = Ok(RevisionDiff::TooLarge(RevisionDiffLimit::FileSize));
    if revision_bytes > MAX_REVISION_DIFF_FILE_BYTES
        || matches!(target, DiffTarget::Revision { bytes, .. } if bytes > MAX_REVISION_DIFF_FILE_BYTES)
    {
        return too_large;
    }
    let revision = load_revision()?;
    if revision.len() as u64 > MAX_REVISION_DIFF_FILE_BYTES {
        return too_large;
    }
    let newer = match target {
        DiffTarget::Revision { load, .. } => {
            let newer = load()?;
            if newer.len() as u64 > MAX_REVISION_DIFF_FILE_BYTES {
                return too_large;
            }
            newer
        }
        DiffTarget::CurrentFile => match read_current(root, path)? {
            Current::Content(content) => content,
            Current::Missing => Vec::new(),
            Current::NotAFile => return Ok(RevisionDiff::NotText),
            Current::TooLarge => return too_large,
        },
    };
    Ok(diff_contents(path.as_str(), &revision, &newer))
}

enum Current {
    Content(Vec<u8>),
    Missing,
    NotAFile,
    TooLarge,
}

fn read_current(
    root: &TrustedRoot,
    path: &RelativeProjectPath,
) -> Result<Current, ContainmentError> {
    let resolved = match root.resolve_existing(path.clone()) {
        Ok(resolved) => resolved,
        Err(ContainmentError::NotFound) => return Ok(Current::Missing),
        Err(error) => return Err(error),
    };
    if !root.file_metadata(&resolved)?.is_file() {
        return Ok(Current::NotAFile);
    }
    let file = root.open_file(&resolved)?;
    if file.metadata().map_err(ContainmentError::Io)?.len() > MAX_REVISION_DIFF_FILE_BYTES {
        return Ok(Current::TooLarge);
    }
    // One past the bound, so a file that grew since the metadata read is
    // refused rather than cut.
    let mut content = Vec::new();
    file.take(MAX_REVISION_DIFF_FILE_BYTES + 1)
        .read_to_end(&mut content)
        .map_err(ContainmentError::Io)?;
    if content.len() as u64 > MAX_REVISION_DIFF_FILE_BYTES {
        return Ok(Current::TooLarge);
    }
    Ok(Current::Content(content))
}

/// The diff itself, over two already-bounded buffers.
pub(super) fn diff_contents(path: &str, revision: &[u8], current: &[u8]) -> RevisionDiff {
    let (ContentKind::Text, ContentKind::Text) =
        (classify_bytes(revision), classify_bytes(current))
    else {
        return RevisionDiff::NotText;
    };
    let (Ok(old), Ok(new)) = (std::str::from_utf8(revision), std::str::from_utf8(current)) else {
        return RevisionDiff::NotText;
    };
    if old == new {
        return RevisionDiff::Unchanged;
    }
    let diff = TextDiff::configure()
        .timeout(DIFF_TIMEOUT)
        .diff_lines(old, new);
    let (mut added_lines, mut removed_lines) = (0usize, 0usize);
    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Insert => added_lines += 1,
            ChangeTag::Delete => removed_lines += 1,
            ChangeTag::Equal => {}
        }
        if added_lines + removed_lines > MAX_REVISION_DIFF_CHANGED_LINES {
            return RevisionDiff::TooLarge(RevisionDiffLimit::ChangeSize);
        }
    }
    let unified_diff = diff
        .unified_diff()
        .context_radius(3)
        .header(&format!("a/{path}"), &format!("b/{path}"))
        .to_string();
    if unified_diff.len() > MAX_DIFF_BYTES {
        return RevisionDiff::TooLarge(RevisionDiffLimit::ChangeSize);
    }
    RevisionDiff::Changes {
        unified_diff,
        added_lines,
        removed_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_line_is_one_removal_and_one_addition_from_the_revision() {
        let diff = diff_contents("notes.txt", b"alpha\nbeta\n", b"alpha\ngamma\n");
        let RevisionDiff::Changes {
            unified_diff,
            added_lines,
            removed_lines,
        } = diff
        else {
            panic!("expected changes, got {diff:?}");
        };
        assert_eq!((added_lines, removed_lines), (1, 1));
        assert!(unified_diff.starts_with("--- a/notes.txt\n+++ b/notes.txt\n"));
        assert!(unified_diff.contains("\n-beta\n"));
        assert!(unified_diff.contains("\n+gamma\n"));
    }

    #[test]
    fn identical_content_is_unchanged() {
        assert_eq!(
            diff_contents("a.txt", b"same\n", b"same\n"),
            RevisionDiff::Unchanged
        );
    }

    #[test]
    fn binary_or_non_utf8_on_either_side_is_not_text() {
        assert_eq!(
            diff_contents("a.bin", b"a\0b", b"text"),
            RevisionDiff::NotText
        );
        assert_eq!(
            diff_contents("a.txt", b"text", &[0xff, 0xfe]),
            RevisionDiff::NotText
        );
    }

    #[test]
    fn an_edit_over_the_line_limit_is_refused_rather_than_cut() {
        let old = "a\n".repeat(MAX_REVISION_DIFF_CHANGED_LINES);
        let new = "b\n".repeat(MAX_REVISION_DIFF_CHANGED_LINES);
        assert_eq!(
            diff_contents("a.txt", old.as_bytes(), new.as_bytes()),
            RevisionDiff::TooLarge(RevisionDiffLimit::ChangeSize)
        );
    }

    #[test]
    fn a_few_very_long_lines_are_refused_by_the_rendered_size() {
        let old = format!("{}\n", "a".repeat(MAX_DIFF_BYTES));
        let new = format!("{}\n", "b".repeat(MAX_DIFF_BYTES));
        assert_eq!(
            diff_contents("a.txt", old.as_bytes(), new.as_bytes()),
            RevisionDiff::TooLarge(RevisionDiffLimit::ChangeSize)
        );
    }

    #[test]
    fn an_edit_just_inside_the_line_limit_is_shown() {
        let old = "a\n".repeat(MAX_REVISION_DIFF_CHANGED_LINES / 2);
        let new = "b\n".repeat(MAX_REVISION_DIFF_CHANGED_LINES / 2);
        let diff = diff_contents("a.txt", old.as_bytes(), new.as_bytes());
        assert!(
            matches!(
                diff,
                RevisionDiff::Changes {
                    added_lines: 1_000,
                    removed_lines: 1_000,
                    ..
                }
            ),
            "{diff:?}"
        );
    }
}
