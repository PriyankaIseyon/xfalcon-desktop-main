# webview-capability-lockdown Specification

## Purpose

The privileged surface the renderer may reach, and the commands it must never
gain. The allowlist lives in production Rust. Tests prove other copies match it.

## Scope

| File                                     | What it owns here                                                             |
| ---------------------------------------- | ----------------------------------------------------------------------------- |
| `src-tauri/src/security/capabilities.rs` | `APP_COMMANDS`, `FORBIDDEN_COMMANDS`, `PLUGIN_PERMISSIONS`, derived `allow-*` |
| `src-tauri/src/lib.rs`                   | `generate_handler!` registration                                              |
| `src-tauri/build.rs`                     | Tauri AppManifest names that mint permission files                            |
| `src-tauri/capabilities/default.json`    | window capability grant                                                       |
| `src-tauri/src/security/tests.rs`        | parse-and-compare enforcement                                                 |
| `src/security/frontend-boundary.test.ts` | renderer may not call `invoke` except through `native.ts`                     |

## Boundaries

- **Command argument and return types** belong to `crates/contracts`.
- **Per-command behaviour** belongs to the capability that owns that command
  (`reports`, `lean-report-export`, `user-skills`, `preview-protocol`, and others).
- **The preview origin and its per-document CSP** belong to `preview-protocol`.
  This capability owns only the app CSP `frame-src` that names that origin.
- **Navigation and CSP strings** are enforced here; threat identifiers live in
  `docs/threat-model.md`.

## Requirements

### Requirement: The production allowlist is the source of truth

The system MUST declare the exact set of privileged command names in
`security::capabilities::APP_COMMANDS`. Tests MUST parse `lib.rs`
`generate_handler!`, `build.rs` `AppManifest::commands`, and
`capabilities/default.json`, and MUST assert each equals that set (or, for the
capability file, the `allow-*` identifiers derived from it plus
`PLUGIN_PERMISSIONS`).

Tests MUST NOT own a second copy of the command list. Adding a command SHALL
require an edit to `APP_COMMANDS`. A test-only list SHALL NOT be sufficient to
admit a command.

#### Scenario: Handler matches the allowlist

- **WHEN** the security tests run
- **THEN** the names in `generate_handler!` equal `APP_COMMANDS`

#### Scenario: Permission generation matches the allowlist

- **WHEN** the security tests run
- **THEN** the names in `build.rs` equal `APP_COMMANDS`
- **AND** every generated permission file corresponds to one allowlisted command
  and no leftover file remains

#### Scenario: Window capability is derived

- **WHEN** the security tests run
- **THEN** `default.json` admits exactly `allow-*` for each `APP_COMMANDS` entry
  plus `PLUGIN_PERMISSIONS`
- **AND** a command registered but not allowlisted is unreachable and fails the
  test

#### Scenario: A test-only list cannot widen the surface

- **GIVEN** a command name present only in a test constant
- **WHEN** it is absent from `APP_COMMANDS`
- **THEN** the command is not admitted, and the tests that compare registration
  and capability to `APP_COMMANDS` fail if someone registers it anyway

### Requirement: Forbidden commands stay absent

The system MUST declare `FORBIDDEN_COMMANDS` in production code. That set SHALL
include `run_shell`, `write_file`, `write_text_file`, `read_file`,
`read_text_file`, `fetch_url`, `get_credential`, and `upload_file_chunk`.

The set's cardinality MUST be pinned. It MUST NOT grow or shrink without an
edit to that production constant. None of those names MAY appear in
`APP_COMMANDS` or in `generate_handler!`.

#### Scenario: Export does not unlock a renderer-named path

- **GIVEN** `document_export_save` and `document_export_pdf` are allowlisted
- **THEN** `write_file` and `write_text_file` remain forbidden
- **AND** the forbidden set still has eight names

#### Scenario: A forbidden name cannot be registered

- **WHEN** the security tests run
- **THEN** no forbidden name is present in the registered handler
