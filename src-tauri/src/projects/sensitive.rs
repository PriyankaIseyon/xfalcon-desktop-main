//! Sensitive-file classification and the explicit read-override grant.
//!
//! Two independent responsibilities live here:
//!
//! 1. [`SecretClassifier`]/[`classify`](SecretClassifier::classify) -- a
//!    pure, deterministic, default-deny classifier deciding whether a
//!    project-relative path names one of the secret classes
//!    `docs/decisions.md` (Task 5,
//!    lines 378-381) requires: `.env*`, PEM/private keys, cloud/Kubernetes
//!    credentials, password/vault exports, browser profiles, SSH/GPG
//!    material, and token/config stores. These built-in classes defend
//!    against CWE-798 (use of hard-coded credentials) and CWE-312
//!    (cleartext storage of sensitive information) -- see
//!    `docs/threat-model.md` TM-006's "sensitive-file deny rules".
//!    Callers may extend the built-in set with their own "configured secret
//!    class" glob patterns (plan line 380): [`SecretClassifier::new`] takes
//!    them as a constructor parameter rather than reading new persisted
//!    per-project configuration -- nothing in this task's migration scope
//!    (`persistence/migrations/0004_project_index.sql`) defines storage for
//!    extra patterns, and the plan does not ask for one, so inventing a
//!    table here would be undisclosed scope expansion. A future caller that
//!    *does* want per-project configured classes can source the strings
//!    from wherever it likes (a config file, a future settings table) and
//!    pass them in unchanged.
//! 2. [`SensitiveOverrideGrant`]/[`SensitiveOverrideRequest`] -- the
//!    high-salience, explicit-user-only override the plan requires before a
//!    sensitive file's content may enter model context (plan lines
//!    411-414). This mirrors `crate::tools::policy::ApprovalRequest`/
//!    `ApprovalToken` exactly: a grant is only ever constructed by
//!    [`SensitiveOverrideRequest::grant`] (standing in for the real, future
//!    override-UI action, exactly as `ApprovalRequest::approve` stands in
//!    for the future approval UI -- see that type's doc comment for how
//!    this disclosure is worded), scoped to the exact
//!    `(project_id, relative_path, run_id)` triple it was issued for via
//!    [`SensitiveReadScope`], and never derivable from model, skill, MCP, or
//!    tool output: there is no `Deserialize`/`From<&str>` path to either
//!    type, and the only route to a grant is a caller that already holds a
//!    [`SensitiveOverrideRequest`] produced by [`evaluate`]. Nothing here
//!    persists a grant beyond the call that presents it -- the plan says
//!    "for that exact ... run," and no migration in this task's scope
//!    stores one.
//!
//! `.xfalcon/` is excluded from indexing/search entirely by
//! `super::index`/`super::search` before classification ever runs (see
//! those modules' doc comments) -- this module has no special-case for it.

use std::path::Path;

use globset::{Glob, GlobBuilder, GlobSet, GlobSetBuilder};
use xfalcon_contracts::RunId;

use super::domain::ProjectId;
use super::path_policy::RelativeProjectPath;

/// Hard ceiling on how many extra "configured secret class" glob patterns a
/// caller may supply to [`SecretClassifier::new`] -- defense against an
/// unbounded/hostile pattern list, mirroring every other `MAX_*` bound in
/// this module tree (e.g. `path_policy::MAX_COMPONENTS`).
pub const MAX_CONFIGURED_SECRET_GLOBS: usize = 128;

#[derive(Debug, thiserror::Error)]
pub enum SensitiveError {
    #[error("too many configured secret-class patterns (maximum {0})")]
    TooManyConfiguredGlobs(usize),
    #[error("configured secret-class pattern {pattern:?} is not a valid glob: {source}")]
    InvalidConfiguredGlob {
        pattern: String,
        #[source]
        source: globset::Error,
    },
}

/// Which secret class, if any, classified a path as sensitive. Variants are
/// deliberately named after the plan's own class list (line 378-381) rather
/// than after the individual filename patterns behind them, so this stays
/// stable as the pattern lists are tuned. `Configured` covers any
/// caller-supplied pattern -- see the module doc comment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretClass {
    DotEnv,
    PrivateKeyMaterial,
    CloudOrKubernetesCredentials,
    PasswordOrVaultExport,
    BrowserProfileSecret,
    SshOrGpgMaterial,
    TokenOrConfigStore,
    Configured,
}

