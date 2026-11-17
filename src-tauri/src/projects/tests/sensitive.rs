//! Adversarial coverage for `super::super::sensitive`: default-deny
//! classification for every named secret class, the "configured secret
//! class" extension point, and the exact-scoped, unforgeable override
//! grant.

use xfalcon_contracts::RunId;

use crate::projects::{
    evaluate, ProjectId, RelativeProjectPath, SecretClass, SecretClassifier, SensitiveDecision,
    SensitiveError, SensitiveReadScope, MAX_CONFIGURED_SECRET_GLOBS,
};

fn relative(raw: &str) -> RelativeProjectPath {
    RelativeProjectPath::new(raw).expect("valid relative path fixture")
}

fn classify(raw: &str) -> Option<SecretClass> {
    SecretClassifier::built_in().classify(&relative(raw))
}

// ── Ordinary files are never classified ─────────────────────────────────

#[test]
fn ordinary_source_files_are_not_sensitive() {
    for path in [
        "src/main.rs",
        "README.md",
        "package.json",
        "Cargo.toml",
        "notes/todo.txt",
        "public_key.pub",
        ".gitignore",
    ] {
        assert_eq!(classify(path), None, "unexpectedly classified: {path}");
    }
}

// ── `.env*` ──────────────────────────────────────────────────────────────

#[test]
fn dot_env_files_are_classified() {
    for path in [".env", ".env.local", ".env.production", ".envrc"] {
        assert_eq!(
            classify(path),
            Some(SecretClass::DotEnv),
            "not classified: {path}"
        );
    }
}

#[test]
fn dot_env_classification_is_case_insensitive() {
    assert_eq!(classify(".ENV"), Some(SecretClass::DotEnv));
}

// ── PEM / private keys ───────────────────────────────────────────────────

#[test]
fn private_key_material_is_classified() {
    for path in [
        "id_rsa",
        "keys/id_ed25519",
        "certs/server.pem",
        "certs/server.key",
        "client.p12",
        "client.pfx",
    ] {
        assert_eq!(
            classify(path),
            Some(SecretClass::PrivateKeyMaterial),
            "not classified: {path}"
        );
    }
}

#[test]
fn public_key_files_are_never_classified_as_private_key_material() {
    // `.pub` files are the whole point of asymmetric key material -- meant
    // to be shared -- so they must never trip the same deny rule as the
    // private half.
    for path in ["id_rsa.pub", "keys/id_ed25519.pub", "authorized_keys"] {
        assert_eq!(classify(path), None, "wrongly classified: {path}");
    }
}

// ── Cloud / Kubernetes credentials ──────────────────────────────────────

#[test]
fn cloud_and_kubernetes_credentials_are_classified() {
    for path in [
        "credentials.json",
        "application_default_credentials.json",
        "infra/.aws/credentials",
        "infra/.kube/config",
        "service-account-prod.json",
    ] {
        assert_eq!(
            classify(path),
            Some(SecretClass::CloudOrKubernetesCredentials),
            "not classified: {path}"
        );
    }
}

// ── Password / vault exports ────────────────────────────────────────────

#[test]
fn password_and_vault_exports_are_classified() {
    for path in ["backup.kdbx", "export.1pif", "config/secrets.yaml"] {
        assert_eq!(
            classify(path),
            Some(SecretClass::PasswordOrVaultExport),
            "not classified: {path}"
        );
    }
}

// ── Browser profile secrets ──────────────────────────────────────────────

#[test]
fn browser_profile_secrets_are_classified() {
    for path in [
        "Default/Login Data",
        "profile/Cookies",
        "firefox/key4.db",
        "firefox/logins.json",
    ] {
        assert_eq!(
            classify(path),
            Some(SecretClass::BrowserProfileSecret),
            "not classified: {path}"
        );
    }
}

// ── SSH / GPG material ───────────────────────────────────────────────────

