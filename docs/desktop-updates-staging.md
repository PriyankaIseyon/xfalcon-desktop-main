# Desktop updates in staging

Reviewed 2026-10-15 against `infrastructure/desktop-updates-staging.json`,
`scripts/publish-desktop-staging.mjs`, `scripts/prepare-desktop-release.mjs`,
`.github/workflows/desktop-build.yml`, `desktop-promote-staging.yml` and
`src-tauri/src/desktop_update.rs`. Account: `572186105468`. Region: `us-east-1`.
Bucket: `xfalcon-desktop-artifacts-staging`.

The source baseline is 0.5.50; see [its release notes](releases/v0.5.50.md).
A recorded workflow dispatch is not evidence of completed delivery. No live channel
status is asserted here.

## Delivery and trust

`infrastructure/desktop-updates-staging.json` manages CloudFront, origin access
control and the GitHub publisher role. The separate
`scripts/configure-desktop-update-origin.mjs` deployment step merges the delivery
rule into the pre-existing bucket policy. The existing
bucket retains public-access blocking, encryption, versioning, and disabled ACLs.
CloudFront can read release objects and the staging pointer; candidates remain
private. The template references the existing GitHub OIDC provider.

The publisher role is restricted to
`repo:iSeyon-Analytics/xfalcon-desktop:environment:desktop-staging`.
That GitHub environment must allow deployment from the `staging` branch only.
The repository variable `DESKTOP_UPDATE_BASE_URL` holds the CloudFront HTTPS
origin without a trailing slash. No AWS credentials belong in app configuration.

The staging workflow merges `src-tauri/tauri.staging.conf.json` at build time.
`npm run dev:app` applies the same configuration when the local checkout is on
`staging`. A copied or detached working tree may not carry that branch metadata,
so select the staging overlay explicitly for a direct native build. In an
existing prepared native build tree, set the overlay and run Cargo in the same
PowerShell session:

```powershell
$env:TAURI_CONFIG = Get-Content .\src-tauri\tauri.staging.conf.json -Raw
cargo run --manifest-path .\src-tauri\Cargo.toml --features custom-protocol --locked
```

This example assumes the tree already has its native toolchain and frontend
`dist`. A copied or detached tree still needs explicit overlay selection. Keep
tenant credentials and dotenv files out of private build snapshots.

