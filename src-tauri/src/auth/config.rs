use std::env;
use std::path::Path;
use std::sync::OnceLock;

use reqwest::Url;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthConfig {
    pub da_base_url: Url,
    pub casdoor_issuer: Url,
    pub client_id: String,
    pub organization: String,
    pub loopback_port: u16,
    /// The OAuth client an MCP sign-in falls back to when the authorization
    /// server will not register one dynamically.
    ///
    /// Optional, and only reached on the per-device path: a deployment that
    /// configures the shared grant key takes its client from the DA row instead
    /// and never consults this. Without either, the surface asks the operator to
    /// paste a client id — which is the whole thing this spares them.
    ///
    /// Its redirect URI is the MCP loopback (`loopback_port + 1`), NOT the login
    /// one, so a client registered only for sign-in will be refused here.
    /// The pair is used only when MCP discovery identifies `casdoor_issuer`;
    /// an unrelated authorization server must obtain its own client.
    pub mcp_client_id: Option<String>,
    /// The secret for [`Self::mcp_client_id`], for an authorization server that
    /// will only issue a confidential client.
    ///
    /// Meaningless without that id and never read apart from it — see
    /// `crate::mcp::commands::device_path_client`, which resolves the two as one
    /// value so a pasted id can never be sent with a packaged secret.
    ///
    /// A bundled secret is extractable from any copy of the build; it is
    /// therefore a tenant-wide shared credential, not a per-device one. It is
    /// absent unless an operator supplies it, and a public client (an id with
    /// no secret) is preferred wherever the authorization server will issue
    /// one — PKCE, not a secret, is what protects a native app's code exchange.
    /// The shared grant key is still never bundled
    /// (see `.github/workflows/desktop-build.yml`).
    pub mcp_client_secret: Option<String>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthConfigError {
    #[error(
        "auth is not configured: set XFALCON_DA_BASE_URL, XFALCON_CASDOOR_ISSUER, XFALCON_CASDOOR_CLIENT_ID, and XFALCON_CASDOOR_ORGANIZATION (copy .env.example to .env)"
    )]
    NotConfigured,
    #[error("invalid auth configuration")]
    Invalid,
}

/// Tenant coordinates rendered into `resources/tenant-config.json` by the CI
/// build's `workflow_dispatch` inputs (see `.github/workflows/desktop-build.yml`).
/// Field names match those inputs — not the `XFALCON_*` env var names — since
/// this file is produced by the workflow, never typed by hand.
#[derive(Debug, Deserialize)]
struct BundledTenantConfig {
    da_base_url: String,
    casdoor_issuer: String,
    casdoor_client_id: String,
    casdoor_organization: String,
    /// Optional: absent in every build cut before this field existed, and in any
    /// tenant that has no MCP client to offer.
    #[serde(default)]
    mcp_client_id: Option<String>,
    /// Optional, and only meaningful beside `mcp_client_id`.
    #[serde(default)]
    mcp_client_secret: Option<String>,
}

/// Set once, during app setup, from the resource directory Tauri resolved for
/// this install. `None` means no bundled file was found (every local dev build
/// — resources only exist in a `tauri build` bundle), or the one found was
/// rejected; both are handled the same way by [`AuthConfig::from_env`].
static BUNDLED_TENANT_CONFIG: OnceLock<Option<BundledTenantConfig>> = OnceLock::new();

/// The SHA-256 of the `resources/tenant-config.json` this binary was compiled
/// beside, from `build.rs`. A bundled file with any other digest is not this
/// build's tenant.
const COMPILED_TENANT_CONFIG_SHA256: &str = env!("XFALCON_TENANT_CONFIG_SHA256");

/// Whether a missing or rejected bundled config falls through to the process
/// environment.
///
/// Debug builds only, like `load_workspace_dotenv`. A release build that
/// honoured `XFALCON_*` would let anyone who can set an environment variable,
/// or delete one file, point it at an issuer and a DA they run themselves and
/// sign in as whoever they like — so a release build trusts no tenant but the
/// one it was compiled with.
const PROCESS_ENV_FALLBACK: bool = cfg!(debug_assertions);

