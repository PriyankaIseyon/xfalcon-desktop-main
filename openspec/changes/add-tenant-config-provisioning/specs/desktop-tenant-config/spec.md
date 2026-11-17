## ADDED Requirements

### Requirement: Bundled Tenant Config Resolution

The app SHALL resolve its Casdoor/DA tenant config by checking, in order: a
JSON resource file bundled at build time, then process environment variables
(`.env` in a debug build), then fail with the existing not-configured error.
A value present in the bundled file SHALL NOT require any environment
variable to be set on the target machine.

#### Scenario: Bundled config file present, no environment set

- **WHEN** the app launches on a machine with no `XFALCON_*` environment
  variables and no `.env` file
- **THEN** it reads `XFALCON_DA_BASE_URL`, `XFALCON_CASDOOR_ISSUER`,
  `XFALCON_CASDOOR_CLIENT_ID`, and `XFALCON_CASDOOR_ORGANIZATION` from the
  bundled resource file and proceeds to sign-in normally

#### Scenario: No bundled file, dev environment set

- **WHEN** the bundled resource file is absent (a plain debug build) and the
  process environment provides all four values (via `.env` or the shell)
- **THEN** the app resolves config from the environment exactly as it does
  today

#### Scenario: Neither source configured

- **WHEN** no bundled file and no environment variables provide the four
  values
- **THEN** the app fails with the existing `NotConfigured` error naming the
  missing variables

### Requirement: CI Builds a Named Tenant From Typed Inputs

The CI build SHALL run only via `workflow_dispatch`, taking `version` and the
four tenant values as required typed inputs, and SHALL produce a bundle
configured for exactly those values. No tag-push trigger SHALL exist.

#### Scenario: Manual dispatch

- **WHEN** a person runs `workflow_dispatch` with `version` and the four
  tenant values filled in
- **THEN** the build renders those values into the bundled resource file
- **AND** the resulting release uses `version` as its tag and release name

#### Scenario: A required input is left blank

- **WHEN** `workflow_dispatch` is run with one of the five required inputs
  empty
- **THEN** the run is rejected before the build starts (GitHub Actions
  enforces `required: true` on dispatch inputs)

### Requirement: Bundled Config Is Not a Secret

The bundled tenant config file SHALL hold only DA base URL and Casdoor
issuer/client id/organization. It SHALL NOT hold provider API keys, MCP
grants, or any value the app otherwise stores encrypted via the OS keyring.

#### Scenario: Reviewing bundle contents

- **WHEN** the bundled resource file is inspected on an installed copy
- **THEN** it contains only the four tenant-identification values in plain
  JSON
- **AND** no provider credential or MCP grant appears anywhere in the app's
  install directory unencrypted
