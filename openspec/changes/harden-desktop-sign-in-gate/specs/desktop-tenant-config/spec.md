## MODIFIED Requirements

### Requirement: Bundled Tenant Config Resolution

The app SHALL resolve its Casdoor/DA tenant config from a JSON resource file
bundled at build time. It SHALL accept that file only if the file's SHA-256
equals the digest compiled into the binary from the same file, and SHALL refuse
any other file, including a well-formed one, and log why.

A debug build without a usable bundled file SHALL fall back to process
environment variables (`.env` in a debug build). A release build SHALL NOT read
the four tenant values from the environment. Without a usable bundled file it
SHALL fail with the existing not-configured error.

A value present in the bundled file SHALL NOT require any environment variable
to be set on the target machine.

#### Scenario: Bundled config file present, no environment set

- **WHEN** the app launches on a machine with no `XFALCON_*` environment
  variables and no `.env` file
- **THEN** it reads the DA base URL, Casdoor issuer, client id, and
  organization from the bundled resource file and proceeds to sign-in normally

#### Scenario: The bundled file was edited after the build

- **WHEN** an installed copy's bundled resource file is changed in any way,
  for example to name a different issuer
- **THEN** the app refuses the file, logs that it is not the tenant config the
  build was compiled with, and offers no sign-in against it

#### Scenario: Release build with no usable bundled file and the environment set

- **WHEN** a release build's bundled file is missing or refused and the process
  environment provides all four `XFALCON_*` tenant values
- **THEN** the app fails with the not-configured error and does not use the
  environment

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
