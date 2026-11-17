# Operations

Source version 0.5.51 · reviewed 2026-11-04 · **How-to** · audience: release engineers, operators, support

Describes the 0.5.51 source baseline, not release approval or general availability.

Building, signing, shipping, updating, and diagnosing the desktop app. Local
development is in [getting-started.md](getting-started.md); user-facing failures
are in [troubleshooting.md](troubleshooting.md). The staging update channel (AWS
delivery) is in [desktop-updates-staging.md](desktop-updates-staging.md).

> No version is generally available. Repository version numbers are not
> release-support commitments. Read [status.md](status.md) and
> [compliance.md](compliance.md) § open gaps before shipping to anyone.

---

## Pipelines

Four workflows live in `.github/workflows/` ([decisions.md](decisions.md)
ADR-026). The two per-change workflows are disabled in GitHub Actions to control
runner cost. The release workflow and the promotion workflow run on demand.

| Workflow                      | Trigger                                                                    | Does                                                                                                                                                                                      |
| ----------------------------- | -------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `ci.yml`                      | configured for pushes to `main` and pull requests; **disabled in Actions** | format, lint, the frontend suite, `cargo fmt`, `clippy -D warnings`, `cargo test --workspace --locked` and `cargo deny check` on `macos-latest` **and** `windows-latest`                  |
| `chat-slice-e2e.yml`          | configured for pushes and pull requests; **disabled in Actions**           | `unix-rust` (`cargo test --workspace --locked` on ubuntu-22.04), `chat-slice-e2e` (Playwright over the mocked native boundary) and `visual-regression` (`npm run test:visual` on Windows) |
| `desktop-build.yml`           | `workflow_dispatch` only, and only for the `staging` ref                   | native Rust format, Clippy and tests on macOS and Windows, then package; updater and macOS signing are required, Windows Authenticode is optional; publish and stage jobs follow          |
| `desktop-promote-staging.yml` | `workflow_dispatch` only, `staging` ref                                    | verify a staged candidate and advance the staging channel pointer. Use it to retry an interrupted promotion                                                                               |

`scripts/gate.sh` (`npm run gate`) is the local change gate. It runs format,
typecheck, lint, the style, tooltip and matrix checks, the frontend suite,
`cargo fmt`, Clippy, `cargo test --workspace --locked` and `cargo deny`
([contributing.md](contributing.md) § 4).

`desktop-build.yml` has **no separate source-gate job**. Its `build` job runs
`cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`
and `cargo test --workspace --locked` on each runner before it packages. It runs
no frontend suite and no `cargo deny`. Run `npm run gate` first.

The one Cargo feature, `custom-protocol`, is not enabled by the gate or by the
release job's Rust checks, and no step uses `--all-features`.

---

## Cutting a release

### Pre-flight — do these in order

1. **Run `npm run gate` locally and record the result.** The release job repeats
   only Rust format, Clippy and workspace tests on macOS and Windows. The latest
   full-gate record (Windows, 2026-10-16) passed. It does not cover macOS: run
   platform-specific checks on a Mac before changes to macOS-only code, such as
   `window_chrome.rs`.
2. **Bump all six app version fields consistently:** `package.json`; both root
   version fields in `package-lock.json`; `[package] version` in
   `src-tauri/Cargo.toml`; `version` in `src-tauri/tauri.conf.json`; and the
   `xfalcon-desktop` package entry in `Cargo.lock`. The workflow checks the tag
   against `tauri.conf.json` only.
3. **Update the source-version surfaces:** the version lines in `README.md` and
   `docs/configuration.md`, the inventory in `docs/README.md`, and the new release
   notes at `docs/releases/v<version>.md`. The `publish` job reads the notes for
   the manifest and release body and fails without them. Link to the notes from
   the README and docs index.
4. **Know the tenant.** One build serves one tenant
   ([decisions.md](decisions.md) ADR-027). Have the DA base URL, Casdoor issuer,
   client id, and organization to hand.
5. **Confirm the Casdoor application allows the loopback redirect** for that
   tenant: `http://127.0.0.1:19847/callback`, and `:19848` if MCP OAuth is in
   use.
6. **Dispatch from the `staging` ref.** The `build` job has the condition
   `github.ref == 'refs/heads/staging'`, and `publish` and `stage-artifacts`
   depend on it. A dispatch from `main` builds nothing. The reason is in the
   workflow: a build packages the tenant's MCP OAuth client into
   `resources/tenant-config.json` in plain text, an arrangement accepted for
   staging only (`openspec/changes/persist-mcp-oauth-client`).
