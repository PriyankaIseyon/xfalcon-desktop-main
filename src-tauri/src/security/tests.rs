use std::collections::BTreeSet;

use serde_json::Value;

use super::capabilities::{
    command_allow_permission, main_window_permissions, APP_COMMANDS, FORBIDDEN_COMMANDS,
    MACOS_WINDOW_PERMISSIONS, PLUGIN_PERMISSIONS,
};

mod account_cleanup;
mod browsing_data;
mod sign_in_gate;
mod uninstall_hooks;

fn registered_commands() -> BTreeSet<String> {
    let source = include_str!("../lib.rs");
    let handler = source
        .split("tauri::generate_handler![")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .expect("lib.rs must register one explicit Tauri handler");

    handler
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            name.rsplit("::")
                .next()
                .expect("registered command has a function name")
                .to_owned()
        })
        .collect()
}

fn build_script_commands() -> BTreeSet<String> {
    let source = include_str!("../../build.rs");
    let list = source
        .split("AppManifest::new().commands(&[")
        .nth(1)
        .and_then(|tail| tail.split(']').next())
        .expect("build.rs must declare one command list");

    list.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.trim_matches('"').to_owned())
        .collect()
}

#[test]
fn command_surface_is_the_exact_explicit_allowlist() {
    let expected = APP_COMMANDS
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let registered = registered_commands();
    assert_eq!(
        build_script_commands(),
        expected,
        "build.rs command list must match APP_COMMANDS so permission generation cannot drift"
    );

    assert_eq!(registered, expected);
    for forbidden in FORBIDDEN_COMMANDS {
        assert!(!registered.contains(forbidden));
    }
    // Pinned as a number, not only as an array length, so the reports
    // capability's own review is legible here: the set the renderer may never
    // reach did not move. `write_file` / `write_text_file` were considered for a
    // download affordance and refused -- saving a report to a project is what
    // replaces that action (spec: reports).
    assert_eq!(
        FORBIDDEN_COMMANDS.len(),
        8,
        "the forbidden set must not grow or shrink without saying why"
    );
    assert!(FORBIDDEN_COMMANDS.contains(&"write_file"));
    assert!(FORBIDDEN_COMMANDS.contains(&"write_text_file"));
}

fn capability() -> Value {
    serde_json::from_str(include_str!("../../capabilities/default.json"))
        .expect("capability JSON must parse")
}

fn macos_window_capability() -> Value {
    serde_json::from_str(include_str!("../../capabilities/macos-window.json"))
        .expect("macOS window capability JSON must parse")
}

fn permission_list(capability: &Value) -> Vec<String> {
    capability["permissions"]
        .as_array()
        .expect("permissions must be an array")
        .iter()
        .map(|permission| {
            permission
                .as_str()
                .expect("permissions must be explicit identifiers")
                .to_owned()
        })
        .collect()
}

fn capability_permissions() -> BTreeSet<String> {
    permission_list(&capability()).into_iter().collect()
}

#[test]
fn main_window_capability_is_local_and_minimal() {
    let capability = capability();

    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert!(
        capability.get("remote").is_none(),
        "remote content must never inherit native capabilities"
    );

    let plugins: BTreeSet<String> = capability_permissions()
        .into_iter()
        .filter(|permission| permission.contains(':'))
        .collect();
    assert_eq!(
        plugins,
        PLUGIN_PERMISSIONS
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    );
    assert!(
        !capability_permissions()
            .iter()
            .any(|permission| permission.ends_with(":default")),
        "no plugin default set is granted: no permission in default.json ends with `:default`"
    );
    // Dialogs are opened by the backend (`app.dialog()`), never by the webview:
    // a renderer that may open the picker itself could hand back a path it chose.
    assert!(
        !capability_permissions()
            .iter()
            .any(|permission| permission.starts_with("dialog:")),
        "the webview may not open native dialogs; the backend opens them"
    );
}