A build without the overlay keeps the base updater configuration, which points at
GitHub releases. `desktop-build.yml` runs only for the `staging` ref (a build
packages the tenant's MCP OAuth client in plain text, accepted for staging only),
so a release build always carries the staging override. Release tags are
`v<version>` and are shared at the repository level, so an existing version cannot
be rebuilt under the same tag.

The staging distribution serves `https://d3rdmiksm582v0.cloudfront.net`; the
hostname is compiled into `tauri.staging.conf.json` and the native allowlist.
The distribution id (`EMR9LKA7USSS9`) is recorded here only; no file in the
repository holds it (unverified). Delivery is enabled for downloads by URL. S3 and
candidate files remain private.

## Release sequence

1. Set the next application version consistently in the repository and run its
   quality gates. The first build with this updater needs manual installation
   for clients that still use the private GitHub endpoint.
2. Run `desktop-build` on staging with the intended staging tenant configuration.
   Updater signatures are mandatory on both platforms. Apple signing and
   notarization are mandatory on macOS; Windows Authenticode is optional. The
   `publish` job creates the GitHub draft release. The `stage-artifacts` job then
   uploads immutable candidate files and writes their index last.
3. The same job verifies signatures and the actual CloudFront delivery route,
   then conditionally updates `desktop/channels/staging/latest.json`.
4. `desktop-promote-staging` remains available on staging to retry an interrupted
   promotion with the candidate version (without `v`). An exact-byte retry of
   an already advertised version succeeds without rewriting the pointer.
5. After successful delivery, both workflows run `cleanup-releases`. It keeps
   five GitHub release entries, including drafts, ordered by release creation
   time. If the current published latest release is older, it occupies one of
   those five slots so the GitHub latest-download endpoint remains available.
   Failed builds or delivery skip cleanup. Cleanup requires the delivered tag
   to remain in the retained set and fails visibly on listing or deletion errors.
   Both cleanup jobs share a concurrency group so a build and recovery cannot
   race to delete the same release.

GitHub cleanup deletes old release entries and their attached assets. Git tags,
source history, and the separate S3 candidate/release objects are preserved.
The script lists every release page and validates the complete result before
sending any deletion. `npm run test:release` includes its HTTP integration tests;
the cleanup jobs also run those tests before using the repository token.
To inspect the planned retention without deletions, provide `GH_TOKEN` and run:

```bash
GITHUB_REPOSITORY=iSeyon-Analytics/xfalcon-desktop \
  node scripts/cleanup-desktop-releases.mjs v0.5.29 --dry-run
```

Objects are arranged as:

```text
desktop/candidates/<version>/...       # private build output and index
desktop/releases/<version>/...         # immutable delivered artifacts
desktop/channels/staging/latest.json   # mutable channel pointer, published last
```

Re-running candidate upload with identical bytes is safe. Conflicting bytes fail.
Promotion must advance the version or exactly match the advertised manifest;
concurrent pointer modification fails its
conditional write. A failed delivery check never advertises the new version.
Release files are cacheable for a year; the pointer requires revalidation.
This channel is for the staging tenant only. Do not publish another tenant's
bundle to it.

The workflow authenticates to AWS with OIDC (`aws-actions/configure-aws-credentials`
and the publisher role) and reads no long-lived AWS key. Whether long-lived keys
also sit in the `desktop-staging` environment is not recorded in the repository
(unverified).

## App behavior

After secure startup and every four hours (`update_schedule.rs`), the native
updater compares the
manifest against the running package version using Tauri's SemVer handling.
It offers an in-app notice on all workspaces; this is periodic discovery, not
an operating-system push service. Settings provides the same controls.

When an update is downloading or ready to restart, Settings renders the selected
release notes through `ReleaseNotesMarkdown` as Markdown, including headings, lists,
links, tables and code formatting. The reader does not expose assistant preview
actions. Source and browser-test coverage do not establish a remotely delivered
release or a successful live update.

Newer versions download and stage automatically without blocking chats or
transfers. No Download click or browser visit is required. Failed downloads can
be retried immediately or on the next scheduled check.
Every check reads the manifest, including while an update is staged. A newer
release replaces the staged package, and Restart to update reads the manifest
once more first. One restart therefore installs the latest release and never an
older one before it. If the manifest cannot be reached at that moment, the
verified staged package is applied.
Restart to update verifies the cache again, checks native active work, and
applies under the work-admission barrier. Windows exits when launching its
installer; macOS relaunches after bundle replacement. OS elevation prompts may
still appear depending on the installation location. A staged cache is restored
after a successful fresh manifest check; offline startup waits for discovery.

The app retains the embedded updater signing key and checks HTTPS delivery hosts,
redirects, timeouts, package size, and package signatures. Do not replace its
trust key when changing delivery. Windows Authenticode and Apple notarization
remain distinct from mandatory updater signatures.

## Infrastructure deployment

Use `aws cloudformation deploy` with the checked-in template, stack name
`xfalcon-desktop-updates-staging`, and `CAPABILITY_NAMED_IAM` in the verified
staging account with `--parameter-overrides DeliveryEnabled=true` for the approved
downloads by URL. After the stack completes, run:

```bash
AWS_PROFILE=staging node scripts/configure-desktop-update-origin.mjs
```

This preserves the existing bucket policy and adds only the distribution-scoped
release read rule. If a replacement distribution changes its hostname, update
the compiled staging endpoint, native allowlist, and GitHub variable together;
do not publish a manifest that points to the previous distribution.

## Operations and verification

Verify the account before infrastructure writes:

```bash
aws sts get-caller-identity --profile staging
aws cloudformation validate-template \
  --template-body file://infrastructure/desktop-updates-staging.json \
  --region us-east-1 --profile staging
```

Local publisher use requires `AWS_PROFILE=staging` and
`DESKTOP_UPDATE_BASE_URL`. The script independently checks the account ID.
Do not manually overwrite versioned objects or the channel pointer.

Test discovery with a same-version manifest (no notice), a newer signed candidate
(automatic download and Restart notice), offline failure, corrupted cache, active work during download,
and restart refusal during active work. A real packaged upgrade between distinct
versions on both platforms is required before claiming installation end to end.
Starting an extracted package payload does not prove installer behavior.

### Local package check (2026-10-15)

Both unsigned Windows bundles built successfully from the verified staging
snapshot, and the extracted NSIS payload started with the staging tenant
configuration. Neither installer was executed, no updater artifact was created,
and nothing was published to the staging channel. This is local build and
startup evidence only; it does not verify installer lifecycle, signed delivery,
an update, or a live tenant. The detailed package hashes and limits are in the
[assurance evidence](assurance-plan.md#staging-validation-evidence-2026-10-15). The
release-notes browser suite has 8 tests; this is UI evidence, not channel-delivery evidence.

Withdrawing a bad channel does not downgrade installations already updated.
Prepare a corrected, higher-version release. Bucket version history is recovery
evidence, not an automatic rollback system.