impl SecretClass {
    /// Stable machine code, mirroring `PolicyDenial::code`'s "stable code
    /// for journaling/wire use, never `Display` text" discipline -- kept
    /// here even though nothing journals it yet in this task, so a future
    /// caller (Task 7's dispatch layer) has a stable identifier to log
    /// rather than inventing one against a `Debug` string.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::DotEnv => "dot_env",
            Self::PrivateKeyMaterial => "private_key_material",
            Self::CloudOrKubernetesCredentials => "cloud_or_kubernetes_credentials",
            Self::PasswordOrVaultExport => "password_or_vault_export",
            Self::BrowserProfileSecret => "browser_profile_secret",
            Self::SshOrGpgMaterial => "ssh_or_gpg_material",
            Self::TokenOrConfigStore => "token_or_config_store",
            Self::Configured => "configured",
        }
    }
}

/// Whether `basename` is the checked-in template of an env file, which by
/// convention holds placeholders rather than values.
fn is_env_template(basename: &str) -> bool {
    let lower = basename.to_ascii_lowercase();
    lower.starts_with(".env.")
        && matches!(
            lower.rsplit('.').next(),
            Some("example" | "sample" | "template" | "dist")
        )
}

#[derive(Debug)]
struct ClassPatterns {
    class: SecretClass,
    /// Matched against the path's final component only (e.g. `id_rsa`,
    /// `*.pem`) -- most built-in classes are name/extension shaped.
    names: GlobSet,
    /// Matched against the full, `/`-normalized relative path (e.g.
    /// `**/.ssh/**`) -- used for whole-directory classes.
    paths: GlobSet,
}

impl ClassPatterns {
    fn is_match(&self, basename: &str, normalized_path: &str) -> bool {
        self.names.is_match(basename) || self.paths.is_match(normalized_path)
    }
}

fn glob_set(patterns: &[&str]) -> GlobSet {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        builder.add(case_insensitive_glob(pattern));
    }
    builder
        .build()
        .expect("built-in secret-class glob patterns are valid by construction")
}

/// Every built-in pattern is compiled case-insensitively: a Windows
/// filesystem is case-insensitive by default, and even where it is not
/// (Unix), a differently-cased secret file (`.ENV`, `ID_RSA`) is exactly the
/// kind of over-inclusive false positive this default-deny control accepts
/// on purpose -- see the module doc comment's CWE-798/CWE-312 framing: a
/// missed secret is the asymmetric risk, not an occasional false positive.
fn case_insensitive_glob(pattern: &str) -> Glob {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .build()
        .expect("built-in secret-class glob patterns are valid by construction")
}

fn built_in_classes() -> Vec<ClassPatterns> {
    vec![
        ClassPatterns {
            class: SecretClass::DotEnv,
            names: glob_set(&[".env", ".env.*", ".envrc"]),
            paths: GlobSet::empty(),
        },
        ClassPatterns {
            class: SecretClass::PrivateKeyMaterial,
            names: glob_set(&[
                "id_rsa",
                "id_dsa",
                "id_ecdsa",
                "id_ed25519",
                "identity",
                "*.pem",
                "*.key",
                "*.p12",
                "*.pfx",
                "*.ppk",
                "*.jks",
                "*.keystore",
            ]),
            paths: GlobSet::empty(),
        },
        ClassPatterns {
            class: SecretClass::CloudOrKubernetesCredentials,
            names: glob_set(&[
                "credentials",
                "credentials.json",
                "application_default_credentials.json",
                "service-account*.json",
                "*.kubeconfig",
                "kubeconfig",
                "clouds.yaml",
            ]),
            paths: glob_set(&[
                "**/.aws/**",
                "**/.azure/**",
                "**/.config/gcloud/**",
                "**/.kube/**",
            ]),
        },
        ClassPatterns {
            class: SecretClass::PasswordOrVaultExport,
            names: glob_set(&[
                "*.kdbx",
                "*.kdb",
                "*.opvault",
                "*.1pif",
                "*.1pux",
                "*.agilekeychain",
                "secrets.yml",
                "secrets.yaml",
                "vault.yml",
                "vault.yaml",
            ]),
            paths: GlobSet::empty(),
        },
        ClassPatterns {
            class: SecretClass::BrowserProfileSecret,
            names: glob_set(&[
                "Login Data",
                "Login Data For Account",
                "Cookies",
                "cookies.sqlite",
                "key4.db",
                "key3.db",
                "logins.json",
                "signons.sqlite",
            ]),
            paths: GlobSet::empty(),
        },
        ClassPatterns {
            class: SecretClass::SshOrGpgMaterial,
            names: glob_set(&["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519", "*.ppk"]),
            paths: glob_set(&["**/.ssh/**", "**/.gnupg/**", "**/.gnupg2/**"]),
        },
        ClassPatterns {
            class: SecretClass::TokenOrConfigStore,
            names: glob_set(&[
                ".npmrc",
                ".pypirc",
                ".netrc",
                "_netrc",
                ".git-credentials",
                ".terraformrc",
                "terraform.rc",
                ".dockercfg",
            ]),
            paths: glob_set(&["**/.docker/config.json", "**/.npm/_auth"]),
        },
    ]
}