/// What a build without a usable bundled config does instead, for the log.
const FALLBACK: &str = if PROCESS_ENV_FALLBACK {
    "falling back to the process environment"
} else {
    "sign-in is unavailable: a release build reads no other source"
};

#[derive(Debug, Error)]
enum BundledConfigRejection {
    #[error("it is not the tenant config this build was compiled with")]
    NotThisBuild,
    /// The tracked `{}` a checkout ships. The release workflow writes the real
    /// tenant over it, so a local build finding it is the expected shape, and
    /// only a release build finding it is broken.
    #[error("it is the unconfigured placeholder a source checkout ships")]
    Placeholder,
    #[error("it could not be parsed: {0}")]
    Unparseable(#[from] serde_json::Error),
}

/// Accepts `contents` only as the exact file whose digest is `compiled_sha256`.
///
/// The digest is checked first, so an edited file is refused as edited rather
/// than parsed. The file is not secret, so the comparison need not be
/// constant-time.
fn parse_bundled(
    contents: &[u8],
    compiled_sha256: &str,
) -> Result<BundledTenantConfig, BundledConfigRejection> {
    if format!("{:x}", Sha256::digest(contents)) != compiled_sha256 {
        return Err(BundledConfigRejection::NotThisBuild);
    }
    if serde_json::from_slice::<serde_json::Map<String, serde_json::Value>>(contents)
        .is_ok_and(|object| object.is_empty())
    {
        return Err(BundledConfigRejection::Placeholder);
    }
    Ok(serde_json::from_slice(contents)?)
}

/// Reads the bundled `resources/tenant-config.json` resolved for this install,
/// if any, and makes it available to every later `AuthConfig::from_env()` call.
/// Safe to call with `None` or a path that does not exist — that's the expected
/// shape for local development, not an error.
///
/// Call exactly once, before any `AuthConfig::from_env()` call — `run()` does
/// this during `.setup()` using `PathResolver::resolve(..., BaseDirectory::Resource)`.
/// A second call is a no-op (`OnceLock::set` fails silently), which only matters
/// to a future caller, not today's single-call site.
///
/// Logs which of the ways this can find no usable config actually happened —
/// path resolution, a missing/unreadable file, a file that is not this build's,
/// or a parse failure each point at a different cause, and a distributed build
/// has no other way to tell them apart after the fact. This is the one thing a
/// report of "settings didn't come back after updating" needs first: whether the
/// bundled tenant config was ever found at all.
pub fn init_bundled_tenant_config(config_path: Option<&Path>) {
    let bundled = match config_path {
        None => {
            tracing::info!(
                "no bundled tenant config resource path resolved for this install; {FALLBACK}"
            );
            None
        }
        Some(path) => match std::fs::read(path) {
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "bundled tenant config resource path resolved but the file could not \
                     be read; {FALLBACK}"
                );
                None
            }
            Ok(contents) => match parse_bundled(&contents, COMPILED_TENANT_CONFIG_SHA256) {
                Err(BundledConfigRejection::Placeholder) if PROCESS_ENV_FALLBACK => {
                    tracing::info!(
                        path = %path.display(),
                        "bundled tenant config is the source checkout's placeholder; {FALLBACK}"
                    );
                    None
                }
                Err(error) => {
                    tracing::warn!(
                        path = %path.display(),
                        error = %error,
                        "bundled tenant config file was rejected; {FALLBACK}"
                    );
                    None
                }
                Ok(config) => {
                    tracing::info!(
                        organization = %config.casdoor_organization,
                        path = %path.display(),
                        "bundled tenant config resolved"
                    );
                    Some(config)
                }
            },
        },
    };
    let _ = BUNDLED_TENANT_CONFIG.set(bundled);
}

