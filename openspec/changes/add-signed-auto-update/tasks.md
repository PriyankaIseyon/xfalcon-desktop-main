## 1. Release Trust

- [x] 1.1 Pin the Ed25519 public key and HTTPS GitHub Releases endpoint.
- [x] 1.2 Fail release builds when updater signing material is absent.
- [x] 1.3 Require Apple signing and notarization material on macOS.
- [x] 1.4 Import and validate the Windows Authenticode certificate.
- [x] 1.5 Prefer NSIS and publish one immutable manifest after both platform builds pass.
- [x] 1.6 Pin release actions to immutable commits and validate the release version.
- [x] 1.7 Generate a CycloneDX SBOM and signed build provenance.
- [x] 1.8 Gate signing on the full source checks for the exact release commit.

## 2. Safe Update Lifecycle

- [x] 2.1 Keep raw updater and process plugins outside the renderer capability; expose typed native update commands.
- [x] 2.2 Run startup discovery after secure application bootstrap.
- [x] 2.3 Move the four-hour cadence to the native runtime.
- [x] 2.4 Keep silent checks free of installation and user-visible errors.
- [x] 2.5 Require explicit installation and use native admission to block active and newly starting work.
- [x] 2.6 Track check outcome, install progress, and restart failure.
- [x] 2.7 Release declined and completed native update state; retain failed installs for retry.
- [x] 2.8 Use Tauri's full SemVer guard with downgrades disabled.

## 3. Verification

- [x] 3.1 Test silent discovery without installer execution.
- [x] 3.2 Test installation and relaunch guards during active work.
- [x] 3.3 Test progress, resource cleanup, and restart recovery UI.
- [x] 3.4 Test the native four-hour cadence constant.
- [x] 3.5 Assert updater endpoint, public key, delivery-host policy, and privilege boundaries.