/// Default-deny classifier over the plan's named secret classes plus any
/// caller-supplied "configured" patterns. See the module doc comment for why
/// configured patterns are a constructor parameter, not persisted state.
/// `Debug`-derived only so tests can `.expect_err`/`.unwrap_err` an
/// `Ok(SecretClassifier)` case; it carries no secret material itself (only
/// compiled glob patterns), so deriving it has no confidentiality
/// implication.
#[derive(Debug)]
pub struct SecretClassifier {
    classes: Vec<ClassPatterns>,
    configured: GlobSet,
    has_configured: bool,
}

impl SecretClassifier {
    /// Builds a classifier from the built-in classes plus `configured_globs`
    /// (each matched against the full, `/`-normalized relative path).
    pub fn new(configured_globs: &[String]) -> Result<Self, SensitiveError> {
        if configured_globs.len() > MAX_CONFIGURED_SECRET_GLOBS {
            return Err(SensitiveError::TooManyConfiguredGlobs(
                MAX_CONFIGURED_SECRET_GLOBS,
            ));
        }
        let mut builder = GlobSetBuilder::new();
        for pattern in configured_globs {
            let glob = GlobBuilder::new(pattern)
                .case_insensitive(true)
                .build()
                .map_err(|source| SensitiveError::InvalidConfiguredGlob {
                    pattern: pattern.clone(),
                    source,
                })?;
            builder.add(glob);
        }
        let configured =
            builder
                .build()
                .map_err(|source| SensitiveError::InvalidConfiguredGlob {
                    pattern: configured_globs.join(","),
                    source,
                })?;
        Ok(Self {
            classes: built_in_classes(),
            has_configured: !configured_globs.is_empty(),
            configured,
        })
    }

    /// Classifier with only the built-in classes -- the common case for
    /// production callers with no extra configured patterns.
    #[must_use]
    pub fn built_in() -> Self {
        Self::new(&[]).expect("no configured globs means construction cannot fail")
    }

    /// Returns the first matching secret class for `relative`, or `None` if
    /// it names no known secret class. Built-in classes are checked in the
    /// (arbitrary but deterministic) order [`built_in_classes`] declares
    /// them, then the configured set -- callers needing "is this sensitive
    /// at all" only need `is_some()`, never the specific ordering.
    #[must_use]
    pub fn classify(&self, relative: &RelativeProjectPath) -> Option<SecretClass> {
        let path = relative.as_path();
        let basename = path
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default();
        let normalized_path = normalized_relative_string(path);

        for class in &self.classes {
            // `.env.example` and its kin are committed placeholders, written
            // to be read and edited; treating them as secrets made the agent
            // unable to keep a project's own template in step with its code.
            if class.class == SecretClass::DotEnv && is_env_template(&basename) {
                continue;
            }
            if class.is_match(&basename, &normalized_path) {
                return Some(class.class);
            }
        }
        if self.has_configured
            && (self.configured.is_match(&normalized_path) || self.configured.is_match(&*basename))
        {
            return Some(SecretClass::Configured);
        }
        None
    }

    /// Like [`Self::classify`], but for a path known to name a *directory*
    /// (never a file): additionally tests every path-glob class against
    /// `relative`'s normalized path with a trailing `/` appended.
    ///
    /// `globset`'s own `**/x/**` -> regex translation requires a literal
    /// path separator immediately after `x` (verified directly against
    /// `globset`'s own test suite), so it matches `parent/x/child` but not
    /// the bare directory name `parent/x` itself -- a project-root child
    /// literally named `.aws`/`.ssh`/`.kube`/`.gnupg`/`.azure` would
    /// otherwise classify as `None` even though every path-glob class here
    /// exists specifically to flag that directory's *contents* as secret
    /// material. Appending the trailing separator before re-testing the
    /// path-glob classes closes that labeling gap for the directory entry
    /// itself, without changing [`Self::classify`]'s behavior for any file
    /// path (a plain file is never classified via this method).
    #[must_use]
    pub fn classify_directory(&self, relative: &RelativeProjectPath) -> Option<SecretClass> {
        if let Some(class) = self.classify(relative) {
            return Some(class);
        }
        let normalized_path = normalized_relative_string(relative.as_path());
        let normalized_path_with_trailing_slash = format!("{normalized_path}/");
        for class in &self.classes {
            if class.paths.is_match(&normalized_path_with_trailing_slash) {
                return Some(class.class);
            }
        }
        if self.has_configured
            && self
                .configured
                .is_match(&normalized_path_with_trailing_slash)
        {
            return Some(SecretClass::Configured);
        }
        None
    }
}