#[test]
fn window_drag_permissions_are_granted_on_macos_only() {
    // The macOS title bar is an overlay, so the header drags and zooms the
    // window only there. Windows keeps its native frame, so the pair lives in a
    // capability limited to macOS, not in default.json, which applies everywhere.
    let capability = macos_window_capability();

    assert_eq!(capability["windows"], serde_json::json!(["main"]));
    assert_eq!(capability["platforms"], serde_json::json!(["macOS"]));
    assert!(
        capability.get("remote").is_none(),
        "remote content must never inherit native capabilities"
    );

    let listed = permission_list(&capability);
    let granted: BTreeSet<String> = listed.iter().cloned().collect();
    assert_eq!(
        granted.len(),
        listed.len(),
        "no window permission may be listed twice"
    );
    assert_eq!(
        granted,
        MACOS_WINDOW_PERMISSIONS
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    );

    let everywhere = capability_permissions();
    for permission in MACOS_WINDOW_PERMISSIONS {
        assert!(
            !everywhere.contains(permission),
            "{permission} must not be granted on every platform"
        );
    }
}

#[test]
fn both_capabilities_are_enabled_in_the_config() {
    // Tauri enables only the identifiers that `app.security.capabilities` lists
    // and drops every other capability file without an error. Without the
    // macOS entry, the header drag and zoom are denied on macOS.
    let config: Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
        .expect("Tauri config must parse");

    assert_eq!(
        config["app"]["security"]["capabilities"],
        serde_json::json!(["default", "main-window-macos"]),
        "a capability file that the config does not list is silently ignored"
    );
    assert_eq!(capability()["identifier"], serde_json::json!("default"));
    assert_eq!(
        macos_window_capability()["identifier"],
        serde_json::json!("main-window-macos")
    );
}

/// The generated permission for `command`, as `build.rs`'s `AppManifest`
/// names its file.
fn permission_files() -> BTreeSet<String> {
    std::fs::read_dir(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("permissions/autogenerated"),
    )
    .expect("generated permissions directory must exist")
    .filter_map(Result::ok)
    .filter_map(|entry| {
        entry
            .path()
            .file_stem()
            .and_then(|stem| stem.to_str())
            .map(str::to_owned)
    })
    .collect()
}

#[test]
fn every_registered_command_has_a_generated_permission() {
    // `build.rs` repeats the names because Tauri's AppManifest cannot import
    // [`APP_COMMANDS`]. A command missing there makes the capability entry
    // unresolvable — the build panics with "Permission allow-x not found".
    // The reverse is silent: a leftover permission file blocks this assertion,
    // which is how a stale `chat_submit_prompt` survived its command's removal.
    // `command_surface_is_the_exact_explicit_allowlist` also parses build.rs
    // so that copy cannot drift from [`APP_COMMANDS`] while a leftover file
    // still exists.
    assert_eq!(
        permission_files(),
        APP_COMMANDS
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>()
    );
}

#[test]
fn every_registered_command_is_admitted_by_the_window_capability() {
    // A command registered in `generate_handler!` that the capability does not
    // admit is registered and UNREACHABLE: the webview's `invoke` is denied
    // before it reaches Rust, so the surface sees a rejection carrying no
    // typed code and shows its fallback message. Nothing else in this suite
    // catches that — the handler list and the capability were previously two
    // hand-maintained lists, each only ever compared against itself, and six
    // commands shipped registered-but-denied because of it.
    //
    // Deriving one from the other is what makes that impossible rather than
    // merely noticed: adding a command to the handler now fails here until it
    // is admitted, and admitting one that is not registered fails too.
    let expected: BTreeSet<String> = APP_COMMANDS
        .into_iter()
        .map(command_allow_permission)
        .collect();
    let admitted: BTreeSet<String> = capability_permissions()
        .into_iter()
        .filter(|permission| !permission.contains(':'))
        .collect();

    assert_eq!(admitted, expected);
    assert_eq!(
        capability_permissions(),
        main_window_permissions()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "default.json must match the derived main-window grant from APP_COMMANDS"
    );
}