#[test]
fn ssh_and_gpg_material_is_classified() {
    for path in [
        "home/.ssh/config",
        ".gnupg/private-keys-v1.d/secret",
        "nested/.ssh/known_hosts",
    ] {
        assert_eq!(
            classify(path),
            Some(SecretClass::SshOrGpgMaterial),
            "not classified: {path}"
        );
    }
}

/// A path can satisfy more than one class — `.ssh/id_rsa` is both SSH
/// material and private-key material. `SecretClassifier::classify`'s contract
/// states the winning class among overlapping matches is "arbitrary but
/// deterministic" and that callers need only `is_some()`, so asserting one
/// specific label here would test declaration order rather than behaviour.
/// What must hold — and all any caller relies on — is that it is classified
/// at all.
#[test]
fn a_path_matching_two_classes_is_still_classified_sensitive() {
    for path in [".ssh/id_rsa", ".ssh/id_ed25519", "home/.ssh/id_ecdsa"] {
        assert!(
            classify(path).is_some(),
            "must be treated as secret material: {path}"
        );
    }
}

// ── Token / config stores ────────────────────────────────────────────────

#[test]
fn token_and_config_stores_are_classified() {
    for path in [
        ".npmrc",
        ".pypirc",
        ".netrc",
        ".git-credentials",
        "home/.docker/config.json",
    ] {
        assert_eq!(
            classify(path),
            Some(SecretClass::TokenOrConfigStore),
            "not classified: {path}"
        );
    }
}

// ── Configured secret classes ────────────────────────────────────────────

#[test]
fn configured_glob_patterns_extend_the_built_in_set() {
    let classifier =
        SecretClassifier::new(&["**/*.internal-secret".to_owned()]).expect("valid pattern");
    assert_eq!(
        classifier.classify(&relative("config/db.internal-secret")),
        Some(SecretClass::Configured)
    );
    assert_eq!(classifier.classify(&relative("config/db.txt")), None);
}

#[test]
fn an_invalid_configured_glob_is_rejected() {
    let error = SecretClassifier::new(&["[".to_owned()]).expect_err("malformed glob");
    assert!(matches!(
        error,
        SensitiveError::InvalidConfiguredGlob { .. }
    ));
}

// ── Bare directory names for path-glob secret classes ────────────────────

#[test]
fn a_bare_secret_directory_name_is_not_classified_by_plain_classify() {
    // `**/.aws/**` requires a literal separator after `.aws`, so the bare
    // directory name itself does not match via `classify` -- only its
    // descendants do (see `classify_directory`'s doc comment).
    let classifier = SecretClassifier::built_in();
    assert_eq!(classifier.classify(&relative(".aws")), None);
    assert_eq!(classifier.classify(&relative("infra/.ssh")), None);
}

#[test]
fn classify_directory_flags_a_bare_secret_directory_name() {
    let classifier = SecretClassifier::built_in();
    for path in [".aws", ".ssh", ".kube", ".gnupg", "infra/.aws"] {
        assert!(
            classifier.classify_directory(&relative(path)).is_some(),
            "expected {path} to be flagged sensitive as a directory"
        );
    }
}

#[test]
fn classify_directory_still_classifies_a_file_path_the_same_as_classify() {
    // `classify_directory` must not diverge from `classify` for anything
    // that already matches without the trailing-slash special case (e.g. a
    // basename-shaped class, or a path-glob class matching a descendant).
    let classifier = SecretClassifier::built_in();
    assert_eq!(
        classifier.classify_directory(&relative(".env")),
        classifier.classify(&relative(".env"))
    );
    assert_eq!(
        classifier.classify_directory(&relative(".aws/credentials")),
        classifier.classify(&relative(".aws/credentials"))
    );
}

