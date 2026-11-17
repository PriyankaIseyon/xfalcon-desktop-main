## ADDED Requirements

### Requirement: Smoke Suite Drives The Built Binary

A smoke suite SHALL drive the built desktop binary through WebDriver. It SHALL
NOT mock the native boundary, and it SHALL NOT substitute a browser for the
app's own webview.

#### Scenario: The suite runs against a build

- **WHEN** the smoke suite is run
- **THEN** it launches the bundled binary produced by the release profile
- **AND** every command it exercises executes the registered Rust handler

#### Scenario: A required binary is absent

- **WHEN** the suite is run with no built binary present
- **THEN** it fails with a message naming the missing artifact and the command
  that builds it
- **AND** it does not fall back to a browser or to mocks

### Requirement: The Suite Covers The Renderer To Rust Seam

The suite SHALL assert the behaviours that only exist when both halves run:
command registration, capability enforcement, the preview scheme, on-disk
persistence, and the storage acknowledgement handshake.

#### Scenario: Every registered command resolves

- **WHEN** the suite asks the running app to invoke each registered command with
  its minimal valid arguments
- **THEN** none answers with an unknown-command error

#### Scenario: A forbidden command stays forbidden

- **WHEN** the suite asks the running app to invoke a filesystem write command
  that the allowlist forbids
- **THEN** the running app refuses it
- **AND** the refusal comes from the shell, not from renderer code

#### Scenario: The preview scheme serves its own origin

- **WHEN** a document is registered and its frame loads
- **THEN** the frame's origin is the preview scheme and not the app origin
- **AND** the document renders

#### Scenario: The acknowledgement handshake completes

- **WHEN** the app is launched and left idle past the acknowledgement timeout
- **THEN** the process is still running

#### Scenario: State survives a restart

- **WHEN** a conversation is created, the app is quit, and the app is launched
  again
- **THEN** the conversation is present from local storage before any network
  call completes

### Requirement: The Provider Is Stubbed At The Network Edge

A run under the smoke suite SHALL reach a stub provider over the network rather
than a stubbed Tauri command, so the command path, the store, and the projection
all execute.

#### Scenario: A turn is sent

- **WHEN** the suite sends one turn with the stub provider configured
- **THEN** the answer streams into the transcript through the real run path
- **AND** the turn is present in local storage after the run settles

### Requirement: The Suite Is A Gate With A Budget

CI SHALL run the smoke suite for a pull request that changes `src-tauri/` and
for every release tag. The suite SHALL complete inside a stated wall-clock
budget, and SHALL fail rather than retry a case that does not pass.

#### Scenario: A shell change is proposed

- **WHEN** a pull request changes any file under `src-tauri/`
- **THEN** the smoke suite runs on Windows and its result gates the merge

#### Scenario: The suite exceeds its budget

- **WHEN** the suite runs longer than its stated budget
- **THEN** it fails with the elapsed time
- **AND** the failure names the case that was running

### Requirement: Visual Suites Assert Against Baselines

Every visual suite SHALL have a baseline image for each case it declares. A
suite with a missing baseline SHALL fail rather than pass silently.

#### Scenario: A baseline is missing

- **WHEN** a visual case has no baseline image
- **THEN** the suite fails and names the case and the expected baseline path

#### Scenario: Baselines are present

- **WHEN** the visual suites run with baselines present
- **THEN** each case compares its render against its baseline
