## ADDED Requirements

### Requirement: Staging delivery

The system SHALL serve a versioned, signed update manifest from a compiled HTTPS
CloudFront endpoint backed by private S3. Candidate uploads SHALL NOT advertise
an update. Promotion SHALL verify assets and signatures before atomically
publishing the staging pointer and SHALL reject downgrade or conflicting bytes.

#### Scenario: Incomplete candidate

- **WHEN** a platform artifact or signature is missing
- **THEN** publication fails and the channel pointer remains unchanged

#### Scenario: Current version

- **WHEN** the channel version equals the running version
- **THEN** the app reports no update

### Requirement: Deferred application

The application SHALL automatically download and verify each discovered newer version, persist the
verified package, and show readiness without exiting or blocking active work.
It SHALL apply only after explicit restart and native idle-state admission.

#### Scenario: Active conversation during download

- **WHEN** discovery finds a newer version during a model run
- **THEN** the run continues and the app displays download progress

#### Scenario: Restart during active work

- **WHEN** a user requests apply during protected work
- **THEN** native admission refuses application without losing the staged offer

#### Scenario: Cached package changed

- **WHEN** cached bytes no longer match the pinned signature
- **THEN** the package is never executed and the user can redownload

### Requirement: Visible update status

The application SHALL expose update availability, download progress, and an
explicit restart action outside Settings, with the same state shown in Settings.
It SHALL recover verified staged readiness after reopening.

#### Scenario: Startup offer

- **WHEN** startup discovers a newer version
- **THEN** the download starts automatically and an app-wide notice reports progress
- **AND** verified completion offers restart without automatically applying the update
