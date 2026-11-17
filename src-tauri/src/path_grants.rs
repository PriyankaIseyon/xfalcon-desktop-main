//! Filesystem paths the renderer may name only by id.
//!
//! The person picks a folder or files in a native dialog, or drops them on the
//! window. Rust records the paths here and hands the webview an opaque id. The
//! commands that act on those paths take the id, never a path, so a compromised
//! renderer can act only on what the person chose, and only for a short while.
//!
//! A grant is not consumed by use: a folder is looked up and then created from,
//! and both steps resolve the same id. A grant expires after [`GRANT_LIFETIME`],
//! and at most [`MAX_LIVE_GRANTS`] of each kind are held, the oldest of that
//! kind evicted first.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use rand::RngCore;

/// How long a choice stays usable: long enough to finish a sheet someone left
/// open, short enough that a grant does not outlive the moment it was made for.
pub(crate) const GRANT_LIFETIME: Duration = Duration::from_secs(15 * 60);

/// Live grants held at once, per kind. A person has one picker or sheet open at a
/// time, so the bound only matters to a caller that issues without limit. It is
/// per kind so a burst of dropped files cannot evict the folder choice a sheet
/// is still using.
pub(crate) const MAX_LIVE_GRANTS: usize = 16;

/// Paths one grant may carry. A selection past this is refused whole, never cut
/// down to a prefix that the person did not choose.
pub(crate) const MAX_PATHS_PER_GRANT: usize = 4096;

/// The refusal for a grant that is unknown, expired, or issued for something
/// else. The text the renderer shows for it.
pub(crate) const EXPIRED_CHOICE: &str = "That choice expired. Choose it again.";

/// What a grant was issued for. A grant resolves only for the kind it was
/// issued as, so an id from the folder picker cannot be used to import skills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GrantKind {
    Folder,
    SkillSources,
}

struct Grant {
    id: String,
    kind: GrantKind,
    paths: Vec<PathBuf>,
    issued_at: Instant,
}

/// The live grants of this process. Managed as `Arc<PathGrants>`; see `lib.rs`.
#[derive(Default)]
pub(crate) struct PathGrants {
    grants: Mutex<VecDeque<Grant>>,
}

impl PathGrants {
    /// Records `paths` as a grant of `kind` and returns its id. `None` when there
    /// are more than [`MAX_PATHS_PER_GRANT`]: nothing is recorded in that case.
    pub(crate) fn issue(&self, kind: GrantKind, paths: Vec<PathBuf>) -> Option<String> {
        self.issue_at(kind, paths, Instant::now())
    }

    /// The paths behind `id`, when it was issued as `kind` and has not expired.
    /// Unknown, expired, and wrong-kind ids all answer `None`.
    pub(crate) fn resolve(&self, kind: GrantKind, id: &str) -> Option<Vec<PathBuf>> {
        self.resolve_at(kind, id, Instant::now())
    }

    /// Forgets every grant. Called when the account they were made under is left.
    pub(crate) fn clear(&self) {
        self.lock().clear();
    }

    fn issue_at(&self, kind: GrantKind, paths: Vec<PathBuf>, now: Instant) -> Option<String> {
        if paths.len() > MAX_PATHS_PER_GRANT {
            return None;
        }
        let id = new_id();
        let mut grants = self.lock();
        prune(&mut grants, now);
        grants.push_back(Grant {
            id: id.clone(),
            kind,
            paths,
            issued_at: now,
        });
        while grants.iter().filter(|grant| grant.kind == kind).count() > MAX_LIVE_GRANTS {
            let Some(oldest) = grants.iter().position(|grant| grant.kind == kind) else {
                break;
            };
            grants.remove(oldest);
        }
        Some(id)
    }

    fn resolve_at(&self, kind: GrantKind, id: &str, now: Instant) -> Option<Vec<PathBuf>> {
        let mut grants = self.lock();
        prune(&mut grants, now);
        grants
            .iter()
            .find(|grant| grant.id == id && grant.kind == kind)
            .map(|grant| grant.paths.clone())
    }