/// Where [`AuthConfig::from_env`] would take its values from right now.
///
/// For the diagnostics bundle. A distributed build that silently fell through
/// to an empty process environment, and one reading its bundled tenant, produce
/// the same symptom — Settings with nothing in it — and nothing else in the app
/// tells them apart.
#[must_use]
pub fn tenant_config_source() -> &'static str {
    match BUNDLED_TENANT_CONFIG.get() {
        Some(Some(_)) => "bundled tenant config",
        None if PROCESS_ENV_FALLBACK => "process environment (bundled config never resolved)",
        None => "none (bundled config never resolved)",
        Some(None) if PROCESS_ENV_FALLBACK => {
            "process environment (no bundled config, or it was rejected)"
        }
        Some(None) => "none (no bundled config, or it was rejected)",
    }
}

impl AuthConfig {
    /// Resolve auth coordinates, preferring a bundled tenant config over the
    /// process environment.
    ///
    /// Resolution order:
    /// 1. `resources/tenant-config.json`, bundled at CI build time
    ///    (`init_bundled_tenant_config`) — the path a distributed build uses,
    ///    and the only one a release build has.
    /// 2. Debug builds only: the process environment (`.env`, loaded by
    ///    `load_workspace_dotenv`) — the path local development uses:
    ///    - `XFALCON_DA_BASE_URL` (https origin, no path)
    ///    - `XFALCON_CASDOOR_ISSUER` (https issuer URL)
    ///    - `XFALCON_CASDOOR_CLIENT_ID`
    ///    - `XFALCON_CASDOOR_ORGANIZATION` (Casdoor org / tenant slug)
    ///    - `XFALCON_MCP_CLIENT_ID` (optional; the MCP sign-in fallback client)
    ///    - `XFALCON_MCP_CLIENT_SECRET` (optional; only read beside that id)
    ///
    /// `XFALCON_OAUTH_LOOPBACK_PORT` (optional, default 19847 — must match the
    /// Casdoor redirect URI) is always read from the environment: it names a
    /// local port choice, not a tenant identity, so bundling it would give
    /// every install of one tenant's build the same port with no reason to.
    pub fn from_env() -> Result<Self, AuthConfigError> {
        Self::resolve(
            BUNDLED_TENANT_CONFIG.get().and_then(|slot| slot.as_ref()),
            PROCESS_ENV_FALLBACK,
        )
    }

    fn resolve(
        bundled: Option<&BundledTenantConfig>,
        process_env_fallback: bool,
    ) -> Result<Self, AuthConfigError> {
        if let Some(bundled) = bundled {
            return Self::from_bundled(bundled);
        }
        if !process_env_fallback {
            return Err(AuthConfigError::NotConfigured);
        }

        let da_base =
            env::var("XFALCON_DA_BASE_URL").map_err(|_| AuthConfigError::NotConfigured)?;
        let issuer =
            env::var("XFALCON_CASDOOR_ISSUER").map_err(|_| AuthConfigError::NotConfigured)?;
        let client_id =
            env::var("XFALCON_CASDOOR_CLIENT_ID").map_err(|_| AuthConfigError::NotConfigured)?;
        let organization =
            env::var("XFALCON_CASDOOR_ORGANIZATION").map_err(|_| AuthConfigError::NotConfigured)?;

        let da_base_url = parse_https_origin(&da_base).ok_or(AuthConfigError::Invalid)?;
        let casdoor_issuer = parse_https_origin(&issuer).ok_or(AuthConfigError::Invalid)?;
        if client_id.trim().is_empty() || organization.trim().is_empty() {
            return Err(AuthConfigError::Invalid);
        }

        Ok(Self {
            da_base_url,
            casdoor_issuer,
            client_id: client_id.trim().to_owned(),
            organization: organization.trim().to_owned(),
            loopback_port: parse_loopback_port(),
            mcp_client_id: non_empty(env::var("XFALCON_MCP_CLIENT_ID").ok()),
            mcp_client_secret: non_empty(env::var("XFALCON_MCP_CLIENT_SECRET").ok()),
        })
    }