#[test]
fn production_webview_has_strict_content_security_policy() {
    let config: Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
        .expect("Tauri config must parse");
    let security = &config["app"]["security"];
    let csp = security["csp"]
        .as_str()
        .expect("production CSP must be an explicit string");

    assert_eq!(config["app"]["withGlobalTauri"], serde_json::json!(false));
    assert!(!csp.contains("'unsafe-inline'"));
    assert!(!csp.contains("'unsafe-eval'"));
    assert!(!csp.contains("https:"));
    assert!(!csp.contains("http://localhost:*"));
    for directive in [
        "default-src 'self'",
        "script-src 'self'",
        "style-src 'self'",
        "connect-src 'self' ipc: http://ipc.localhost",
        // Sandboxed previews (spec: chat-rich-content) frame model documents
        // from the dedicated xf-preview origin (spec: preview-protocol) —
        // both platform spellings of it. The isolation is the separate
        // origin + per-response document CSP + `sandbox="allow-scripts"`;
        // this directive only permits the embedding.
        "frame-src xf-preview: http://xf-preview.localhost",
        "object-src 'none'",
        "base-uri 'none'",
        "frame-ancestors 'none'",
        "form-action 'none'",
    ] {
        assert!(
            csp.contains(directive),
            "missing CSP directive: {directive}"
        );
    }
    // Local-scheme frames inherit the app CSP in WebView2, which refuses the
    // framed document's own inline styles/scripts — the defect that moved
    // previews onto the xf-preview scheme. Nothing may quietly bring
    // data:-framing back.
    assert!(
        !csp.contains("frame-src data:") && !csp.contains("frame-src 'self' data:"),
        "data: frames must not return to the CSP"
    );

    let development_csp = security["devCsp"]
        .as_str()
        .expect("development CSP must be explicit and loopback-only");
    // Both spellings of the loopback HMR endpoint: the dev server listens on
    // the IPv4 literal (see `vite.config.ts`), and `localhost` stays allowed so
    // a differently-resolved environment is not broken by the CSP.
    assert!(development_csp.contains("ws://localhost:1421"));
    assert!(development_csp.contains("ws://127.0.0.1:1421"));
    assert!(!development_csp.contains("ws://localhost:*"));
    assert!(!development_csp.contains("'unsafe-inline'"));
    assert!(!development_csp.contains("'unsafe-eval'"));
}

#[test]
fn main_window_starts_hidden_on_the_dedicated_migration_entry() {
    let config: Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
        .expect("Tauri config must parse");
    let main = &config["app"]["windows"][0];

    assert_eq!(main["label"], serde_json::json!("main"));
    assert_eq!(main["create"], serde_json::json!(false));
    assert_eq!(main["visible"], serde_json::json!(false));
    assert_eq!(main["url"], serde_json::json!("storage-migration.html"));
}

#[test]
fn runtime_installs_redacted_panics_and_never_schedules_a_global_browsing_scrub() {
    let source = include_str!("../lib.rs");
    let hook = source
        .find("install_redacted_panic_hook();")
        .expect("redacted panic hook");
    let runtime = source
        .find("tauri::Builder::default()")
        .expect("Tauri runtime");

    assert!(hook < runtime);
    assert!(!source.contains("clear_all_browsing_data"));
    assert!(source.contains("app_handle.exit(1)"));
}

#[test]
fn runtime_claims_single_instance_before_initializing_other_plugins() {
    let source = include_str!("../lib.rs");
    let single_instance = source
        .find("tauri_plugin_single_instance::init")
        .expect("single-instance plugin");
    let opener = source
        .find("tauri_plugin_opener::init")
        .expect("system-browser opener plugin");

    assert!(
        single_instance < opener,
        "single-instance coordination must initialize before plugins that can open external state"
    );
}

/// Both platform spellings are exercised on every host, and which one is
/// active is chosen with `cfg!` rather than `#[cfg]`: an attribute-gated arm
/// for the other platform never compiles on this gate's single host, which is
/// how a macOS-only navigation defect stays invisible until release.
fn platform_forms<'a>(windows_form: &'a str, other_form: &'a str) -> (&'a str, &'a str) {
    if cfg!(windows) {
        (windows_form, other_form)
    } else {
        (other_form, windows_form)
    }
}

