//! Reads whether the user's `claude` program is signed in.
//!
//! `claude auth status --json` also prints the signed-in e-mail, organisation and
//! local directories. Those identify the user, so the output is read into a
//! struct with no field for them, and they never leave this module.

use std::path::Path;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;

use super::process::{command, run_capped, ProcessError};

const STATUS_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_STATUS_BYTES: usize = 64 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeAccount {
    pub(crate) logged_in: bool,
    // e.g. "claude.ai", "console", "none"
    pub(crate) auth_method: Option<String>,
    // e.g. "firstParty", "bedrock", "vertex"
    pub(crate) api_provider: Option<String>,
    // e.g. "pro", "max", "team", "enterprise"
    pub(crate) plan: Option<String>,
}

impl ClaudeAccount {
    pub(crate) fn is_ready(&self) -> bool {
        self.logged_in
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum AccountError {
    #[error("claude could not be started")]
    StartFailed,
    #[error("claude did not answer in time")]
    TimedOut,
    #[error("claude printed an unreadable status")]
    Unparseable,
}

/// Only the four fields that may leave this module. Every other key is skipped.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthStatus {
    #[serde(default)]
    logged_in: Option<bool>,
    // Untyped so that a field of the wrong type is dropped, not fatal.
    #[serde(default)]
    auth_method: Value,
    #[serde(default)]
    api_provider: Value,
    #[serde(default)]
    subscription_type: Value,
}

pub(crate) fn parse_auth_status(stdout: &[u8]) -> Result<ClaudeAccount, AccountError> {
    // The serde error is dropped on purpose: its message quotes the offending
    // value, which may be an identity field.
    let status: AuthStatus =
        serde_json::from_slice(stdout).map_err(|_| AccountError::Unparseable)?;
    let logged_in = status.logged_in.ok_or(AccountError::Unparseable)?;
    Ok(ClaudeAccount {
        logged_in,
        auth_method: identifier(&status.auth_method),
        api_provider: identifier(&status.api_provider),
        plan: identifier(&status.subscription_type).map(|plan| plan.to_ascii_lowercase()),
    })
}

pub(crate) async fn read_account(binary: &Path) -> Result<ClaudeAccount, AccountError> {
    let mut invocation = command(binary);
    invocation.args(["auth", "status", "--json"]);
    // A signed-out program exits 1 but still prints this status, so the exit
    // status carries nothing the parse does not already show.
    let captured = run_capped(invocation, None, STATUS_TIMEOUT, MAX_STATUS_BYTES)
        .await
        .map_err(|error| match error {
            ProcessError::StartFailed => AccountError::StartFailed,
            ProcessError::TimedOut => AccountError::TimedOut,
            ProcessError::OutputTooLarge | ProcessError::Io => AccountError::Unparseable,
        })?;
    parse_auth_status(&captured.stdout)
}

/// Status fields are labels, not free text. Anything that is not a short
/// identifier is dropped so that no unexpected value travels further.
fn identifier(value: &Value) -> Option<String> {
    let text = value.as_str()?;
    let well_formed = (1..=MAX_IDENTIFIER_BYTES).contains(&text.len())
        && text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
    well_formed.then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIGNED_OUT: &str = r#"{"loggedIn": false, "authMethod": "none", "apiProvider": "firstParty", "analyticsDisabled": false, "projectsDirectory": "/home/example/.claude/projects", "configDirectory": "/home/example/.claude"}"#;

    const SIGNED_IN_TEAM: &str = r#"{"loggedIn": true, "authMethod": "claude.ai", "apiProvider": "firstParty", "email": "person@example.com", "orgId": "00000000-0000-4000-8000-000000000000", "orgName": "Example Org", "subscriptionType": "team", "analyticsDisabled": false, "projectsDirectory": "/home/example/.claude/projects", "configDirectory": "/home/example/.claude"}"#;

    #[test]
    fn signed_out_status_is_not_ready() {
        let account = parse_auth_status(SIGNED_OUT.as_bytes()).expect("parse");
        assert_eq!(
            account,
            ClaudeAccount {
                logged_in: false,
                auth_method: Some("none".to_owned()),
                api_provider: Some("firstParty".to_owned()),
                plan: None,
            }
        );
        assert!(!account.is_ready());
    }

    #[test]
    fn signed_in_team_status_is_ready() {
        let account = parse_auth_status(SIGNED_IN_TEAM.as_bytes()).expect("parse");
        assert_eq!(
            account,
            ClaudeAccount {
                logged_in: true,
                auth_method: Some("claude.ai".to_owned()),
                api_provider: Some("firstParty".to_owned()),
                plan: Some("team".to_owned()),
            }
        );
        assert!(account.is_ready());
    }

    #[test]
    fn missing_logged_in_is_unparseable() {
        assert_eq!(
            parse_auth_status(br#"{"authMethod": "claude.ai", "subscriptionType": "team"}"#),
            Err(AccountError::Unparseable)
        );
    }

    #[test]
    fn logged_in_that_is_not_a_boolean_is_unparseable() {
        assert_eq!(
            parse_auth_status(br#"{"loggedIn": "true"}"#),
            Err(AccountError::Unparseable)
        );
    }

    #[test]
    fn empty_and_non_json_input_is_unparseable() {
        let inputs: [&[u8]; 5] = [b"", b"   ", b"not json", b"[]", b"{"];
        for input in inputs {
            assert_eq!(parse_auth_status(input), Err(AccountError::Unparseable));
        }
    }

    #[test]
    fn odd_string_values_become_none() {
        let account = parse_auth_status(
            br#"{"loggedIn": true, "authMethod": "claude ai", "apiProvider": "bedrock/../x", "subscriptionType": ""}"#,
        )
        .expect("parse");
        assert_eq!(account.auth_method, None);
        assert_eq!(account.api_provider, None);
        assert_eq!(account.plan, None);

        let account = parse_auth_status(
            br#"{"loggedIn": true, "authMethod": 7, "apiProvider": ["console"], "subscriptionType": {"name": "max"}}"#,
        )
        .expect("parse");
        assert_eq!(account.auth_method, None);
        assert_eq!(account.api_provider, None);
        assert_eq!(account.plan, None);
    }

    #[test]
    fn identifier_over_the_length_limit_becomes_none() {
        let too_long = "a".repeat(MAX_IDENTIFIER_BYTES + 1);
        let input = format!(r#"{{"loggedIn": true, "apiProvider": "{too_long}"}}"#);
        let account = parse_auth_status(input.as_bytes()).expect("parse");
        assert_eq!(account.api_provider, None);
    }

    #[test]
    fn identifier_at_the_length_limit_is_kept() {
        let longest = "a".repeat(MAX_IDENTIFIER_BYTES);
        let input = format!(r#"{{"loggedIn": true, "apiProvider": "{longest}"}}"#);
        let account = parse_auth_status(input.as_bytes()).expect("parse");
        assert_eq!(account.api_provider, Some(longest));
    }

    #[test]
    fn plan_is_lowercased() {
        let account = parse_auth_status(br#"{"loggedIn": true, "subscriptionType": "Enterprise"}"#)
            .expect("parse");
        assert_eq!(account.plan.as_deref(), Some("enterprise"));
    }

    #[test]
    fn identity_fields_never_reach_the_account() {
        let account = parse_auth_status(SIGNED_IN_TEAM.as_bytes()).expect("parse");
        let debug = format!("{account:?}");
        for identity in [
            "person@example.com",
            "Example Org",
            "00000000-0000-4000-8000-000000000000",
            "/home/example/.claude",
        ] {
            assert!(
                !debug.contains(identity),
                "an identity field leaked into the account's Debug output"
            );
        }
        // Naming every field without `..` means a field added later stops this
        // test from compiling until it is reviewed against the privacy rule.
        let ClaudeAccount {
            logged_in: _,
            auth_method: _,
            api_provider: _,
            plan: _,
        } = account;
    }

    #[tokio::test]
    async fn missing_binary_is_start_failed() {
        let error = read_account(Path::new("/nonexistent/xfalcon-claude"))
            .await
            .expect_err("a missing binary must not start");
        assert_eq!(error, AccountError::StartFailed);
    }
}
