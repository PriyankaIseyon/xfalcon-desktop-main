# Change: Add Signed Desktop Auto-Updates

## Why

xFalcon Desktop needs a safe path from a published release to an installed
binary. The existing updater wiring did not provide background discovery,
work-aware installation, mandatory operating-system signing, or release
manifest publication controls.

## What Changes

- Add a native startup and four-hour discovery cadence.
- Keep background checks silent and non-blocking.
- Require an explicit user action before download and installation.
- Refuse installation and relaunch while a model run or transfer is active.
- Pin the update endpoint and Ed25519 public key in the application bundle.
- Require updater signing, Apple signing and notarization, and Windows
  Authenticode signing before release publication.
- Separate platform signing from publication, then publish one manifest that
  prefers the per-user NSIS artifact.
- Keep GitHub Releases as the active delivery source. Reserve a second ordered
  endpoint for a future CDN that serves the same signed artifacts.

## Impact

- Affected spec: `desktop-update` (new capability)
- Affected code: update settings store and UI, native scheduling, security
  tests, Tauri configuration, release workflow, and threat model
- Generated IPC contracts add typed update results and progress payloads.

## Deferred Work

Durable background staging, resumable download, apply-on-quit, managed policy,
critical-update policy, session handoff, and crash-loop recovery need separate
native designs and packaged-platform test harnesses. This change does not claim
those capabilities.