#[test]
fn configured_glob_count_boundary_is_enforced() {
    let at_limit: Vec<String> = (0..MAX_CONFIGURED_SECRET_GLOBS)
        .map(|index| format!("**/pattern-{index}"))
        .collect();
    assert!(SecretClassifier::new(&at_limit).is_ok());

    let over_limit: Vec<String> = (0..=MAX_CONFIGURED_SECRET_GLOBS)
        .map(|index| format!("**/pattern-{index}"))
        .collect();
    let error = SecretClassifier::new(&over_limit).expect_err("over the configured glob limit");
    assert!(matches!(
        error,
        SensitiveError::TooManyConfiguredGlobs(limit) if limit == MAX_CONFIGURED_SECRET_GLOBS
    ));
}

// ── Override grant: exact scoping, unforgeability by construction ───────

fn scope(project: ProjectId, path: &str, run: &str) -> SensitiveReadScope {
    SensitiveReadScope::new(project, relative(path), RunId::from(run))
}

#[test]
fn a_non_sensitive_path_never_requires_an_override() {
    let classifier = SecretClassifier::built_in();
    let decision = evaluate(
        &classifier,
        scope(ProjectId::new(), "src/main.rs", "run-1"),
        None,
    );
    assert_eq!(decision, SensitiveDecision::NotSensitive);
}

#[test]
fn a_sensitive_path_with_no_presented_override_requires_one() {
    let classifier = SecretClassifier::built_in();
    let decision = evaluate(&classifier, scope(ProjectId::new(), ".env", "run-1"), None);
    let SensitiveDecision::RequiresOverride(request) = decision else {
        panic!("expected RequiresOverride, got a different decision");
    };
    assert_eq!(request.class(), SecretClass::DotEnv);
}

#[test]
fn granting_the_exact_pending_request_satisfies_the_same_scope() {
    let classifier = SecretClassifier::built_in();
    let project = ProjectId::new();
    let pending_scope = scope(project, ".env", "run-1");

    let SensitiveDecision::RequiresOverride(request) =
        evaluate(&classifier, pending_scope.clone(), None)
    else {
        panic!("expected a pending override request");
    };
    let grant = request.grant();

    let decision = evaluate(&classifier, pending_scope, Some(&grant));
    assert_eq!(decision, SensitiveDecision::Overridden(SecretClass::DotEnv));
}

#[test]
fn a_grant_does_not_satisfy_a_different_path_in_the_same_project_and_run() {
    let classifier = SecretClassifier::built_in();
    let project = ProjectId::new();
    let SensitiveDecision::RequiresOverride(request) =
        evaluate(&classifier, scope(project, ".env", "run-1"), None)
    else {
        panic!("expected a pending override request");
    };
    let grant = request.grant();

    let decision = evaluate(
        &classifier,
        scope(project, ".env.production", "run-1"),
        Some(&grant),
    );
    assert!(matches!(decision, SensitiveDecision::RequiresOverride(_)));
}

#[test]
fn a_grant_does_not_satisfy_a_different_project_for_the_same_path_and_run() {
    let classifier = SecretClassifier::built_in();
    let SensitiveDecision::RequiresOverride(request) =
        evaluate(&classifier, scope(ProjectId::new(), ".env", "run-1"), None)
    else {
        panic!("expected a pending override request");
    };
    let grant = request.grant();

    let decision = evaluate(
        &classifier,
        scope(ProjectId::new(), ".env", "run-1"),
        Some(&grant),
    );
    assert!(matches!(decision, SensitiveDecision::RequiresOverride(_)));
}

#[test]
fn a_grant_does_not_satisfy_a_different_run_for_the_same_project_and_path() {
    let classifier = SecretClassifier::built_in();
    let project = ProjectId::new();
    let SensitiveDecision::RequiresOverride(request) =
        evaluate(&classifier, scope(project, ".env", "run-1"), None)
    else {
        panic!("expected a pending override request");
    };
    let grant = request.grant();

    let decision = evaluate(&classifier, scope(project, ".env", "run-2"), Some(&grant));
    assert!(matches!(decision, SensitiveDecision::RequiresOverride(_)));
}
