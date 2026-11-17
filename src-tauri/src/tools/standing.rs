//! Standing tool approvals — and the product's auto-approve default.
//!
//! A mutating tool call used to park its run until a human approved that
//! exact invocation (`crate::tools::policy`). Product decision (2026-08-05):
//! the desktop app runs tool calls WITHOUT a per-call human gate — the
//! prompt interrupted every analysis turn (the tenant MCP marks even reads
//! as mutating) and the user asked for it gone entirely. `ToolLoop` still
//! journals and audits a normal approval for every call; only the *click*
//! is gone.
//!
//! `XFALCON_REQUIRE_TOOL_APPROVALS=1` restores ask-every-time (operator
//! escape hatch, read once at store construction). When asking is restored,
//! the per-account session/always grant sets below take effect again.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::RwLock;

pub(crate) const STANDING_APPROVALS_FILE: &str = "standing-tool-approvals.json";
const REQUIRE_APPROVALS_ENV: &str = "XFALCON_REQUIRE_TOOL_APPROVALS";

#[derive(Debug)]
pub struct StandingToolApprovals {
    /// Product default: approve every tool call without asking.
    auto_approve_all: bool,
    /// account -> tool ids allowed for this process lifetime only.
    session: RwLock<HashMap<String, HashSet<String>>>,
    /// account -> tool ids allowed durably (mirrored to disk).
    always: RwLock<HashMap<String, HashSet<String>>>,
    path: PathBuf,
}

impl StandingToolApprovals {
    #[must_use]
    pub fn load(app_local_data: PathBuf) -> Self {
        let require_approvals = std::env::var(REQUIRE_APPROVALS_ENV)
            .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        Self::load_with_mode(app_local_data, !require_approvals)
    }

    #[must_use]
    pub fn load_with_mode(app_local_data: PathBuf, auto_approve_all: bool) -> Self {
        let path = app_local_data.join(STANDING_APPROVALS_FILE);
        let always: HashMap<String, HashSet<String>> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self {
            auto_approve_all,
            session: RwLock::new(HashMap::new()),
            always: RwLock::new(always),
            path,
        }
    }

    #[must_use]
    pub fn allows(&self, account: &str, tool_id: &str) -> bool {
        if self.auto_approve_all {
            return true;
        }
        let in_session = self
            .session
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(account)
            .is_some_and(|tools| tools.contains(tool_id));
        if in_session {
            return true;
        }
        self.always
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(account)
            .is_some_and(|tools| tools.contains(tool_id))
    }

    pub fn allow_for_session(&self, account: &str, tool_id: &str) {
        self.session
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(account.to_owned())
            .or_default()
            .insert(tool_id.to_owned());
    }

    /// Grants durably and mirrors to disk. A failed write degrades to a
    /// session grant (the in-memory set is already updated) rather than
    /// failing the approval the user just gave.
    pub fn allow_always(&self, account: &str, tool_id: &str) {
        {
            let mut always = self
                .always
                .write()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            always
                .entry(account.to_owned())
                .or_default()
                .insert(tool_id.to_owned());
        }
        self.persist();
    }

    fn persist(&self) {
        let snapshot = {
            let always = self
                .always
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            serde_json::to_vec_pretty(&*always)
        };
        let Ok(bytes) = snapshot else {
            return;
        };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::write(&self.path, bytes) {
            tracing::warn!(%error, "standing approvals: persist failed; grant degrades to session scope");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asking_store() -> (tempfile::TempDir, StandingToolApprovals) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = StandingToolApprovals::load_with_mode(dir.path().to_path_buf(), false);
        (dir, store)
    }

    #[test]
    fn auto_approve_mode_allows_everything_without_grants() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = StandingToolApprovals::load_with_mode(dir.path().to_path_buf(), true);
        assert!(store.allows("any-account", "any.tool"));
    }

    #[test]
    fn session_grants_do_not_survive_reload() {
        let (dir, store) = asking_store();
        store.allow_for_session("acct", "mcp.tenant.ida_query");
        assert!(store.allows("acct", "mcp.tenant.ida_query"));

        let reloaded = StandingToolApprovals::load_with_mode(dir.path().to_path_buf(), false);
        assert!(!reloaded.allows("acct", "mcp.tenant.ida_query"));
    }

    #[test]
    fn always_grants_survive_reload_and_stay_per_account() {
        let (dir, store) = asking_store();
        store.allow_always("acct", "mcp.tenant.ida_query");

        let reloaded = StandingToolApprovals::load_with_mode(dir.path().to_path_buf(), false);
        assert!(reloaded.allows("acct", "mcp.tenant.ida_query"));
        assert!(!reloaded.allows("other-acct", "mcp.tenant.ida_query"));
        assert!(!reloaded.allows("acct", "mcp.tenant.other_tool"));
    }

    #[test]
    fn missing_or_corrupt_file_loads_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(STANDING_APPROVALS_FILE), b"not json").expect("write");
        let store = StandingToolApprovals::load_with_mode(dir.path().to_path_buf(), false);
        assert!(!store.allows("acct", "anything"));
    }
}