    fn lock(&self) -> MutexGuard<'_, VecDeque<Grant>> {
        // A panic while holding the lock leaves the table as it was: every
        // mutation here is a single push, pop, or retain.
        self.grants.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn prune(grants: &mut VecDeque<Grant>, now: Instant) {
    grants.retain(|grant| now.saturating_duration_since(grant.issued_at) < GRANT_LIFETIME);
}

/// 16 random bytes as 32 lowercase hex characters.
fn new_id() -> String {
    let mut bytes = [0_u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes
        .iter()
        .fold(String::with_capacity(32), |mut id, byte| {
            // Writing to a `String` cannot fail.
            let _ = write!(id, "{byte:02x}");
            id
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(name: &str) -> PathBuf {
        PathBuf::from(format!("/chosen/{name}"))
    }

    /// A grant the test expects to be recorded: every call here is within the limit.
    fn must_issue(
        grants: &PathGrants,
        kind: GrantKind,
        paths: Vec<PathBuf>,
        now: Instant,
    ) -> String {
        grants
            .issue_at(kind, paths, now)
            .expect("within the grant limit")
    }

    #[test]
    fn ids_are_32_lowercase_hex_characters_and_unique() {
        let grants = PathGrants::default();
        let mut seen = std::collections::BTreeSet::new();
        for index in 0..MAX_LIVE_GRANTS {
            let id = must_issue(
                &grants,
                GrantKind::Folder,
                vec![path(&index.to_string())],
                Instant::now(),
            );
            assert_eq!(id.len(), 32);
            assert!(id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
            assert!(seen.insert(id), "ids must not repeat");
        }
    }

    #[test]
    fn a_grant_resolves_only_for_the_kind_it_was_issued_as() {
        let grants = PathGrants::default();
        let now = Instant::now();
        let id = must_issue(&grants, GrantKind::Folder, vec![path("project")], now);

        assert_eq!(
            grants.resolve_at(GrantKind::Folder, &id, now),
            Some(vec![path("project")])
        );
        assert_eq!(grants.resolve_at(GrantKind::SkillSources, &id, now), None);
    }

    #[test]
    fn an_unknown_id_resolves_to_nothing() {
        let grants = PathGrants::default();
        let now = Instant::now();
        must_issue(&grants, GrantKind::Folder, vec![path("project")], now);

        assert_eq!(
            grants.resolve_at(GrantKind::Folder, "00000000000000000000000000000000", now),
            None
        );
    }

    #[test]
    fn a_grant_expires_after_fifteen_minutes_and_not_before() {
        let grants = PathGrants::default();
        let issued = Instant::now();
        let id = must_issue(&grants, GrantKind::Folder, vec![path("project")], issued);

        let just_before = issued + GRANT_LIFETIME - Duration::from_secs(1);
        assert!(grants
            .resolve_at(GrantKind::Folder, &id, just_before)
            .is_some());
        let at_expiry = issued + GRANT_LIFETIME;
        assert_eq!(grants.resolve_at(GrantKind::Folder, &id, at_expiry), None);
        // Gone for good, not merely hidden from one clock reading.
        assert_eq!(grants.resolve_at(GrantKind::Folder, &id, issued), None);
    }

    #[test]
    fn issuing_a_seventeenth_grant_evicts_the_oldest() {
        let grants = PathGrants::default();
        let now = Instant::now();
        let ids: Vec<String> = (0..=MAX_LIVE_GRANTS)
            .map(|index| {
                must_issue(
                    &grants,
                    GrantKind::Folder,
                    vec![path(&index.to_string())],
                    now,
                )
            })
            .collect();

        assert_eq!(grants.resolve_at(GrantKind::Folder, &ids[0], now), None);
        for (index, id) in ids.iter().enumerate().skip(1) {
            assert_eq!(
                grants.resolve_at(GrantKind::Folder, id, now),
                Some(vec![path(&index.to_string())]),
                "grant {index} is within the live bound"
            );
        }
    }

    #[test]
    fn a_burst_of_one_kind_does_not_evict_another_kinds_grant() {
        let grants = PathGrants::default();
        let now = Instant::now();
        let folder = must_issue(&grants, GrantKind::Folder, vec![path("folder")], now);
        for index in 0..=MAX_LIVE_GRANTS * 2 {
            must_issue(
                &grants,
                GrantKind::SkillSources,
                vec![path(&index.to_string())],
                now,
            );
        }

        assert_eq!(
            grants.resolve_at(GrantKind::Folder, &folder, now),
            Some(vec![path("folder")])
        );
    }

    #[test]
    fn a_grant_carries_up_to_its_cap_and_refuses_one_more_path_whole() {
        let grants = PathGrants::default();
        let now = Instant::now();
        let at_cap: Vec<PathBuf> = (0..MAX_PATHS_PER_GRANT)
            .map(|index| path(&index.to_string()))
            .collect();
        let id = grants
            .issue_at(GrantKind::SkillSources, at_cap, now)
            .expect("a grant at the cap is recorded");
        let resolved = grants
            .resolve_at(GrantKind::SkillSources, &id, now)
            .expect("grant is live");
        assert_eq!(resolved.len(), MAX_PATHS_PER_GRANT);

        let over: Vec<PathBuf> = (0..=MAX_PATHS_PER_GRANT)
            .map(|index| path(&index.to_string()))
            .collect();
        assert_eq!(grants.issue_at(GrantKind::SkillSources, over, now), None);
    }

    #[test]
    fn a_grant_resolves_again_after_use_because_use_does_not_consume_it() {
        let grants = PathGrants::default();
        let now = Instant::now();
        let id = must_issue(&grants, GrantKind::Folder, vec![path("project")], now);

        assert!(grants.resolve_at(GrantKind::Folder, &id, now).is_some());
        assert!(grants.resolve_at(GrantKind::Folder, &id, now).is_some());
    }

    #[test]
    fn clear_forgets_every_grant() {
        let grants = PathGrants::default();
        let id = must_issue(
            &grants,
            GrantKind::Folder,
            vec![path("project")],
            Instant::now(),
        );
        let skills = must_issue(
            &grants,
            GrantKind::SkillSources,
            vec![path("skill.md")],
            Instant::now(),
        );
        assert!(grants.resolve(GrantKind::Folder, &id).is_some());

        grants.clear();

        assert_eq!(grants.resolve(GrantKind::Folder, &id), None);
        assert_eq!(grants.resolve(GrantKind::SkillSources, &skills), None);
    }

    #[test]
    fn the_project_sheet_shows_the_same_expired_choice_text_as_the_backend() {
        // The sheet recognises an expired choice by comparing the backend's
        // message with its own copy. This keeps the two from drifting apart.
        let sheet = include_str!("../../src/features/projects/ProjectCreateSheet.tsx");
        assert!(
            sheet.contains(&format!("\"{EXPIRED_CHOICE}\"")),
            "ProjectCreateSheet.tsx must carry EXPIRED_CHOICE verbatim"
        );
    }
}