    fn from_bundled(bundled: &BundledTenantConfig) -> Result<Self, AuthConfigError> {
        let da_base_url =
            parse_https_origin(&bundled.da_base_url).ok_or(AuthConfigError::Invalid)?;
        let casdoor_issuer =
            parse_https_origin(&bundled.casdoor_issuer).ok_or(AuthConfigError::Invalid)?;
        if bundled.casdoor_client_id.trim().is_empty()
            || bundled.casdoor_organization.trim().is_empty()
        {
            return Err(AuthConfigError::Invalid);
        }

        Ok(Self {
            da_base_url,
            casdoor_issuer,
            client_id: bundled.casdoor_client_id.trim().to_owned(),
            organization: bundled.casdoor_organization.trim().to_owned(),
            loopback_port: parse_loopback_port(),
            mcp_client_id: non_empty(bundled.mcp_client_id.clone()),
            mcp_client_secret: non_empty(bundled.mcp_client_secret.clone()),
        })
    }
}

/// A blank string is the same as an absent one: the CI step renders every field
/// it is given, and an input left empty must not become a client id of "".
fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

pub(crate) fn parse_loopback_port() -> u16 {
    const DEFAULT: u16 = 19847;
    std::env::var("XFALCON_OAUTH_LOOPBACK_PORT")
        .ok()
        .and_then(|value| value.trim().parse::<u16>().ok())
        .filter(|port| *port > 0)
        .unwrap_or(DEFAULT)
}