#[test]
fn navigation_policy_allows_only_packaged_content_and_the_exact_development_origin() {
    use crate::security::capabilities::allows_navigation;

    let (packaged, inactive_packaged_origin) = platform_forms(
        "http://tauri.localhost/index.html",
        "tauri://localhost/index.html",
    );
    assert!(allows_navigation(
        &packaged.parse().expect("packaged URL"),
        false
    ));
    assert!(!allows_navigation(
        &packaged.parse().expect("packaged URL"),
        true
    ));

    let development = "http://localhost:1420/dashboard"
        .parse()
        .expect("development URL");
    assert!(allows_navigation(&development, true));
    assert!(allows_navigation(
        &"http://127.0.0.1:1420/index.html"
            .parse()
            .expect("loopback development URL"),
        true
    ));
    assert!(!allows_navigation(&development, false));

    for url in [
        inactive_packaged_origin,
        "tauri://foreign/index.html",
        "https://tauri.localhost/index.html",
        "http://localhost:3000/",
        "http://user@tauri.localhost/index.html",
        "https://chat.xfalcon.ai/",
        "file:///tmp/index.html",
        "data:text/html,unsafe",
    ] {
        assert!(!allows_navigation(
            &url.parse().expect("untrusted URL"),
            true
        ));
    }
}

/// The preview scheme is a SUBFRAME origin, and on WebKit subframes are
/// policed by the same navigation hook as the main document (wry forwards
/// every `decidePolicyForNavigationAction:`, main frame or not). A boundary
/// that names only the app origin cancels the preview load there and the card
/// settles blank, which is why this is a boundary requirement and not a
/// frontend detail.
#[test]
fn navigation_policy_admits_the_preview_origin_and_nothing_beside_it() {
    use crate::security::capabilities::allows_navigation;

    let (preview, inactive_preview_form) = platform_forms(
        "http://xf-preview.localhost/2f0a1c",
        "xf-preview://localhost/2f0a1c",
    );
    let preview_url: tauri::Url = preview.parse().expect("preview URL");
    // The origin does not move between packaged and dev builds: the same
    // handler serves it either way.
    assert!(allows_navigation(&preview_url, false));
    assert!(allows_navigation(&preview_url, true));

    // The whole origin is admitted, not one path shape: the document id is
    // opaque and every route on the scheme belongs to the same handler.
    let (preview_route, _) = platform_forms(
        "http://xf-preview.localhost/fonts/GoogleSans-Variable.ttf",
        "xf-preview://localhost/fonts/GoogleSans-Variable.ttf",
    );
    assert!(allows_navigation(
        &preview_route.parse().expect("preview route URL"),
        false
    ));

    for url in [
        // The form this build cannot produce stays refused, for the reason
        // the app origin's own inactive form does.
        inactive_preview_form,
        // A different host on the preview scheme is a different origin.
        "xf-preview://evil/2f0a1c",
        "xf-preview://xf-preview.localhost.evil.test/2f0a1c",
        // A port makes it a different origin too.
        "xf-preview://localhost:1420/2f0a1c",
        "http://xf-preview.localhost:8080/2f0a1c",
        // Credentials never belong on an origin the app trusts.
        "xf-preview://user@localhost/2f0a1c",
        "http://user@xf-preview.localhost/2f0a1c",
        // Neighbouring schemes are not this scheme.
        "https://xf-preview.localhost/2f0a1c",
        "xf-previews://localhost/2f0a1c",
    ] {
        assert!(
            !allows_navigation(&url.parse().expect("untrusted preview-shaped URL"), true),
            "{url} must not be admitted"
        );
    }
}

#[test]
fn updater_configuration_uses_https_and_pinned_public_key() {
    let config: Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
        .expect("Tauri config must parse");
    assert_eq!(
        config["bundle"]["createUpdaterArtifacts"],
        serde_json::json!(true)
    );

    let updater = &config["plugins"]["updater"];
    let pubkey = updater["pubkey"]
        .as_str()
        .expect("updater pubkey must be configured");
    assert!(!pubkey.trim().is_empty());
    let native_update = include_str!("../desktop_update.rs");
    assert!(
        native_update.contains(pubkey),
        "the bounded native downloader must verify with the configured updater key"
    );

    let staging: Value = serde_json::from_str(include_str!("../../tauri.staging.conf.json"))
        .expect("staging config must parse");
    assert_eq!(
        staging["plugins"]["updater"]["endpoints"],
        serde_json::json!([
            "https://d3rdmiksm582v0.cloudfront.net/desktop/channels/staging/latest.json"
        ]),
        "staging must use its reviewed account-specific delivery endpoint"
    );

    let endpoints = updater["endpoints"]
        .as_array()
        .expect("updater endpoints must be an array");
    assert_eq!(
        endpoints,
        &[serde_json::json!(
            "https://github.com/iSeyon-Analytics/xfalcon-desktop/releases/latest/download/latest.json"
        )],
        "every update source must be an explicitly reviewed compile-time endpoint"
    );
}

