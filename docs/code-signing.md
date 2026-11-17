# Code signing and release integrity

Version 0.5.50 · reviewed 2026-10-28 · **Understanding + How-to** · audience: release engineers, compliance owners

Which signatures the desktop release carries, what each one protects, what is
signed today on each platform, and what remains to be done. The last part is
the plan for Windows signing and for making the release fail closed when a
signing secret is missing. Scope is the desktop app only.

> Status: the v0.5.37 signing evidence below is historical. This document describes
> the v0.5.50 source; the local unsigned v0.5.41 package evidence is recorded in the
> assurance plan and is not release evidence. It is not a certification,
> an attestation or an audit opinion. Anything marked UNVERIFIED has not been
> checked against the vendor, the platform or a real machine. Confirm it before
> relying on it or buying anything.

## The four kinds of signature and attestation

Each of these answers a different question. None of them replaces another.

### 1. Updater signature (minisign)

- **What it is.** The release signs each updater package with a minisign private
  key. The public half is pinned in `src-tauri/tauri.conf.json` (the `pubkey`
  field). The app checks the signature before it installs an update.
- **What it protects.** An update is installed only when it is signed with the
  pinned key. A channel that serves other bytes is refused.
- **What it does not protect.** A stolen key can sign a malicious update. A new
  public key only reaches copies that accept an update containing it. Rotation is
  not retroactive. See [decisions.md](decisions.md) ADR-042 and TM-017 in
  [threat-model.md](threat-model.md).
