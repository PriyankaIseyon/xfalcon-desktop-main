//! `WWW-Authenticate` parsing (RFC 6750 §3, RFC 9728 §5.1).
//!
//! A resource server that refuses a bearer says *why* in this header, and the
//! difference matters to the caller: `invalid_token` means renew or sign in
//! again, `insufficient_scope` means the grant is real but too narrow (and the
//! challenge names the scopes that would fix it), and anything else means the
//! 401 says nothing this client can act on. Collapsing those into "auth failed"
//! is what makes a client loop pointlessly on a grant no renewal can repair.
//!
//! `resource_metadata` is parsed for the same reason RFC 9728 defines it: an
//! authorization server's coordinates can move, and the challenge is how a
//! resource server points a client at its current metadata.

use reqwest::Url;

/// What a resource server's 401 asked for.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct Challenge {
    /// RFC 6750 §3 `error`, lowercased.
    pub(crate) error: Option<String>,
    /// RFC 6750 §3 `scope` — the scopes that would satisfy the request, present
    /// on `insufficient_scope`.
    pub(crate) scope: Option<String>,
    /// RFC 9728 §5.1 `resource_metadata` — where this server's current
    /// Protected Resource Metadata lives.
    pub(crate) resource_metadata: Option<String>,
}

impl Challenge {
    /// Whether a *new* grant is required, as opposed to a renewal of this one.
    ///
    /// `invalid_token` covers expiry, revocation, a rotated signing key, and a
    /// token minted for the wrong audience. A renewal is tried first regardless
    /// — it is one cheap request — but when it also fails, this is what says the
    /// user has to be involved again.
    #[must_use]
    pub(crate) fn needs_new_grant(&self) -> bool {
        matches!(
            self.error.as_deref(),
            Some("invalid_token" | "invalid_grant" | "invalid_request")
        )
    }

    /// Whether the grant is valid and too narrow, and what would widen it.
    #[must_use]
    pub(crate) fn insufficient_scope(&self) -> Option<&str> {
        if self.error.as_deref() == Some("insufficient_scope") {
            // A challenge may omit `scope` even when it names the error, in
            // which case there is nothing to widen *to* and the caller can only
            // report it.
            return self.scope.as_deref().or(Some(""));
        }
        None
    }

    /// The metadata URL to re-read, when the challenge names one that is
    /// actually about `server_origin`.
    ///
    /// Cross-origin pointers are dropped rather than followed: a `401` is the
    /// least authenticated moment in the exchange, and honouring an arbitrary
    /// URL there would let a compromised or hostile resource server aim
    /// discovery — and therefore the next authorization request — at a host of
    /// its choosing.
    #[must_use]
    pub(crate) fn metadata_url_for(&self, server: &Url) -> Option<Url> {
        let candidate = Url::parse(self.resource_metadata.as_deref()?).ok()?;
        if candidate.origin() != server.origin() || !super::endpoint_is_usable(&candidate) {
            return None;
        }
        Some(candidate)
    }
}

/// Parses `WWW-Authenticate: Bearer …`.
///
/// Tolerant of the shapes real servers emit — quoted and bare values, extra
/// whitespace, a missing scheme — because the alternative to tolerance here is
/// discarding the only machine-readable statement of why a call was refused.
/// Unknown parameters are ignored, and a header with none yields an empty
/// challenge rather than an error: "refused, and it did not say why" is a real
/// outcome that the caller has to handle either way.
#[must_use]
pub(crate) fn parse(header: Option<&str>) -> Challenge {
    let Some(header) = header else {
        return Challenge::default();
    };
    let parameters = header
        .trim()
        .strip_prefix("Bearer ")
        .or_else(|| header.trim().strip_prefix("bearer "))
        .unwrap_or(header.trim());

    let mut challenge = Challenge::default();
    for parameter in split_parameters(parameters) {
        let Some((key, value)) = parameter.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').to_owned();
        if value.is_empty() {
            continue;
        }
        match key.trim().to_ascii_lowercase().as_str() {
            "error" => challenge.error = Some(value.to_ascii_lowercase()),
            "scope" => challenge.scope = Some(value),
            "resource_metadata" => challenge.resource_metadata = Some(value),
            // `realm`, `error_description`, `error_uri`, and vendor extensions
            // are prose about someone else's infrastructure. Nothing here
            // renders them, so nothing here reads them.
            _ => {}
        }
    }
    challenge
}

/// Splits on commas that are not inside a quoted value.
///
/// Quote-aware because `error_description` routinely contains commas inside its
/// quotes, and a naive `split(',')` would then emit fragments that parse as
/// parameters of their own — one of which can shadow a real `error` or `scope`.
fn split_parameters(parameters: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    for (index, character) in parameters.char_indices() {
        match character {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                pieces.push(parameters[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    pieces.push(parameters[start..].trim());
    pieces.retain(|piece| !piece.is_empty());
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_header_is_a_refusal_that_said_nothing() {
        let challenge = parse(None);
        assert_eq!(challenge, Challenge::default());
        assert!(!challenge.needs_new_grant());
        assert!(challenge.insufficient_scope().is_none());
    }

    #[test]
    fn an_expired_token_challenge_asks_for_a_new_grant() {
        let challenge = parse(Some(
            r#"Bearer realm="mcp", error="invalid_token", error_description="The access token expired, please retry""#,
        ));
        assert_eq!(challenge.error.as_deref(), Some("invalid_token"));
        assert!(challenge.needs_new_grant());
        // The comma inside the quoted description must not have produced a
        // spurious parameter that overwrote something real.
        assert_eq!(challenge.scope, None);
    }

    #[test]
    fn an_insufficient_scope_challenge_names_what_would_satisfy_it() {
        let challenge = parse(Some(
            r#"Bearer error="insufficient_scope", scope="mcp:tools:read mcp:tools:write""#,
        ));
        assert!(!challenge.needs_new_grant());
        assert_eq!(
            challenge.insufficient_scope(),
            Some("mcp:tools:read mcp:tools:write")
        );
    }

    #[test]
    fn bare_unquoted_values_and_odd_casing_still_parse() {
        let challenge = parse(Some("bearer ERROR=Invalid_Token"));
        assert_eq!(challenge.error.as_deref(), Some("invalid_token"));
        assert!(challenge.needs_new_grant());
    }

    #[test]
    fn a_metadata_pointer_is_followed_only_within_the_servers_own_origin() {
        let server = Url::parse("https://mcp.example.test/mcp").expect("url");
        let same = parse(Some(
            r#"Bearer resource_metadata="https://mcp.example.test/.well-known/oauth-protected-resource""#,
        ));
        assert!(same.metadata_url_for(&server).is_some());

        for hostile in [
            "https://attacker.example/.well-known/oauth-protected-resource",
            "https://mcp.example.test:8443/.well-known/oauth-protected-resource",
            "http://mcp.example.test/.well-known/oauth-protected-resource",
        ] {
            let challenge = parse(Some(&format!(r#"Bearer resource_metadata="{hostile}""#)));
            assert!(
                challenge.metadata_url_for(&server).is_none(),
                "{hostile} must not redirect discovery"
            );
        }
    }
}
