# Design

Account 572186105468, us-east-1, bucket xfalcon-desktop-artifacts-staging.
S3 remains private, encrypted, versioned, and HTTPS-only. CloudFront OAC is
restricted to this distribution and delivery keys. Candidate uploads are not
readable by viewers. GitHub OIDC is scoped to this repository's staging-only
desktop-staging environment. No long-lived credentials ship in the app or CI.

The staging channel manifest is published last with a conditional write.
Release files use immutable versioned keys. Promotion rejects downgrades and
conflicting bytes. GitHub draft creation remains separate from staging promotion.
The application retains its pinned updater signing key.

The existing update_install command now stages verified bytes; update_restart
applies them under the native work barrier. The additive is_staged check result
restores readiness after a fresh verified discovery. Downloads do not hold the
work barrier. The package is reverified before apply. Windows installer launch
exits the process; macOS relaunch follows successful apply.

Current repository and GitHub draft version is 0.5.4. A same-version manifest
must report no update. Older clients pinned to the private GitHub endpoint need
one manual installation of a build containing the new endpoint. No artificial
version bump or unsigned test installer will be advertised.

This change supersedes the download/apply and delivery-source decisions in
`add-signed-auto-update`. Existing optional platform-signing behavior is retained;
mandatory updater signatures remain enforced.

On 2026-09-07 the user approved downloads by CloudFront URL and automatic
download/verification. Applying the staged update still waits for explicit
restart.