/// Joins `path`'s normal components with `/`, independent of which
/// separator the original raw string used (Windows accepts both `/` and
/// `\`; [`RelativeProjectPath`] preserves the caller's original string
/// verbatim in [`RelativeProjectPath::as_str`], so matching against that
/// directly would make glob patterns like `**/.ssh/**` platform- and
/// input-dependent). Every component of a valid [`RelativeProjectPath`] is
/// [`std::path::Component::Normal`] by construction (see that type's `new`),
/// so this never needs to handle `..`/root/prefix components.
fn normalized_relative_string(path: &Path) -> String {
    let mut normalized = String::new();
    for component in path.components() {
        if !normalized.is_empty() {
            normalized.push('/');
        }
        normalized.push_str(&component.as_os_str().to_string_lossy());
    }
    normalized
}

/// The exact `(project, path, run)` triple a sensitive-read override is
/// scoped to. Mirrors `crate::tools::policy::ApprovalToken`'s
/// `(invocation_id, tool_id)` exact-match scoping, generalized to the three
/// fields the plan names ("for that exact project/path/run").
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SensitiveReadScope {
    project_id: ProjectId,
    relative_path: RelativeProjectPath,
    run_id: RunId,
}

impl SensitiveReadScope {
    #[must_use]
    pub fn new(project_id: ProjectId, relative_path: RelativeProjectPath, run_id: RunId) -> Self {
        Self {
            project_id,
            relative_path,
            run_id,
        }
    }

    #[must_use]
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    #[must_use]
    pub fn relative_path(&self) -> &RelativeProjectPath {
        &self.relative_path
    }

    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }
}

/// Proof a human explicitly overrode the default-deny for one sensitive
/// read. Only ever constructed by [`SensitiveOverrideRequest::grant`], and
/// only matches the exact scope it was issued for -- see the module doc
/// comment for the full non-forgeability argument. Deliberately not
/// `Serialize`: this value must never be persisted, cached across calls, or
/// sent over any wire boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SensitiveOverrideGrant {
    scope: SensitiveReadScope,
}

impl SensitiveOverrideGrant {
    #[must_use]
    pub fn matches(&self, scope: &SensitiveReadScope) -> bool {
        &self.scope == scope
    }
}

/// Returned by [`evaluate`] when `scope` names a classified-sensitive path
/// with no matching grant presented yet. The (future) override UI calls
/// [`Self::grant`] once a human explicitly authorizes *this exact* read --
/// the stand-in for that not-yet-built UI, exactly as
/// `crate::tools::policy::ApprovalRequest::approve` stands in for the future
/// approval UI.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SensitiveOverrideRequest {
    scope: SensitiveReadScope,
    class: SecretClass,
}

impl SensitiveOverrideRequest {
    #[must_use]
    pub fn scope(&self) -> &SensitiveReadScope {
        &self.scope
    }

    #[must_use]
    pub fn class(&self) -> SecretClass {
        self.class
    }

    #[must_use]
    pub fn grant(&self) -> SensitiveOverrideGrant {
        SensitiveOverrideGrant {
            scope: self.scope.clone(),
        }
    }
}

/// Outcome of checking one path against the classifier and whatever
/// override (if any) the caller presented. There is no fourth option, and
/// -- unlike `crate::tools::policy::PolicyDecision` -- no `Deny`: a
/// non-matching or absent grant simply falls back to `RequiresOverride`
/// (fail closed, ask again) rather than a distinct denial, since presenting
/// a grant that does not match is not a distinguishable attack surface here
/// (grants are unforgeable; a mismatch can only mean the caller is
/// requesting a *different* sensitive path than the one it was granted
/// for).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SensitiveDecision {
    NotSensitive,
    RequiresOverride(SensitiveOverrideRequest),
    Overridden(SecretClass),
}

/// Classifies `scope`'s path and checks it against `presented`. Never reads
/// or trusts anything from model/skill/MCP/tool output: `presented` is
/// either `None` or an already-constructed [`SensitiveOverrideGrant`] the
/// caller obtained from a prior [`SensitiveOverrideRequest::grant`] call --
/// there is no code path that manufactures one from content.
#[must_use]
pub fn evaluate(
    classifier: &SecretClassifier,
    scope: SensitiveReadScope,
    presented: Option<&SensitiveOverrideGrant>,
) -> SensitiveDecision {
    let Some(class) = classifier.classify(scope.relative_path()) else {
        return SensitiveDecision::NotSensitive;
    };
    match presented {
        Some(grant) if grant.matches(&scope) => SensitiveDecision::Overridden(class),
        _ => SensitiveDecision::RequiresOverride(SensitiveOverrideRequest { scope, class }),
    }
}
