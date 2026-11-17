# Dependency review — 2026-11-04

Scope: `staging`, Windows and macOS. This source update does not create a release.
The existing v0.5.51 packages were built from `fe0f0a6` before these changes.

## Resolved dependencies

| Dependency | Before | After | Scope |
| --- | --- | --- | --- |
| Vite | 6.4.3 | 6.4.5 | Development server and build |
| Vitest / `@vitest/mocker` | 4.1.11 | 4.1.12 | Tests |
| Zod | 4.4.3 | 4.6.7 | Development tools |
| Undici | 7.30.0 | 7.30.1 | Test DOM |
| minimatch | 10.2.6 | 11.0.4 | Lint configuration and TypeScript parsing |
| Direct ZIP reader and calamine's ZIP reader | 8.6.0 | 9.0.5 | Imported skills, Office files and datasets |
| calamine | 0.36.1 | 0.38.0 | Spreadsheet parsing |
| Tokio | 1.52.3 | 1.53.3 | Native asynchronous runtime |
| rustls-webpki | 0.103.15 | 0.103.16 | TLS certificate verification |
| webpki-root-certs | 1.0.8 | 1.0.10 | TLS root certificates |
| crypto-bigint / rsa | 0.5.5 / 0.9.10 | Removed | JWT crypto provider |

The minimatch override applies only to its three affected parents: `eslint`,
`@eslint/config-array`, and `@typescript-eslint/typescript-estree`. Their declared
10.x ranges cannot select the patched 11.x release. Zod and Undici need only
lockfile updates within their existing ranges.

`jsonwebtoken` remains at 10.4.0 and uses its supported `aws_lc_rs` provider,
already present through rustls, instead of `rust_crypto`. This removes the
affected crypto-bigint graph and the RSA crate's old advisory exception. The
allowed JWT algorithms, issuer, audience, tenant and claim checks remain in
`auth/token.rs`. The provider accepts RSA keys from 2048 through 8192 bits. The
public signing keys discovered at both staging authentication hosts were
4096 bits when checked. A WireMock JWKS regression covers all six allowed RS/PS
algorithms through the real token verifier.

The four Dependabot rustls alerts already had their fix, rustls 0.23.46, in
`staging` before this update. That version remains unchanged.

Comparing all 22 open alert entries with the final resolved dependency versions
leaves only the Linux glib entry affected. This describes the `staging` source,
not the default branch or installed packages.

## Updater archive support

The audited `tauri-plugin-updater` pin stays at 2.10.1. Its optional legacy ZIP
installer feature is disabled; `rustls-tls` remains enabled explicitly. This
removes `zip 4.6.1` from the lockfile, including the critical parser differential,
extraction denial of service and AES authentication findings. All ZIP-related
advisory exceptions were removed from `deny.toml`.

Windows delivery uses signed raw EXE/MSI files. macOS still installs the signed
`app.tar.gz`: the plugin's macOS tar and flate2 dependencies and installer are
unconditional and do not need its ZIP feature. The release preparer accepts
these three formats and rejects legacy ZIP packages before writing output.
The existing 0.5.50 and 0.5.51 staging payloads already use the supported formats.
Pinned-key signature verification, bounded downloads, staging and verification
before installation are unchanged.

## Remaining Linux dependencies

The open glib alert belongs to Tauri's Linux GTK dependency. The additional
RustSec unmaintained notices for `zbus_names`, `zvariant` and `zvariant_derive`
also belong to the Linux graph. Linux changes are outside this task's scope.

Engineering must reassess the Linux dependency notices by 2026-11-11 and before
enabling a Linux release. No new advisory exception or alert dismissal was added.

GitHub [scans the default branch](https://docs.github.com/en/code-security/concepts/supply-chain-security/dependabot-alerts),
which is `main` in this repository. A push
to `staging` does not establish that the dashboard alerts have closed. This
task does not modify `main` or dismiss alerts.

## Verification

- npm 10.9.8 clean install with scripts disabled passed. Full and production
  audits report zero vulnerabilities. Registry verification passed for 393
  package signatures and 128 attestations.
- The frontend gate passed all 2,832 tests, type checks, lint, formatting, font,
  style, tooltip and UX ledger checks. The frontend build passed.
- All 70 release tests passed after the format change, including rejection of
  each legacy ZIP package before output creation. The actual v0.5.51 updater
  artifacts reproduced the existing seven-platform staging manifest exactly
  with the revised preparer, without publishing anything.
- Locked dependency-policy checks passed for Windows x64 and both macOS
  architectures: advisories, bans, licenses and sources. The unfiltered check
  still reports the three Linux-only unmaintained notices listed above.
- Native Windows formatting, strict Clippy and all 3,249 workspace tests passed
  on a private source snapshot (32 ignored), including the six-algorithm JWT
  regression. The snapshot excluded the 18 unrelated untracked permission
  artifacts. Its code and manifests matched the canonical checkout.
- The combined Rust runner's unfiltered dependency-policy step crashed once.
  A direct rerun from the same snapshot reported only the three Linux
  unmaintained notices. The full unfiltered gate is not recorded as passing.
- No native macOS test or packaged update installation was run for
  this dependency change. The earlier v0.5.51 release workflow tested the
  preceding source commit and is not validation of these changes.