7. **Confirm the Windows uninstall hook is wired.** `src-tauri/tauri.conf.json`
   must set `bundle.windows.nsis.installerHooks` to `windows/hooks.nsh`, and that
   file must exist. The Rust test `security::tests::uninstall_hooks` checks both.
   Source and unit-test coverage do not verify uninstall behavior on a real system.
   See [Uninstall and local-data removal](#uninstall-and-local-data-removal).
8. **Inspect release evidence for each platform.** On macOS, confirm the
   `Developer ID Application` identity, notarization acceptance and staple steps.
   Updater signing is mandatory on both platforms; Apple signing and notarization
   are mandatory on macOS. Windows Authenticode is optional, so a green Windows
   workflow can produce unsigned installers. Verify the produced artifacts and
   their signatures before treating them as release evidence. See
   [code-signing.md](code-signing.md).

### Dispatch

Actions → **desktop-build** → _Run workflow_, on `staging`. Five inputs are
required. One is optional:

| Input                  | Example                                                                                                                  |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `version`              | `v0.5.29` — becomes the tag and release name. It must equal `v` plus the `tauri.conf.json` version                       |
| `da_base_url`          | `https://tenant.example.com`                                                                                             |
| `casdoor_issuer`       | `https://auth.example.com`                                                                                               |
| `casdoor_client_id`    | the desktop public client id                                                                                             |
| `casdoor_organization` | tenant slug                                                                                                              |
| `mcp_client_id`        | optional MCP OAuth fallback client id, bundled on both platforms; register `http://127.0.0.1:19848/callback` on it first |

The four tenant values are OAuth client-registration information, not secrets,
which is why they are plain inputs rather than stored anywhere. The client secret
for a confidential `mcp_client_id` comes from the `MCP_CLIENT_SECRET` secret (its
scope is stated in [Signing and secrets](#signing-and-secrets)), never from an
input, because inputs are recorded in the run. The workflow writes the secret
into the bundle only beside its id.

### What the workflow does

**`build`** (one job per platform: `macos-latest` builds a universal binary,
`windows-latest` builds NSIS and MSI installers; cross-compiling from Linux is not
possible, which is why the matrix exists):

1. **Checks out** with `persist-credentials: false`, sets up Node 22 and stable
   Rust (plus both Apple targets on macOS), and runs `npm ci`.
2. **Verifies `TAURI_SIGNING_PRIVATE_KEY` is present**, in shell, and fails the
   run with an explicit error if it is empty. (This check lives in the shell
   rather than an `if:` expression on purpose — an `if:` cannot distinguish an
   absent secret from an empty one.) For macOS, it fails the run when any of the six Apple
   secrets is absent. For Windows, it requires the updater key and only _warns_ when the
   Authenticode certificate is absent.
3. **Verifies the release version** against `tauri.conf.json`.
4. **Renders `src-tauri/resources/tenant-config.json`** from the four tenant
   inputs, plus `mcp_client_id` and `mcp_client_secret` when supplied. It fails if
   a required value is empty, or if a secret has no id.
5. **Configures platform signing.** The macOS job exports all six Apple values
   unconditionally. Item 2 has already failed the run if any of them is missing.
   The Windows job imports the PFX into the current-user store, checks its
   thumbprint against `WINDOWS_CERTIFICATE_THUMBPRINT`, and passes a `sha256`
   digest and a timestamp URL to Tauri. With no Windows certificate, the Windows
   build is unsigned.
6. **Runs `cargo fmt --check`, Clippy `-D warnings` and `cargo test --workspace --locked`**
   natively on the runner.
7. **Builds** via an immutable `tauri-apps/tauri-action` commit, with
   `--config src-tauri/tauri.staging.conf.json` when the ref is `staging`.
8. **Verifies the bundle carries its tenant config**
   (`scripts/verify-tenant-config.mjs`) on both platforms.
9. **On macOS with signing configured:** notarizes and staples the DMG
   (`xcrun notarytool`, `xcrun stapler`), then runs `scripts/verify-macos-release.sh`
   on each DMG and updater archive.
10. **Removes** the imported Windows certificate and clears the Apple environment.
11. **Generates a CycloneDX SBOM** (`anchore/sbom-action`).
12. **Attests** the bundles and the SBOM (`actions/attest-build-provenance`). This
    step is `continue-on-error`, because attestation storage depends on the
    GitHub plan of the organisation.
13. **Uploads** the bundles and SBOM as workflow artifacts.

**`publish`** (environment `desktop-release`, `contents: write`):

1. Downloads both platforms' artifacts.
2. Runs `scripts/prepare-desktop-release.mjs`. It requires an NSIS, an MSI and a
   macOS updater archive, each with a non-empty signature, and rejects duplicate
   asset names and a tag that is not `v` plus the version. It writes one
   `latest.json` with seven platform keys.
3. Refuses to continue if the tag already exists, then creates **one draft
   release** named `xFalcon Desktop <version>` with the assets and the notes.

**`stage-artifacts`** (staging ref only; environment `desktop-staging`, OIDC
credentials for the staging publisher role):

1. Prepares a staging manifest whose download URLs point at the CloudFront
   origin. `DESKTOP_UPDATE_BASE_URL` must equal the origin of the compiled
   staging endpoint.
2. Runs the publisher contract tests, uploads an immutable candidate, then
   verifies CloudFront delivery and advances the channel pointer
   (`scripts/publish-desktop-staging.mjs stage` and `promote`). Details are in
   [desktop-updates-staging.md](desktop-updates-staging.md).

### Post-flight

1. **Install the artifact and launch it.** Nothing in CI does this
   ([compliance.md](compliance.md) CM-023), so it is on you. Confirm at minimum:
   the window appears, sign-in completes against the tenant, one chat turn
   streams, and a document export opens the save dialog.
2. **Confirm `latest.json`** is attached to the draft release and names the right
   platform targets.
3. **Check the staging channel.** A staging build reads
   `https://d3rdmiksm582v0.cloudfront.net/desktop/channels/staging/latest.json`
   (`tauri.staging.conf.json`). The `stage-artifacts` job advances that pointer
   after the draft exists.
4. **Publish the draft** when you want the GitHub endpoint to serve it. The base
   updater endpoint (`tauri.conf.json`) resolves
   `releases/latest/download/latest.json`, and GitHub shows no release there
   until the draft is published.

---

## Signing and secrets

| Secret                                        | Required           | Purpose                                                                         |
| --------------------------------------------- | ------------------ | ------------------------------------------------------------------------------- |
| `TAURI_SIGNING_PRIVATE_KEY`                   | **yes**            | Minisign key for updater artifacts. The run fails without it.                   |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`          | if the key has one |                                                                                 |
| `APPLE_CERTIFICATE`                           | **yes (macOS)**    | Developer ID `.p12`, base64. Absent: the macOS verification step fails the job. |
| `APPLE_CERTIFICATE_PASSWORD`                  | **yes (macOS)**    |                                                                                 |
| `APPLE_SIGNING_IDENTITY`                      | **yes (macOS)**    |                                                                                 |
| `APPLE_ID`, `APPLE_PASSWORD`, `APPLE_TEAM_ID` | **yes (macOS)**    | Notarization.                                                                   |
| `WINDOWS_CERTIFICATE`                         | optional           | Authenticode PFX, base64. Absent: the Windows job builds unsigned.              |
| `WINDOWS_CERTIFICATE_PASSWORD`                | with the above     | Opens the PFX for the scoped runner import.                                     |
| `WINDOWS_CERTIFICATE_THUMBPRINT`              | with the above     | Rejects an unexpected imported certificate.                                     |
| `MCP_CLIENT_SECRET`                           | optional           | Secret of the bundled MCP fallback client. Used only with `mcp_client_id`.      |

| Variable                  | Environment       | Purpose                                         |
| ------------------------- | ----------------- | ----------------------------------------------- |
| `DESKTOP_UPDATE_BASE_URL` | `desktop-staging` | The CloudFront HTTPS origin, no trailing slash. |

The release workflow writes the Apple secrets to `GITHUB_ENV` under a random multi-line
delimiter, so a secret value that contains a newline cannot inject another variable.

Only the `build` job reads the secrets in the table above. It runs in the
`desktop-release` environment, so GitHub gives it that environment's secrets and
any repository secret of the same name. When both exist, the environment value is
used. This repository does not record which scope holds each secret, and the
repository settings were not read in this review (unverified).

The `build` and `publish` jobs run in the `desktop-release` environment. The
`stage-artifacts` and promotion jobs run in `desktop-staging`. Configure
required reviewers and restrict which branches can use each environment before
the first production release.

### First-time macOS signing setup

Platform signing for macOS is **mandatory** in the current workflow. If any of the six
Apple values or the updater key is missing, the job fails before it builds. The
notarize and staple steps run on macOS; Windows Authenticode remains optional. The
workflow definition is source evidence only. Verify the run and packaged artifact before
claiming that a release is signed, notarized or usable.

The account facts in this section (certificate type, identity string, team ids,
expiry date) are not recorded anywhere in this repository. They are unverified
here.

An **Apple Development** certificate is not a substitute for Developer ID: it
signs cleanly, but Apple refuses to notarize it and Gatekeeper blocks it
everywhere except a registered device.

1. In Xcode, **Settings → Accounts → (team) → Manage Certificates → + →
   Developer ID Application**. This needs the Account Holder or Admin role on a
   paid Apple Developer Program membership.
2. Export a `.p12` from **Keychain Access → My Certificates**. Select the
   certificate _row_ and export that, so the private key goes with it — a
   `.cer` straight from the portal is the public half only, and a runner given
   one imports something it cannot sign with. Confirm the pair exists first:
   `security find-identity -v -p codesigning` lists a certificate only when its
   private key is present. `security export` cannot narrow to a single
   identity (`-t identities` emits all of them), which is why this step is the
   GUI.
3. `base64 -i cert.p12 | pbcopy` → `APPLE_CERTIFICATE`; the export password →
   `APPLE_CERTIFICATE_PASSWORD`.
4. `APPLE_SIGNING_IDENTITY` is the full identity string —
   `Developer ID Application: iSeyon Analytics LLC (84L8U2R5QB)`. Read it back
   with `security find-identity -v -p codesigning`.
5. `APPLE_PASSWORD` is an **app-specific password** from appleid.apple.com, not
   the Apple ID password. `APPLE_ID` is the Apple ID it belongs to, and
   `APPLE_TEAM_ID` is `84L8U2R5QB` — the team that _issued the Developer ID
   certificate_. Note it differs from the `NV2MHAH383` team the Apple
   Development certificates sit under; notarization fails if the two disagree.

The Developer ID certificate expires **2027-02-01**. A secure timestamp keeps
already-signed releases valid past that date, so installed copies are
unaffected, but no new build can be signed until it is renewed.

### Verify before publishing

A macOS release has two layers and both must hold. `tauri-action` notarizes and
staples the `.app`, then wraps it in a DMG it does not notarize; Gatekeeper
assesses a downloaded disk image _before_ anything is mounted, so that DMG is
refused with `Unnotarized Developer ID` however good the app inside it is
(the workflow comment records that v0.5.2 shipped this way). When signing is
configured, the `build` job now submits the image itself — which covers its
contents — and staples the result.

The build then runs the check below and fails if Gatekeeper would refuse the
artifact. Run it again on the downloaded asset before publishing the draft,
because a green workflow is not on its own evidence the
artifact is signed:

```bash
scripts/verify-macos-release.sh ~/Downloads/xFalcon_0.5.2_universal.dmg
```

Given a DMG it checks the image, then mounts it and checks the app: signature
integrity, a Developer ID authority, a secure timestamp, a stapled ticket and a
clean Gatekeeper assessment on both, plus the hardened runtime on the app. It
exits non-zero if any check fails. It needs macOS.

Hardened runtime is asserted only on the app. The flag lives in an executable's
code directory; a disk image is a container and never carries it, so requiring
it of a DMG fails a correct artifact.

### Updater key custody

The minisign private key comes from the `TAURI_SIGNING_PRIVATE_KEY` secret (its
scope is stated in [Signing and secrets](#signing-and-secrets)). Where else a copy lives, and who holds it, is a process fact that no file
in this repository records (unverified). The key is never stored in the
repository.

**Rotation is not retroactive.** A new public key only takes effect for copies
that accept an update _containing_ it. A compromised key cannot be revoked for
already-installed copies that never update
([threat-model.md](threat-model.md) TM-017,
[decisions.md](decisions.md) ADR-042). Custody and channel integrity are the
primary defence.

### Update incident and recovery runbook

1. Keep the affected release as a draft, or unpublish it at once. Do not delete
   evidence until the incident owner records asset digests and workflow logs.
2. Revoke the release workflow token and affected platform credentials. Remove
   the signing secrets (their scope is stated in [Signing and secrets](#signing-and-secrets)).
3. If the updater key is still trusted, correct the defect and publish a higher
   version. Never republish different bytes under an existing version. The
   client intentionally rejects downgrade-based rollback.
4. If an operating-system certificate is compromised, replace it, publish a
   higher version, and follow the Apple or Microsoft certificate-revocation
   process. Keep the updater key unchanged unless it is also at risk.
5. If the updater private key is compromised, stop publishing. Existing clients
   cannot securely learn a replacement key from that channel. Distribute a new
   build through an independently authenticated installer channel, then revoke
   the old release key.
6. Verify the replacement on both supported operating systems. Check the OS
   signature, updater signature, provenance digest, installation, launch, and
   one update from the previous trusted version before publishing the draft.

No record shows that this runbook has been exercised
([compliance.md](compliance.md) CM-014).

---

## The update channel

| Property                  | Value                                                                                              |
| ------------------------- | -------------------------------------------------------------------------------------------------- |
| Endpoint (base config)    | `https://github.com/iSeyon-Analytics/xfalcon-desktop/releases/latest/download/latest.json`         |
| Endpoint (staging config) | `https://d3rdmiksm582v0.cloudfront.net/desktop/channels/staging/latest.json`                       |
| Verification              | minisign, against the public key pinned in `tauri.conf.json` and compiled into `desktop_update.rs` |
| Renderer permissions      | Typed `update_check`, `update_install`, and `update_restart` commands. No updater plugin grant.    |
| UI                        | Settings → Application (`src/features/settings/UpdateSection.tsx`)                                 |

The default capability (`src-tauri/capabilities/default.json`) also grants
`opener:allow-open-url` and `opener:allow-default-urls`, and the core event
listen and unlisten permissions. The macOS-only capability
(`src-tauri/capabilities/macos-window.json`) adds two window permissions for
dragging and maximizing. None of these are update commands.
`src-tauri/src/security/capabilities.rs` lists the plugin permissions.

The endpoint is fixed in configuration and **cannot be chosen or redirected by
the renderer**. After startup and then every 4 hours (`update_schedule.rs`), the
app checks the manifest. A newer version downloads in the background
(`update_install` downloads, verifies and stages it; it does not install), and
Settings then offers _Restart_. `update_restart` re-verifies the staged package,
refuses while work is active, takes the work barrier, then installs and
relaunches. Windows exits when it launches the installer.

The native downloader (`src-tauri/src/desktop_update.rs`) accepts only these
hosts: `github.com`, `objects.githubusercontent.com`,
`release-assets.githubusercontent.com` and `d3rdmiksm582v0.cloudfront.net`. It
limits redirects, the check (30 s), the connection (15 s), the download (15
minutes), the package size (512 MiB) and the release notes (32 Ki characters).
The renderer has no raw updater or process permission. `write_file` and
`write_text_file` remain in the asserted forbidden set.
`security::tests::updater_configuration_uses_https_and_pinned_public_key` fails
the build if the scheme, the key or the staging endpoint changes.

Windows manifests contain separate MSI and NSIS targets, each with its own
signature, so an MSI client cannot fall back to NSIS. An invalid signature, a
missing manifest, or a wrong target **fails closed** and leaves the current
version running. There is no unsigned fallback.

On macOS, restart checks that the bundle and system temporary directory are on
the same volume and that the bundle parent permits replacement before invoking
the updater. Unsupported locations fail with manual-install guidance while the
installed bundle remains untouched. This preflight is not a transactional
rollback guarantee if the filesystem changes during installation.

---

## Platform support

`README.md` lists these targets. No application code enforces the OS version
floors (unverified). The macOS bundle declares `LSMinimumSystemVersion` 13.0 from
`bundle.macOS.minimumSystemVersion`, so macOS refuses to start the app on older
versions. That is not yet verified on a real macOS build.

| Target                                | Status in `README.md`       | Evidence in this repository                                                                                                                   |
| ------------------------------------- | --------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------- |
| Windows 11 23H2+ x64 / ARM64          | supported                   | The release job builds `windows-latest` (x64) and runs the Rust checks there, for the `staging` ref only. No ARM64 job exists                 |
| macOS 13+ Apple Silicon / Intel       | supported                   | The release job builds a universal binary on `macos-latest` and runs the Rust checks there, for the `staging` ref only                        |
| Ubuntu 22.04+, Debian 12+ x64 / ARM64 | claimed, **no CI coverage** | `chat-slice-e2e.yml` (`unix-rust`, ubuntu-22.04) is disabled in Actions. `scripts/gate.sh` runs the Rust suite on a Linux host when run there |

No release job runs on Linux. A headless Linux host has no freedesktop Secret
Service, so vault behaviour on Linux has no evidence. Debian and every ARM64
Linux target have none at all ([compliance.md](compliance.md) CM-024).

Windows installs WebView2 via `downloadBootstrapper`, so a machine without it
fetches the runtime at install time and needs network access then.

---

## Diagnostics

### Where the logs are

```
<app-local-data>/logs/xfalcon-desktop.<YYYY-MM-DD>.log
```

| OS      | `<app-local-data>`                                  |
| ------- | --------------------------------------------------- |
| Windows | `%LOCALAPPDATA%\ai.xfalcon.desktop\`                |
| macOS   | `~/Library/Application Support/ai.xfalcon.desktop/` |
| Linux   | `~/.local/share/ai.xfalcon.desktop/`                |

Daily rotation, 7 files retained. Default level `info`; raise it with
`XFALCON_LOG=xfalcon_desktop=debug`. Logs also go to stderr, so a
terminal-launched build shows them live.

Logs rotate daily and seven files are kept. Each line starts with an
RFC 3339 UTC timestamp with microseconds, for example
`2026-10-13T01:37:27.750151Z  INFO xfalcon_desktop_lib: ...`.

**What logs are meant to contain:** error kinds, identifiers, step outcomes.
**What they are meant never to contain:** tokens, secrets, message text, or
project paths. Redaction runs at write time as well,
but it matches patterns only. Some lines still carry app-owned paths
([security.md](security.md) § Logging and diagnostics, TM-012). A panic writes
location, thread, and backtrace with the payload discarded entirely.

### Collecting a bundle from a user

Settings → Application → Support → **Export diagnostics report**. The backend returns the text and the
renderer saves it through the save dialog; it **never sends anything**, and no
remote sink exists. Every line passes through central redaction for bearer
tokens, `Authorization` headers, `sk-`/`ida_sk_` keys, sensitive query
parameters, sensitive JSON fields, and user home paths. Log files are redacted
at write time as well (see below).

Ask the user to send you the file. Then read
[troubleshooting.md](troubleshooting.md).

### Log redaction at write time

The log writer redacts each line before it reaches the file or stderr. Emails
become `[REDACTED_EMAIL]`, JWT-shaped values become `[REDACTED_JWT]`, and sensitive
headers become `[REDACTED_HEADER]`. Credential names such as `DB_PASSWORD=`, Debug-wrapped
values, sensitive JSON fields and Bearer tokens are redacted too. Existing active-format log files are streamed through the redactor at startup.
Memory is bounded per line; lines over 2 MiB are drained through the next newline and
replaced with a marker. One new log event is capped at 1 MiB; its complete prior lines
are redacted and its discarded tail gets an `…[event truncated]` marker. The known gaps are listed at the top of [troubleshooting.md](troubleshooting.md).
A `[REDACTED_*]` placeholder in a log is expected. It shows that the redaction ran. UUIDs
are not redacted, because support uses them to correlate events.

### What you cannot see

There is **no central audit feed**. The desktop app writes a local,
tamper-evident audit trail on each device. It is not sent anywhere,
and no alerting reads it. Read it only through the diagnostics report. See
[Supporting a customer with the audit trail](#supporting-a-customer-with-the-audit-trail)
and [audit-log.md](audit-log.md). The trail proves edits and mid-log removal.
It does not prove removal of the newest records. Incident reconstruction also
depends on local logs and DA's own server-side records.

There is also no telemetry, no crash reporting service, and no monitoring or
alerting.

## Supporting a customer with the audit trail

Source behavior in 0.5.42. This procedure is local and does not verify an external audit feed.

1. Ask the user for a diagnostics report. Settings → Application → Support → **Export
   diagnostics report**, then save the file. It is written locally and not sent.
2. Open the file and find `## 4. Security Audit Trail`. The sections before it
   are the rest of the report.
3. Read the first line under the heading. `Integrity: verified` means every
   retained record passed its checks. `Integrity: FAILED — <sentence>` names the
   first problem. The sentence table is in [audit-log.md](audit-log.md) §
   "How an auditor or support engineer reads it".
4. Read the flags: `Older segments pruned` is normal after retention. `Torn tail
repaired` means the app cut back a partial last line on open. `Previous log
damaged and set aside` means a damaged log was moved to `audit.damaged-<unix seconds>`.
5. Check `Dropped events this session`. A value above zero means some actions
   were not written. The count resets at each launch.
6. Do not edit, delete or repair files in the `audit` folder. For a failed
   check, ask the user to close the app and copy the `audit` folder and any
   `audit.damaged-*` folders, with the report, to you.

The section lists only the newest 200 events. The older events are in the
segment files. Those files are not part of the report.

If the section says the trail is not available on this device, the OS key store
or the log folder could not be opened. Auditing is off for that session. Check
the application log for the error kind. Do not change the keychain by hand.

---

## Uninstall and local-data removal

The behavior in this section has not been verified on a real Windows,
macOS or Linux machine. Real-system verification is **UNVERIFIED**. Do not tell a
user that uninstall leaves "no residue" until it is verified. Data retention in
detail is in [data-lifecycle.md](data-lifecycle.md).

There are three levels of removal:

| Level                    | How                                                         | Removes                                                                                                                                                                                                                                                                                                      |
| ------------------------ | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| In-app **Reset xFalcon** | Settings → Data & reset → Reset xFalcon, then type `RESET`  | Everything the headless purge removes, for every account on this device, including `audit/` and `logs/`. The account clear runs first, then the purge after the app exits, and the app relaunches. It does not touch the xFalcon server, project folders or exported files. **UNVERIFIED** on real machines. |
| Headless purge           | `xfalcon-desktop --purge-local-data`                        | Everything the app stores: OS credential store entries for both services, then all app folders, including `audit/` and `logs/`.                                                                                                                                                                              |
| Uninstall                | Installer or uninstaller, depending on the platform (below) | The app files. The purge runs as part of uninstall where a hook exists: Windows NSIS on a real uninstall, never for an update (below). MSI does not purge.                                                                                                                                                   |

**The purge command.** It runs before any window, log file or keychain setup. It
deletes the credential store entries first, then the fixed paths. Each path must
be absolute, must not contain `..`, and must end in one of the app's names. If
any path fails that check, nothing is deleted. Symlinks and junctions are removed,
never followed. The command continues past single failures and reports what
remains. It prints counts only, never user paths, to stderr.

| Exit code | Meaning                                                                                                             |
| --------- | ------------------------------------------------------------------------------------------------------------------- |
| `0`       | Clean. Nothing remains.                                                                                             |
| `2`       | Something remains: a path, or the credential store could not be cleared. Close the app, then run the command again. |
| `1`       | Refused, or an error. A target path was unsafe, so nothing was deleted.                                             |

**Windows, NSIS installer.** A real uninstall runs
the purge without a checkbox: from Apps & features, from `uninstall.exe`, or from a
silent `uninstall.exe /S`. The purge is skipped when the silent updater runs the
uninstaller (it passes `/UPDATE`, so `$UpdateMode` is 1). It is also skipped when the
old uninstaller runs in place as a step of "Uninstall before installing", where
`$EXEDIR` equals `$INSTDIR`, unless the person ticks **Delete the application data**
on that run's uninstall page. Ticking the box purges in every case except an update.
A real uninstall runs from a temp copy that NSIS makes, so `$EXEDIR` differs from
`$INSTDIR`. The hook is `src-tauri/windows/hooks.nsh`, set by
`bundle.windows.nsis.installerHooks`. This replaces the earlier rule that only the
box triggered the purge. A non-zero exit writes one line to the uninstall log and
does not stop the uninstall. **UNVERIFIED:** the uninstall, install and upgrade of
xFalcon on a real machine. The `$EXEDIR` behaviour was checked with a throwaway NSIS
installer, and the real installer script with the hook compiled with `makensis`.

**Windows, MSI installer.** The purge does **not** run on MSI uninstall. Run the
purge first, from the installed `xfalcon-desktop.exe`, then uninstall:

```powershell
"<install folder>\xfalcon-desktop.exe" --purge-local-data
```

**macOS.** A drag-installed app has no uninstall hook. Use the script in the
repository, `scripts/uninstall-macos.sh [path-to-xFalcon.app]`. It quits the app,
refuses anything that is not an app bundle with the expected executable and
bundle identifier, runs the purge, then removes the bundle. After a forced stop it waits up to 5 more seconds and warns if the app is still running. The script is not
yet bundled into the `.app`. Manual equivalent:

```bash
"/Applications/xFalcon.app/Contents/MacOS/xfalcon-desktop" --purge-local-data && rm -rf /Applications/xFalcon.app
```

**Linux.** No uninstall hook exists. Run the purge command before you remove the
package.

**Residue the app cannot remove.** OS crash dumps, Windows Prefetch and Recent
files, Spotlight and QuickLook caches, backups, files the user exported to a
location they chose, data inside the user's own projects, and anything held on
the server. Tell the user about these.

## Support runbook

### Reset a user's local state

**Settings → Data & reset → Reset xFalcon.** The implementation exists; behavior is **UNVERIFIED** on real machines.
It is the only in-app erase action. Use it for a complete reset of the device. The
person must type `RESET` exactly. The app closes, a helper runs the full purge after
it exits, and the app relaunches. Everything the headless purge removes goes, for
every account on the device, including `audit/` and `logs/`. The account and data on
the xFalcon server, project folders and exported files are not touched. Helper exit
codes: 0 clean, 1 refused, 2 residue or a credential-store error, 3 the app did not
exit within 120 s (nothing deleted, no relaunch). A second Reset while one is under way
says "A reset is already under way. xFalcon will close and reopen on its own." Wait; do
not start another. The helper runs the purge until clean, for at most about 10 seconds.

The first step is the account clear. It runs per-account handlers: it removes the
session vault entries, the run journal, the local database, local projections, saved
reports, the export folder, project data, decrypted working files, chat model
settings, and the account's skills. Then it removes the account folder **only if it
is empty**. An unknown file leaves the account folder behind. The account clear then
runs a device sweep for loose items. The purge that follows removes what remains,
including `audit/`, `logs/`, `EBWebView/` and `migrations/`.

**It is local only.** It does not delete DA records, and the UI must never imply
that it does. `auth_logout` clears in-memory state and deletes no files.

### Find a setting

Settings has four sections:

- **Appearance**: theme and reading size.
- **Models & connections**: default model, private models, model providers, and MCP
  servers.
- **Agent capabilities**: the model reasoning switch, the next-suggestions switch
  (both moved here from Appearance), and skills.
- **Application**: software updates; Support, with **Export diagnostics report** and
  **Report a security issue**; and Data & reset, with **Reset xFalcon**.

### Security issue reports

Settings → Application → Support → **Report a security issue**. The row opens the
user's mail app with a draft to `support@xfalcon.ai`. The app sends nothing itself.

- Subject: `[URGENT SECURITY] xFalcon Desktop <version> - needs immediate attention`.
- Body: prompts for what happened, where, the steps, the impact, when it started,
  whether it is still happening, who else knows, and how to reach the reporter. It also
  has an automatic block with the app version, platform and UTC time. It asks the
  reporter not to include passwords, API keys, tokens or customer data.
- The draft contains no account email, tenant, file path or token.
- The recipient is the `support@xfalcon.ai` address. The repository does not say who
  reads that mailbox. **UNVERIFIED**: confirm the current owners with the support team
  before relying on this path.
- Whether each OS mail app opens the draft with its line breaks intact is
  **UNVERIFIED**. If it does not, the reporter can copy the address and the text by hand.

### Idle sign-out (explaining it to a user)

If a user says they were signed out, the likely cause is the idle sign-out.
xFalcon signs a person out after 12 hours without input in the window: a mouse move,
click, key press, scroll or touch. The sign-in screen then shows "Signed out after 12
hours of inactivity. Sign in to continue." Signing in again clears the notice.

- A running chat, or an edit or regenerate in progress, postpones the sign-out.
- An open dialog postpones it, but only until 24 hours after the last input.
- If the sign-out fails, the user sees "Could not sign you out after 12 hours of
  inactivity. Close xFalcon or sign out from the account menu." xFalcon retries after
  5 minutes.
- The limit is fixed in the build. There is no setting for it. The sign-out deletes no
  local data.
- The timer uses the computer's clock. A laptop that slept signs out at the first check
  after it wakes. See [runtime-flows.md](runtime-flows.md#inactivity-sign-out).

### Rotate a provider key

Settings → Models & connections → Model providers → replace. Use _replace_, not
delete-and-add: re-adding under the same label collides with the workspace
uniqueness rule, and under a new label the old unusable row would keep the
default flag.

### A user's MCP server stopped working

1. Settings → Models & connections → MCP servers → **Test**. It dials now and
   reports what that machine saw, storing nothing — so a laptop behind a captive
   portal cannot mark a healthy server broken for the whole tenant.
2. If the row shows an unavailable reason, read it — it is precise. See
   [agent-runtime-reference.md](agent-runtime-reference.md) § MCP.
3. `NeedsSignIn` is recoverable in place: sign in from the same sheet.
4. `UnsupportedTransport` means the row is a `stdio` or unknown-transport server.
   It cannot work here ([decisions.md](decisions.md) ADR-009).
5. A server on a private, loopback or local address is allowed while
   `ENFORCE_PUBLIC_MCP_HOSTS` is `false` in `src-tauri/src/mcp/registry.rs`. `InvalidUrl`
   means the URL shape was refused: userinfo, a query or fragment on the endpoint, no
   host, a non-web scheme, or `http://` to a public-looking name. See
   [troubleshooting.md](troubleshooting.md#mcp).

### Restore a file the agent changed

Project explorer → the file → History & Revisions → restore. Every agent edit,
file-viewer save and restore captures a revision first, up to 50 per file and
512 MiB per account ([decisions.md](decisions.md) ADR-043).

### A chat is stuck

Cancel the run. If a run is stuck in a non-terminal state after a crash, restart
the app — cold-start recovery reads the run journal and marks anything unsafe to
resume as `Interrupted` so a retry cannot duplicate output.

---

## Repository governance

`.github/CODEOWNERS` names reviewers for the security-critical paths. It has 35
path patterns. They cover `security/`, `capabilities/`, the Tauri configuration,
`src-tauri/src/lib.rs`, `src-tauri/src/app/commands.rs`, `src-tauri/src/path_grants.rs`,
`src-tauri/windows/`, auth, credentials and persistence, the MCP registry and DNS code, custom model
endpoints, projects, tools, approvals and the agent, audit and diagnostics, the
updater, `.github/`, `scripts/`, the dependency policy and lockfiles, and the
security documents. The file has no catch-all `*` rule. Its owners are the three
repository admins, by user name. IT should replace them with a team.

**The file does nothing by itself.** It takes effect only when the branch
protection rule on `staging` (and `main`) enables **Require review from Code
Owners**. IT owns those GitHub settings. Engineering must not change them.

IT action items. Settings were checked read-only through the GitHub API on
2026-10-15. This is an observation, not a claim about future settings. The
repository is private.

| Setting                             | Observed 2026-10-15                                         | IT action                                                                                   |
| ----------------------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Branch protection on `staging`      | None; rulesets list is empty                                | Require pull requests and reviews, require review from Code Owners, and block force pushes. |
| Branch protection on `main`         | Not checked (UNVERIFIED)                                    | Check it, then apply the required rules.                                                    |
| `desktop-release` environment       | No protection reviewers; administrators can bypass          | Add required reviewers and restrict branch access before a production release.              |
| `desktop-staging` environment       | Branch policy only; no reviewers; administrators can bypass | Add required reviewers and review bypass policy.                                            |
| CODEOWNERS owners                   | Three individual admins                                     | Replace them with a team that has write access.                                             |
| Secret scanning and push protection | Disabled                                                    | Enable both.                                                                                |
| Dependabot security updates         | Disabled                                                    | Enable.                                                                                     |
| Code scanning                       | Disabled                                                    | Enable, or record why it is not used.                                                       |

`ci.yml` and `chat-slice-e2e.yml` are manually disabled in GitHub Actions (see
[Pipelines](#pipelines)). The local gate is the per-change automated check available
in the repository. Its latest recorded run passed on Windows on 2026-10-16.
The release workflow is a manual staging dispatch and does not replace per-change
checks.

## Operational gaps

Know these before you commit to an SLA.

| Gap                                                               | Consequence                                                                                                                                                                                                                                                                                                                                                     |
| ----------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Audit trail is per device                                         | The local audit log is not sent to a central store. Nothing alerts on it.                                                                                                                                                                                                                                                                                       |
| No monitoring, alerting, or telemetry                             | You learn about failures from users.                                                                                                                                                                                                                                                                                                                            |
| No binary-level test                                              | A release can break the React↔Rust seam and pass every check.                                                                                                                                                                                                                                                                                                   |
| No per-change native-platform run                                 | `ci.yml` is disabled. The release job runs the Rust checks natively, but only on release dispatch and not the frontend suite.                                                                                                                                                                                                                                   |
| Windows signing is optional                                       | A Windows release can finish green and ship unsigned. Updater signing is mandatory on both platforms; Apple signing and notarization are mandatory on macOS.                                                                                                                                                                                                    |
| No packaged signature dry run                                     | Workflow configuration can pass while the produced installer fails an OS trust check.                                                                                                                                                                                                                                                                           |
| No release job on Linux                                           | No Debian, ARM64, or headless vault evidence.                                                                                                                                                                                                                                                                                                                   |
| `cargo deny` is not in the release job                            | Dependency policy runs only in `scripts/gate.sh`.                                                                                                                                                                                                                                                                                                               |
| `--all-features` is not used anywhere                             | Feature-gated code (`custom-protocol`) can rot silently.                                                                                                                                                                                                                                                                                                        |
| Uninstall and purge are UNVERIFIED on real systems                | No recorded run of the NSIS hook (real uninstall, in-place run, checkbox and update rules), the credential store purge, Reset xFalcon, or `scripts/uninstall-macos.sh`.                                                                                                                                                                                         |
| MSI and macOS uninstall need a manual purge                       | Users who skip the step keep the local data and the keychain entries.                                                                                                                                                                                                                                                                                           |
| Private MCP addresses and plain `http` presigned URLs are allowed | `ENFORCE_PUBLIC_MCP_HOSTS` and `ENFORCE_HTTPS_PRESIGNED_URLS` are both `false` (`src-tauri/src/mcp/registry.rs`, `src-tauri/src/uploads/da_repository.rs`). A registration can point the app's MCP and OAuth requests at the person's own network, and plain `http` presigned URLs go to any host. Set both to `true` and rebuild to restore the earlier rules. |

Tracked with owners in [compliance.md](compliance.md) § open gaps.
