## ADDED Requirements

### Requirement: Signed Release Trust Chain

The release workflow SHALL publish updater-signed and operating-system-signed
desktop artifacts. It SHALL fail before packaging when required signing or
notarization material is absent.

#### Scenario: Signing material is missing

- **WHEN** a release job lacks signing material for its platform
- **THEN** the job fails with the missing secret name
- **AND** it does not run the packaging and publication step

#### Scenario: Windows updater manifest is published

- **WHEN** a signed Windows release is published
- **THEN** `latest.json` selects the per-user NSIS updater artifact
- **AND** one publisher writes the manifest only after both read-only platform jobs pass
- **AND** publication fails if the release tag already exists

### Requirement: Pinned Signed Update Discovery

The application SHALL use a compile-time HTTPS manifest endpoint and Ed25519
public key. The renderer SHALL NOT supply an endpoint, public key, or artifact
path.

#### Scenario: Update artifact is modified

- **WHEN** the downloaded artifact does not match its updater signature
- **THEN** Tauri rejects it before installer execution

#### Scenario: A future CDN is added

- **WHEN** a second delivery source is approved
- **THEN** its HTTPS endpoint is compiled into the ordered endpoint list
- **AND** it serves artifacts signed by the existing release trust chain

### Requirement: Native Background Discovery Cadence

The application SHALL check after secure startup and every four hours while it
runs. The native runtime SHALL own the periodic cadence. Background failures
SHALL NOT display an alert or block workspace startup.

#### Scenario: Background update is available

- **WHEN** a startup or scheduled check returns a newer signed version
- **THEN** Settings records its version and release notes
- **AND** no download, installation, or relaunch starts automatically

#### Scenario: Background check fails

- **WHEN** a background check cannot reach the manifest endpoint
- **THEN** the store records the check time and failed outcome
- **AND** the application stays idle without a user-visible error

### Requirement: Explicit Work-Aware Installation

The application SHALL install an update only after explicit user action. It
SHALL refuse installation or relaunch while a model run or file transfer is
active.

#### Scenario: User installs while idle

- **WHEN** a signed update is available and no protected work is active
- **THEN** the application downloads, verifies, and installs it
- **AND** native work admission blocks new runs and transfers until restart
- **AND** it reports byte and percentage progress when size is known
- **AND** it relaunches when the platform installer returns control

#### Scenario: User installs during protected work

- **WHEN** a model run or file transfer is active
- **THEN** the installation action is disabled where that state is visible
- **AND** the native command refuses direct installation calls
- **AND** no download, installation, or relaunch starts

### Requirement: Update Status and Resource Lifecycle

Settings SHALL show the running version, last-check time, availability,
progress, and actionable errors. The application SHALL release pending native
update state when the user declines an update or installation completes.

#### Scenario: Manual check fails

- **WHEN** the user requests a check and it fails
- **THEN** Settings displays one inline error
- **AND** it records the check time and failed outcome

#### Scenario: Relaunch fails after installation

- **WHEN** installation completes but relaunch fails
- **THEN** Settings presents one non-duplicated warning
- **AND** it keeps a retry-relaunch action available