- **Where it is enforced.** The release job fails when `TAURI_SIGNING_PRIVATE_KEY`
  is missing. This is the only signing secret that is mandatory today. Custody
  is described in [operations.md](operations.md#updater-key-custody).

### 2. Platform code signing

- **macOS.** A Developer ID Application certificate signs the `.app`. The
  signature is made with the hardened runtime on. Gatekeeper checks this
  signature when an app is first opened.
- **Windows.** Authenticode signing. Not configured today. See
  [Windows: what must be done](#windows-what-must-be-done).
- **What it protects.** It names the publisher to the operating system. It
  detects changes to the signed file. Gatekeeper and SmartScreen use it for
  their decisions.
- **What it does not protect.** A valid signature does not show that the code is
  safe. It does not replace the updater signature. The two check different things.

### 3. macOS notarization and stapling

- **What it is.** The workflow sends the disk image to Apple's notary service.
  Apple scans it and returns an accepted or rejected status. The workflow then
  staples the ticket to the file.
- **What it protects.** A machine that is offline at first launch can still
  accept the app, because the ticket is stapled to it.
- **Where it runs.** The step "Notarize and staple the macOS disk image" in
  `.github/workflows/desktop-build.yml`. It submits the DMG. The DMG covers the
  app inside it.

### 4. Build provenance and SBOM

- **Provenance.** `actions/attest-build-provenance` (pinned to a commit SHA)
  records which workflow run produced each file. The subjects are the files that
  match the workflow's `subject-path` globs: DMG, EXE, MSI, `.tar.gz`, `.zip` and
  the SBOM.
- **SBOM.** `anchore/sbom-action` (pinned) writes a CycloneDX JSON file for each
  platform.
- **What they protect.** They let a reviewer tie a file to a run and list its
  components. They are not a publisher signature.
- **Limit.** The provenance step is marked `continue-on-error: true`. The
  workflow comment says attestation storage depends on the GitHub plan. In the
  v0.5.37 run it succeeded. A future failure would not stop the release.

## Current state

The table covers the v0.5.37 run, 38360714825, on 2026-10-13. The macOS and
Windows job logs were read with `gh run view 38360714825 --log`. Anyone with
access to the repository can reproduce the reading.

| Artifact                                         | Platform | Signed                                | Notarized                   | Stapled                   | Checked by                                        | State                                            |
| ------------------------------------------------ | -------- | ------------------------------------- | --------------------------- | ------------------------- | ------------------------------------------------- | ------------------------------------------------ |
| `xFalcon.app` inside the DMG                     | macOS    | Developer ID Application              | Accepted                    | Yes                       | `scripts/verify-macos-release.sh` in the workflow | **Signed and notarized**                         |
| `xFalcon_0.5.37_universal.dmg`                   | macOS    | Developer ID Application              | Accepted                    | Yes                       | Notarize step, then the verify script             | **Signed and notarized**                         |
| Updater archive (`.app.tar.gz`)                  | macOS    | The app inside is Developer ID signed | The app inside is notarized | The app inside is stapled | Verify script, on the extracted app               | The archive itself is not notarized. Its app is. |
| NSIS installer (`.exe`)                          | Windows  | No                                    | No                          | No                        | None                                              | **Unsigned**                                     |
| MSI installer (`.msi`)                           | Windows  | No                                    | No                          | No                        | None                                              | **Unsigned**                                     |
| Application executable in the Windows installers | Windows  | No                                    | n/a                         | n/a                       | None                                              | **Unsigned**                                     |
| Updater signature                                | All      | Yes (minisign)                        | n/a                         | n/a                       | The workflow requires the key                     | **Signed**                                       |
| CycloneDX SBOM                                   | Both     | n/a                                   | n/a                         | n/a                       | Not checked by a script                           | Produced                                         |
| Build provenance attestation                     | Both     | n/a                                   | n/a                         | n/a                       | GitHub attestation store                          | Produced                                         |

### The evidence for macOS

The macOS job is `build (macos-latest, --target universal-apple-darwin)`. These
lines appear in its log.

- The signing identity: `0x00000001 <blob>="Mac Developer ID Application: iSeyon Analytics LLC"` (07:17:34Z).
- App notarization: `Notarizing Finished with status Accepted for id f9ac4d41-578e-415f-b0b1-3652ee84684f (Processing complete)` (07:18:03Z).
- DMG notarization: `status: Accepted` and `Processing: .../xFalcon_0.5.37_universal.dmg` (07:19:08Z).
- Stapling: `The staple and validate action worked!` (07:19:09Z).
- DMG checks: `ok: disk image signed with a Developer ID Application certificate`, `ok: disk image has a secure timestamp`, `ok: the disk image has a stapled ticket (offline machines accept it)`, and `accepted source=Notarized Developer ID` for the image, followed by `ok: Gatekeeper accepts the disk image` (07:19:09Z to 07:19:10Z).
- App checks: `ok: app signature is intact`, `ok: app signed with a Developer ID Application certificate`, `ok: app has the hardened runtime enabled`, and `ok: Gatekeeper accepts the app` (07:19:13Z to 07:19:14Z).
- Updater archive checks: `ok: updater app signed with a Developer ID Application certificate` and `ok: Gatekeeper accepts the updater app` (07:19:15Z to 07:19:15Z).

No error lines appear in the run log.

### The evidence for Windows

The Windows job is `build (windows-latest)`. Its log has this warning, at
07:25:35Z:

```text
##[warning]WINDOWS_CERTIFICATE is not configured -- building unsigned on Windows (official Windows signing is in progress)
```

The same message is printed by the workflow step "Verify updater and Windows
signing secrets" and by "Import optional Windows signing certificate". The build
completed without a certificate.

### Provenance and SBOM evidence

- macOS: `SBOM scan completed in: 10.526s`, then `Attestation created for 3 subjects`
  and `Attestation uploaded to repository`.
- Windows: `SBOM scan completed in: 22.334s`, then `Attestation created for 3 subjects`
  and `Attestation uploaded to repository`.

## macOS signing

macOS signing and notarization are complete for the current release process.
The workflow requires its Apple signing secrets and runs notarization and
Gatekeeper verification for macOS builds (`.github/workflows/desktop-build.yml`;
ADR-052). The v0.5.37 run cited above is historical artifact evidence; it does
not prove the signature of a later candidate. Packaged-binary verification is
covered in [Packaged-binary verification](assurance-plan.md#packaged-binary-verification). This document
does not reopen macOS signing work.

## Windows: what must be done

Windows artifacts remain unsigned in the historical v0.5.37 evidence. The
workflow state must be rechecked before any release claim. A publicly trusted
Authenticode signature gives Windows a publisher identity and detects changes to
the signed file; it does not establish that the application is safe.

### Signing service decision

Evaluate **Microsoft Artifact Signing Public Trust** if the signing legal entity
meets Microsoft's current eligibility rules. The published eligible-organization
country list currently does not include India. Do not infer eligibility from the
developer's location: confirm the contracting legal entity's country and name
with the service before starting enrollment. Alternative: a CA signing service
such as **DigiCert KeyLocker**, subject to its own identity validation, account,
region and CI support requirements. Pricing and this organisation's eligibility
are operational decisions and remain **UNVERIFIED**. Do not purchase until the
legal publisher name and jurisdiction are confirmed.

Official guidance: [Azure Artifact Signing quickstart](https://learn.microsoft.com/en-us/azure/artifact-signing/quickstart),
[trust models and eligibility](https://learn.microsoft.com/en-us/azure/artifact-signing/concept-trust-models),
[signing integrations](https://learn.microsoft.com/en-us/azure/artifact-signing/how-to-signing-integrations),
[Tauri Windows signing](https://v2.tauri.app/distribute/sign/windows/),
[SignTool](https://learn.microsoft.com/en-us/windows/win32/seccrypto/signtool), and
[DigiCert KeyLocker GitHub integration](https://docs.digicert.com/en/digicert-keylocker/ci-cd-integrations-and-deployment-pipelines/plugins/github/binary-signing-using-github-actions.html).

### First signed-candidate checklist

1. Confirm the registered legal publisher name and country. Complete the
   provider's identity validation and create a dedicated signing profile/account.
2. Give CI only the minimum signing permissions. Put credentials or workload
   identity configuration in the protected release environment; do not store
   credentials in the repository or print them in logs.
3. Configure Tauri `bundle.windows.signCommand` for the selected vendor's client.
   Sign the application EXE and the MSI or NSIS installer during bundling. Verify
   the NSIS uninstaller separately; include it if the produced package contains
   a standalone uninstaller.
4. Use SHA-256 file digests and a trusted timestamp service. Generate the
   signature before the updater archive is hashed and signed, so the updater
   signature covers the final signed bytes.
5. On every generated EXE and MSI, run the Windows SDK check below with
   `/pa /all /v /tw`. Confirm a valid signature, the expected publisher and a
   timestamp. Record the artifact hashes and verification output as release
   evidence.
6. Keep Authenticode separate from minisign. Authenticode identifies and signs
   Windows executables/installers; minisign verifies the updater package against
   the app's pinned updater key. One does not replace the other.

### Verification

On a Windows machine with the Windows SDK installed:

```powershell
signtool verify /pa /all /v /tw "<path to file>"
Get-AuthenticodeSignature "<path to file>" | Format-List Status, SignerCertificate, TimeStamperCertificate
```

Check that:

- the status is `Valid`,
- the signer subject matches the verified legal publisher name,
- a timestamp countersignature is present,
- the check passes for the application executable, the NSIS installer, the
  uninstaller and the MSI.

**SmartScreen on a clean virtual machine.** Download the installer through a
browser, so that Windows marks the file as downloaded from the internet. Run it
on a clean Windows virtual machine. Record what SmartScreen shows, the Windows
version and the signer name. Do this for each release candidate.

**Reputation caveat.** A valid signature does not remove SmartScreen prompts on
its own. A new certificate has no download history. Reputation builds over time
as signed files are downloaded. The current Microsoft policy on reputation is
**UNVERIFIED** in this review. Do not promise users that a prompt will not show.

## Signing gates

The current macOS workflow requires its signing configuration and verifies the
signed artifacts. Windows has no Authenticode signing service configured yet.
The updater signature remains mandatory on every platform.

| Platform or artifact | Current source behavior |
| --- | --- |
| macOS signing | The workflow requires Apple signing secrets and runs notarization and Gatekeeper verification. This is implemented; this document does not claim that a later candidate was verified. |
| Windows Authenticode | No signing service is configured. The workflow can produce unsigned artifacts and warns about the missing certificate. Follow the Windows checklist above before claiming signed Windows releases. |
| Updater package | The updater signing key is mandatory and its signature is verified independently of platform signing. |

The test `security::tests::release_workflow_fails_closed_and_separates_signing_from_publication`
pins the updater-key requirement, macOS signing gate, release environment, publication
dependency and staging-only release trigger. The Apple secrets are written to
`GITHUB_ENV` with a random multi-line delimiter. The staging changes pass Windows and Linux native tests; the exact counts are in
[assurance-plan.md](assurance-plan.md#staging-validation-evidence-2026-10-15). This does
not verify a packaged candidate.

## Key management

- **Updater key.** The private key is the `TAURI_SIGNING_PRIVATE_KEY` secret. Its
  scope is stated in [operations.md](operations.md#signing-and-secrets).
  Custody and the holder list are described in [operations.md](operations.md#updater-key-custody).
  Rotation is not retroactive ([decisions.md](decisions.md) ADR-042). No rotation
  is scheduled. The holder list must be written down and reviewed.
- **Apple keys.** The Developer ID certificate is exported as a `.p12` file and
  stored as `APPLE_CERTIFICATE`, with its password. The app-specific password is
  `APPLE_PASSWORD`. Both are described in [operations.md](operations.md#first-time-macos-signing-setup).
- **Windows keys.** With a cloud service, the private key does not leave the
  service. The company holds the account credentials. Those credentials must be
  rotated on a schedule, and the vendor's own rotation rules apply.
- **Where the secrets live.** The build and publish jobs run in the
  `desktop-release` environment (`environment: desktop-release` in the workflow).
  Required reviewers and branch limits for that environment are set by the
  repository owners. The repository settings were not read in this review.
  [operations.md](operations.md#signing-and-secrets) lists the secrets.
- **Repository controls.** The fact sheet records that the `staging` branch has no
  branch protection, that secret scanning and push protection are off, and that
  the CODEOWNERS file lists the three repository admins. It has 35 path patterns
(up from 31 in v0.5.37). The added patterns are `src-tauri/src/lib.rs`,
`src-tauri/src/app/commands.rs`, `src-tauri/src/path_grants.rs` and `src-tauri/windows/`
(startup, the reset command, path grants and the Windows installer hooks). These are IT actions.
  They are not changed by this document.

## Open items

Owners are roles, not names. Dates are proposals. None of them has been agreed.

| #   | Item                                                                                                     | Owner (role)                              | Proposed date         |
| --- | -------------------------------------------------------------------------------------------------------- | ----------------------------------------- | --------------------- |
| 1   | Choose the Windows signing service after confirming legal-entity eligibility, pricing and regions        | IT, compliance and finance                | Proposed 2026-11-30   |
| 2   | Complete the vendor's legal publisher identity validation                                               | Compliance owner                          | Proposed after item 1 |
| 3   | Replace the PFX step; configure `bundle.windows.signCommand` and sign every Windows executable/installer | Release engineering                       | Proposed after item 1 |
| 4   | Confirm the NSIS uninstaller is signed by the build                                                      | Release engineering                       | Proposed with item 3  |
| 5   | Require Windows signing secrets after the service is configured                                         | Release engineering                       | Proposed with item 3  |
| 6   | Remove `continue-on-error` from provenance once the GitHub plan allows it                                | IT, then release engineering              | Proposed 2026-12-15   |
| 7   | Write down the updater key holder list and review it                                                      | Release engineering, with compliance      | Proposed 2026-11-30   |
| 8   | Replace the CODEOWNERS individuals with a team                                                           | IT                                        | Proposed 2026-11-06   |
| 9   | Enable branch protection on `staging`, secret scanning and push protection                               | IT                                        | Proposed 2026-11-06   |

## Related

- [operations.md](operations.md): the release steps, the secrets table and the
  macOS verification procedure.
- [decisions.md](decisions.md): ADR-042, the updater key.
- [threat-model.md](threat-model.md): TM-017, release signing key compromise.
- [compliance.md](compliance.md): the open items that this plan supports.
- [audit-log.md](audit-log.md) and [data-lifecycle.md](data-lifecycle.md): the
  other two reference documents for this release.
- [soc2-control-mapping.md](soc2-control-mapping.md) and
  [assurance-plan.md](assurance-plan.md): control mapping and assurance status.
