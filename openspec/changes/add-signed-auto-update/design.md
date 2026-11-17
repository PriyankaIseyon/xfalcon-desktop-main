## Context

xFalcon uses Tauri v2 on Windows and macOS. Tauri verifies updater artifacts
with an embedded Ed25519 public key. The operating system separately verifies
the installed application publisher.

The renderer has no direct updater or process permission. Typed native commands
own discovery, download, signature verification, installation, and restart.

## Desktop update research baseline — 2026-08-31

This review covers ten widely deployed desktop applications. It is a design
sample, not a popularity ranking. Only first-party product and engineering
documentation is used.

| Application                                                                                                      | Current delivery behavior                                                                                                                                                                                                                            | Design signal for xFalcon                                                                                                           |
| ---------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| [Google Chrome](https://support.google.com/chrome/answer/95414?co=GENIE.Platform%3DDesktop&hl=en)                | Downloads in the background, applies on relaunch, and restores normal windows and tabs. Enterprise policy supports silent, manual, pinned, and suppressed update periods.                                                                            | Keep discovery non-disruptive. Treat restart and managed policy as separate controls.                                               |
| [Microsoft Edge](https://learn.microsoft.com/en-us/deployedge/microsoft-edge-update-progressive-rollout)         | Uses progressive stable rollouts over several days and supports enterprise channels, version targets, and temporary rollback.                                                                                                                        | Add staged rollout and rollback only with server-side health evidence and explicit policy.                                          |
| [Mozilla Firefox](https://firefox-source-docs.mozilla.org/toolkit/mozapps/update/docs/InAppUpdateProcess.html)   | Resumes downloads, stages by default, applies after restart, and falls back from a failed partial MAR to a complete MAR. [MAR files](https://firefox-source-docs.mozilla.org/toolkit/mozapps/update/docs/MarFiles.html) are signed and size-bounded. | Durable staging, delta fallback, and package verification are later reliability layers. They must not weaken full-package recovery. |
| [Visual Studio Code](https://code.visualstudio.com/docs/setup/windows#_updates)                                  | Recommends a per-user installer because it avoids elevation and gives smoother background updates. The app prompts before installation.                                                                                                              | Prefer the per-user NSIS artifact on Windows and keep the install action clear.                                                     |
| [Slack](https://slack.com/help/articles/360048367814-Update-the-Slack-desktop-app)                               | Direct installs expose check, badge, and restart-to-apply actions. Store and managed installs delegate update control to the store or administrator.                                                                                                 | Keep availability visible without forcing work interruption. Preserve a future managed-distribution boundary.                       |
| [Zoom Workplace](https://support.zoom.com/hc/en/article?id=zm_kb&sysparm_article=KB0063814)                      | Standard packages enable automatic updates and offer slow or fast channels. Managed packages can disable client updates. Security releases can bypass progressive rollout.                                                                           | Channels and critical-update policy need explicit product and administrator rules. They are not renderer flags.                     |
| [Signal Desktop](https://support.signal.org/hc/en-us/articles/360007059212-How-do-I-ensure-Signal-is-up-to-date) | Checks at startup and periodically. It shows a banner and lets the user download or restart. Linux delegates delivery to APT.                                                                                                                        | The current startup plus periodic discovery and explicit restart model is familiar and low surprise.                                |
| [1Password](https://support.1password.com/update-1password/)                                                     | Checks weekly. It can install automatically while locked, but notifies while unlocked. Managed and package-manager installs can delegate control.                                                                                                    | Work-aware installation is stronger than a timer alone. A later idle or locked policy must remain native.                           |
| [Microsoft Teams](https://learn.microsoft.com/en-us/microsoftteams/teams-client-update)                          | Checks at startup and every few hours, stages delta updates, rolls out gradually, applies while idle, and requires restart. macOS prepares a clone and swaps it after quit.                                                                          | Separate discovery, download, staging, and activation. Use real idle state before any automatic activation.                         |
| [Dropbox](https://help.dropbox.com/installs/update-mac)                                                          | Uses a background updater, downloads without interrupting work, and waits until the user finishes a task before restart. Stable releases roll out gradually.                                                                                         | A future background service must share the same native work barrier and must never restart active work.                             |

The common mature pattern is a privileged update owner, silent discovery,
cryptographic and operating-system trust, staged or user-timed activation,
gradual rollout, and a managed-install escape hatch. Delta delivery, durable
staging, rollback, critical-update policy, and telemetry require server and
packaged-client evidence. This change lays the trust, native command, work
barrier, deterministic publication, and second-source seams without claiming
those later controls.

## Goals / Non-Goals

### Goals

- Signed discovery and installation from a compile-time endpoint.
- Native ownership of the periodic schedule.
- No automatic installation or work interruption.
- Fail-closed release signing and deterministic manifest publication.
- A source strategy that can add a CDN without adding a trust root.

### Non-Goals

- Durable or resumable background staging.
- Apply-on-quit.
- Mandatory-update policy or renderer-defined manifest fields.
- MDM policy, session serialization, rollback, or crash-loop recovery.

## Decisions

### Native cadence and update boundary

The Rust runtime emits `xfalcon://update-check-requested` every four hours. The
renderer subscribes after secure bootstrap and requests one immediate startup
check through the typed native command. The renderer owns no long-lived timer.

Background checks record the time and success outcome. Network errors remain
silent. A manual check displays the same error inline.

### Explicit install only

Discovery stores the native Tauri `Update` value. It does not expose a resource
identifier to the renderer. The Settings action calls `update_install`. A native
admission barrier excludes new runs and transfers, then checks native work state
before any package byte is downloaded.

Tauri performs full SemVer ordering in native code. The renderer cannot provide
comparison options or enable downgrades. xFalcon does not maintain a second
incomplete SemVer implementation.

Decline and successful installation remove the pending native value. A failed
installation retains it for retry. Installation progress exposes bytes and
percentage when the server supplies a content length.

### Trust and delivery sources

`tauri.conf.json` pins the GitHub Releases manifest endpoint and Ed25519 public
key. The webview cannot provide an endpoint or artifact path. The native client
allows reviewed HTTPS delivery hosts only. It limits redirects, connection and
total time, release-note size, and package size. It verifies the package before
installer execution.

GitHub Actions builds and signs the release. GitHub Releases is the active
delivery source. A future CDN will be an ordered, compile-time endpoint that
serves the identical manifest and signed artifacts. It will not provide a new
key or accept renderer configuration.

### Release publication

The release workflow requires all signing material before packaging. macOS
uses Developer ID signing and Apple notarization. Windows imports a scoped PFX
certificate and configures SHA-256 Authenticode with an RFC 3161 timestamp.

Workflow runs serialize at the release level. Platform jobs can build in
parallel because they have read-only repository access and cannot publish.
After both jobs pass, one publisher validates all signed assets and writes one
deterministic `latest.json`. It prefers NSIS for Windows updater metadata. It
fails if the release tag already exists, so a retry cannot replace published
bytes. Release tags must equal the bundled application version. Release actions
use immutable commits. A protected environment gates signing and publication.
The workflow generates a CycloneDX SBOM and creates signed build provenance
before uploading workflow artifacts. A Linux verification job runs the full
frontend and Rust source gates on the exact release commit before either
signing job starts.

## Risks / Trade-offs

- The app does not pre-download updates. This avoids hidden process exits and
  memory-only pseudo-staging, but installation takes longer after user action.
- The native package path is bounded at 512 MiB. Tauri's installer API consumes
  verified bytes, so the process still holds at most that bounded package in
  memory during installation.
- A future CDN must pass availability, cache invalidation, and provenance tests
  before its endpoint is compiled into the app.