fn parse_https_origin(value: &str) -> Option<Url> {
    let url = Url::parse(value.trim()).ok()?;
    if url.scheme() != "https" || url.host_str().is_none() {
        return None;
    }
    if !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    let path_ok = url.path().is_empty() || url.path() == "/";
    if !path_ok || url.query().is_some() || url.fragment().is_some() {
        return None;
    }
    Some(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_https_origins() {
        assert!(parse_https_origin("http://example.com").is_none());
        assert!(parse_https_origin("https://example.com/path").is_none());
        assert!(parse_https_origin("https://example.com").is_some());
    }

    // These exercise `AuthConfig::resolve` directly with an explicit `bundled`
    // argument rather than through `from_env`/`BUNDLED_TENANT_CONFIG` or real
    // `XFALCON_*` env vars — both are process-global state, and mutating
    // either from a test would race every other test running in the same
    // process. `resolve` is the pure seam that avoids that.

    fn valid_bundled() -> BundledTenantConfig {
        BundledTenantConfig {
            da_base_url: "https://da.example.com".to_owned(),
            casdoor_issuer: "https://auth.example.com".to_owned(),
            casdoor_client_id: "client-123".to_owned(),
            casdoor_organization: "acme".to_owned(),
            mcp_client_id: None,
            mcp_client_secret: None,
        }
    }

    /// A build cut before the field existed, and a tenant with no MCP client to
    /// offer, are the same shape on disk: the key is simply absent.
    #[test]
    fn a_tenant_config_without_an_mcp_client_still_parses() {
        let without = r#"{
            "da_base_url": "https://da.example.com",
            "casdoor_issuer": "https://auth.example.com",
            "casdoor_client_id": "client-123",
            "casdoor_organization": "acme"
        }"#;
        let parsed: BundledTenantConfig = serde_json::from_str(without).expect("parses");
        assert_eq!(parsed.mcp_client_id, None);
        assert_eq!(parsed.mcp_client_secret, None);
        let config = AuthConfig::resolve(Some(&parsed), false).expect("resolves");
        assert_eq!(config.mcp_client_id, None);
        assert_eq!(config.mcp_client_secret, None);
    }

    #[test]
    fn a_packaged_mcp_client_reaches_the_config() {
        let mut bundled = valid_bundled();
        bundled.mcp_client_id = Some("  mcp-client-9  ".to_owned());
        let config = AuthConfig::resolve(Some(&bundled), false).expect("resolves");
        assert_eq!(config.mcp_client_id.as_deref(), Some("mcp-client-9"));
    }

    /// A confidential client needs both halves to arrive, and both are trimmed
    /// the same way: the CI step renders whatever it is handed.
    #[test]
    fn a_packaged_confidential_mcp_client_carries_its_secret() {
        let bundled: BundledTenantConfig = serde_json::from_str(
            r#"{
                "da_base_url": "https://da.example.com",
                "casdoor_issuer": "https://auth.example.com",
                "casdoor_client_id": "client-123",
                "casdoor_organization": "acme",
                "mcp_client_id": "  mcp-client-9  ",
                "mcp_client_secret": "  mcp-secret-9  "
            }"#,
        )
        .expect("parses");
        let config = AuthConfig::resolve(Some(&bundled), false).expect("resolves");
        assert_eq!(config.mcp_client_id.as_deref(), Some("mcp-client-9"));
        assert_eq!(config.mcp_client_secret.as_deref(), Some("mcp-secret-9"));
    }

    /// A dispatch that left the secret blank must not package an empty
    /// credential: the authorization server would be sent `client_secret=""`
    /// and refuse a client that is otherwise correct.
    #[test]
    fn a_blank_packaged_secret_is_absent_rather_than_empty() {
        let mut bundled = valid_bundled();
        bundled.mcp_client_id = Some("mcp-client-9".to_owned());
        bundled.mcp_client_secret = Some("   ".to_owned());
        let config = AuthConfig::resolve(Some(&bundled), false).expect("resolves");
        assert_eq!(config.mcp_client_id.as_deref(), Some("mcp-client-9"));
        assert_eq!(config.mcp_client_secret, None);
    }

    /// The CI step renders whatever it is handed, and a `workflow_dispatch`
    /// input left blank arrives as an empty string. A client id of "" would be
    /// sent to the authorization server as a real one.
    #[test]
    fn a_blank_packaged_mcp_client_is_the_same_as_none() {
        let mut bundled = valid_bundled();
        bundled.mcp_client_id = Some("   ".to_owned());
        let config = AuthConfig::resolve(Some(&bundled), false).expect("resolves");
        assert_eq!(config.mcp_client_id, None);
    }

    #[test]
    fn resolve_prefers_a_valid_bundled_config() {
        let bundled = valid_bundled();
        let config =
            AuthConfig::resolve(Some(&bundled), false).expect("valid bundled config resolves");
        assert_eq!(config.da_base_url.as_str(), "https://da.example.com/");
        assert_eq!(config.casdoor_issuer.as_str(), "https://auth.example.com/");
        assert_eq!(config.client_id, "client-123");
        assert_eq!(config.organization, "acme");
    }

    #[test]
    fn resolve_rejects_a_bundled_config_with_a_non_https_url() {
        let mut bundled = valid_bundled();
        bundled.da_base_url = "http://da.example.com".to_owned();
        assert_eq!(
            AuthConfig::resolve(Some(&bundled), false),
            Err(AuthConfigError::Invalid)
        );
    }

    #[test]
    fn resolve_rejects_a_bundled_config_with_a_blank_client_id() {
        let mut bundled = valid_bundled();
        bundled.casdoor_client_id = "   ".to_owned();
        assert_eq!(
            AuthConfig::resolve(Some(&bundled), false),
            Err(AuthConfigError::Invalid)
        );
    }

    #[test]
    fn resolve_rejects_a_bundled_config_with_a_blank_organization() {
        let mut bundled = valid_bundled();
        bundled.casdoor_organization = "".to_owned();
        assert_eq!(
            AuthConfig::resolve(Some(&bundled), false),
            Err(AuthConfigError::Invalid)
        );
    }

    #[test]
    fn init_bundled_tenant_config_tolerates_a_missing_config_path() {
        // No panic, no filesystem access assumed to succeed -- this is the
        // shape every local dev build hits (resources only exist inside a
        // `tauri build` bundle).
        init_bundled_tenant_config(None);
    }

    #[test]
    fn init_bundled_tenant_config_tolerates_a_path_that_does_not_exist() {
        // Different from `None` above: the resource path DID resolve, but
        // nothing is there to read -- the shape a build whose bundler didn't
        // actually ship the resource would hit.
        init_bundled_tenant_config(Some(Path::new("/does/not/exist/tenant-config.json")));
    }

    #[test]
    fn init_bundled_tenant_config_tolerates_malformed_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("tenant-config.json");
        std::fs::write(&path, "not json").expect("write");
        init_bundled_tenant_config(Some(&path));
    }

    /// A release build's only tenant is its bundled one. Holding every
    /// `XFALCON_*` variable does not change that: `false` is what `from_env`
    /// passes in a release build, and the environment is never read.
    #[test]
    fn a_release_build_without_a_bundled_config_is_not_configured() {
        assert_eq!(
            AuthConfig::resolve(None, false),
            Err(AuthConfigError::NotConfigured)
        );
    }

    const BUNDLED_JSON: &str = r#"{
        "da_base_url": "https://da.example.com",
        "casdoor_issuer": "https://auth.example.com",
        "casdoor_client_id": "client-123",
        "casdoor_organization": "acme"
    }"#;

    fn sha256_hex(contents: &[u8]) -> String {
        format!("{:x}", Sha256::digest(contents))
    }

    #[test]
    fn the_file_this_build_was_compiled_with_is_accepted() {
        let parsed = parse_bundled(
            BUNDLED_JSON.as_bytes(),
            &sha256_hex(BUNDLED_JSON.as_bytes()),
        )
        .expect("the compiled file is this build's tenant");
        assert_eq!(parsed.casdoor_issuer, "https://auth.example.com");
    }

    /// The attack this exists for: an installed copy's file rewritten to name
    /// an issuer the editor runs. It still parses, and it is still refused.
    #[test]
    fn an_edited_bundled_file_is_refused_even_though_it_parses() {
        let compiled = sha256_hex(BUNDLED_JSON.as_bytes());
        let edited =
            BUNDLED_JSON.replace("https://auth.example.com", "https://auth.attacker.example");

        assert!(matches!(
            parse_bundled(edited.as_bytes(), &compiled),
            Err(BundledConfigRejection::NotThisBuild)
        ));
    }

    /// The file a checkout tracks. It matches its own digest, and it names no
    /// tenant, so it is told apart from a file that failed to parse.
    #[test]
    fn the_checkout_placeholder_is_recognised_rather_than_misparsed() {
        let contents = b"{}\n";

        assert!(matches!(
            parse_bundled(contents, &sha256_hex(contents)),
            Err(BundledConfigRejection::Placeholder)
        ));
    }

    #[test]
    fn a_partial_config_is_still_unparseable_not_a_placeholder() {
        let contents = br#"{"casdoor_issuer": "https://auth.example.com"}"#;

        assert!(matches!(
            parse_bundled(contents, &sha256_hex(contents)),
            Err(BundledConfigRejection::Unparseable(_))
        ));
    }

    #[test]
    fn a_digest_match_does_not_excuse_an_unparseable_file() {
        let contents = b"not json";

        assert!(matches!(
            parse_bundled(contents, &sha256_hex(contents)),
            Err(BundledConfigRejection::Unparseable(_))
        ));
    }

    /// Ties the digest `build.rs` compiles in to the file it names, so a build
    /// script that hashed anything else would fail here rather than refuse
    /// every real install.
    #[test]
    fn the_compiled_digest_is_the_digest_of_the_checked_in_file() {
        assert_eq!(
            COMPILED_TENANT_CONFIG_SHA256,
            sha256_hex(include_bytes!("../../resources/tenant-config.json"))
        );
    }
}