#[test]
fn release_workflow_fails_closed_and_separates_signing_from_publication() {
    let workflow = include_str!("../../../.github/workflows/desktop-build.yml");

    for required in [
        "TAURI_SIGNING_PRIVATE_KEY",
        "APPLE_CERTIFICATE",
        "APPLE_SIGNING_IDENTITY",
        "APPLE_ID",
        "APPLE_PASSWORD",
        "APPLE_TEAM_ID",
        "WINDOWS_CERTIFICATE",
        "WINDOWS_CERTIFICATE_PASSWORD",
        "WINDOWS_CERTIFICATE_THUMBPRINT",
    ] {
        assert!(
            workflow.contains(&format!("secrets.{required}")),
            "release workflow must read {required} from repository secrets"
        );
    }
    // TAURI_SIGNING_PRIVATE_KEY (the updater signature) is unconditionally
    // required. macOS signing and notarization are mandatory too: a missing or
    // empty Apple secret fails the release before anything is built, and there
    // is no "unsigned macOS" path left in the workflow. Windows signing is
    // deferred, so its absence still degrades to an unsigned build with a
    // warning. Making Windows mandatory must be a deliberate edit here.
    assert!(workflow.contains("Required secret $name is missing or empty"));
    assert!(workflow.contains("refusing to build an unsigned or un-notarized macOS release"));
    assert!(
        !workflow.contains("APPLE_CERTIFICATE is not configured"),
        "macOS signing is mandatory: no warn-and-continue path may return"
    );
    assert!(
        !workflow.contains("env.APPLE_CERTIFICATE != ''"),
        "the macOS notarize and Gatekeeper steps must not be skippable"
    );
    assert!(workflow.contains("WINDOWS_CERTIFICATE is not configured"));
    assert!(workflow.contains("group: desktop-release"));
    // No verify/quality-gate job runs here -- ci.yml (currently disabled for
    // Actions-minutes cost) or a local `scripts/gate.sh` run is what's
    // expected to have kept main green before a release is cut.
    assert!(!workflow.contains("needs: verify"));
    assert!(workflow.contains("persist-credentials: false"));
    assert!(workflow.contains("environment: desktop-release"));
    // Releases are restricted to `staging` while the tenant MCP OAuth client is
    // bundled into `resources/tenant-config.json` in plain text. Both gates are
    // pinned: the job-level condition, which also gates `publish` and
    // `stage-artifacts` through `needs`, and the in-job source check behind it.
    // `main` must appear in NEITHER, or a release could be cut from it again.
    assert!(workflow.contains("if: github.ref == 'refs/heads/staging'"));
    assert!(workflow.contains("if: github.ref != 'refs/heads/staging'"));
    assert!(
        !workflow.contains("github.ref == 'refs/heads/main'"),
        "re-enabling main must be a deliberate edit to this test as well"
    );
    assert!(workflow.contains("attest-build-provenance@e8998f949152b193b063cb0ec769d69d929409be"));
    // Attestation storage is gated on GitHub org billing plan; don't let its
    // absence block a release.
    assert!(workflow.contains("continue-on-error: true"));
    assert!(workflow.contains("anchore/sbom-action@e22c389904149dbc22b58101806040fa8d37a610"));
    assert!(!workflow.contains("dtolnay/rust-toolchain@stable"));
    assert!(!workflow.contains("swatinem/rust-cache@v2"));
    assert!(!workflow.contains("tauri-apps/tauri-action@fce9c6108b31ea247710505d3aaaa893ee6768d4"));
    assert!(workflow.contains("needs: build"));
    assert!(workflow.contains("Prepare one fail-closed update manifest"));
    assert!(
        workflow.contains("Release tag $RELEASE_TAG already exists; release bytes are immutable")
    );
    assert!(workflow.contains("scripts/prepare-desktop-release.mjs"));
    let configuration: serde_json::Value =
        serde_saphyr::from_str(workflow).expect("release workflow must be valid YAML");
    let writers: BTreeSet<_> = configuration["jobs"]
        .as_object()
        .expect("release workflow must declare jobs")
        .iter()
        .filter(|(_, job)| job["permissions"]["contents"].as_str() == Some("write"))
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(writers, BTreeSet::from(["publish", "cleanup-releases"]));
    // Every Apple secret is required by the verify step, and the steps that
    // notarize and assess the result run on every macOS build, unconditionally.
    let build_steps = configuration["jobs"]["build"]["steps"]
        .as_array()
        .expect("build job must declare steps");
    let step = |name: &str| {
        build_steps
            .iter()
            .find(|step| step["name"].as_str() == Some(name))
            .unwrap_or_else(|| panic!("release workflow must keep the step {name:?}"))
    };
    let verify = step("Verify updater and Apple signing secrets");
    let verify_script = verify["run"]
        .as_str()
        .expect("verify step must run a script");
    for secret in [
        "TAURI_SIGNING_PRIVATE_KEY",
        "APPLE_CERTIFICATE",
        "APPLE_CERTIFICATE_PASSWORD",
        "APPLE_SIGNING_IDENTITY",
        "APPLE_ID",
        "APPLE_PASSWORD",
        "APPLE_TEAM_ID",
    ] {
        assert!(
            verify_script.contains(secret),
            "the macOS verify step must require {secret}"
        );
        assert!(
            verify["env"][secret].as_str().is_some(),
            "the macOS verify step must read {secret} from the environment"
        );
    }
    assert!(
        verify_script.contains("exit 1"),
        "a missing Apple secret must fail the job"
    );
    for name in [
        "Configure Apple signing and notarization",
        "Notarize and staple the macOS disk image",
        "Verify the macOS artifacts pass Gatekeeper",
    ] {
        assert_eq!(
            step(name)["if"].as_str(),
            Some("matrix.platform == 'macos-latest'"),
            "{name} must run on every macOS build"
        );
    }
    assert!(!workflow.contains("GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}"));
    assert!(!workflow.contains("releaseDraft:"));
    assert!(workflow.contains("certificateThumbprint = $certificate.Thumbprint"));
    assert!(workflow.contains("tsp = $true"));
    // Gatekeeper assesses a downloaded disk image before anything is mounted,
    // so a DMG that is signed but not notarized is refused however good the
    // app inside it is -- v0.5.2 shipped that way. Stapling the image, and
    // then holding the build to what Gatekeeper actually says, is what keeps a
    // published DMG openable.
    assert!(workflow.contains("xcrun stapler staple"));
    assert!(workflow.contains("scripts/verify-macos-release.sh"));
}

#[test]
fn release_cleanup_requires_successful_staging_delivery() {
    for (workflow, predecessor) in [
        (
            include_str!("../../../.github/workflows/desktop-build.yml"),
            "stage-artifacts",
        ),
        (
            include_str!("../../../.github/workflows/desktop-promote-staging.yml"),
            "promote",
        ),
    ] {
        let configuration: serde_json::Value =
            serde_saphyr::from_str(workflow).expect("release workflow must be valid YAML");
        assert_eq!(
            configuration["permissions"]["contents"].as_str(),
            Some("read")
        );
        let cleanup = &configuration["jobs"]["cleanup-releases"];
        assert_eq!(cleanup["needs"].as_str(), Some(predecessor));
        assert_eq!(
            cleanup["if"].as_str(),
            Some("github.ref == 'refs/heads/staging'")
        );
        assert_eq!(cleanup["permissions"]["contents"].as_str(), Some("write"));
        assert_eq!(cleanup["environment"].as_str(), Some("desktop-release"));
        assert_eq!(
            cleanup["concurrency"]["group"].as_str(),
            Some("desktop-release-cleanup")
        );
        assert_eq!(
            cleanup["concurrency"]["cancel-in-progress"].as_bool(),
            Some(false)
        );
    }
}
