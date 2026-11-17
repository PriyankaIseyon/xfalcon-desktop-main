# local-diagnostics Specification

## Purpose

The local diagnostics log and user-selected bundle provide bounded, redacted
troubleshooting evidence without sending diagnostics to a network destination.

## Requirements

### Requirement: Log events are bounded and redacted before they reach a sink

Application log events SHALL pass through write-time diagnostic sanitization
before they reach the file or stderr sink. A single event SHALL retain no more
than 1 MiB before truncation, and truncation SHALL be marked. At startup, existing
active application logs SHALL be streamed through the same sanitizer before
normal logging resumes. This is pattern-based defense in depth; it does not prove
that every possible secret format is detected.

#### Scenario: A credential appears in an event

- **WHEN** a log event contains a supported credential pattern
- **THEN** the emitted event contains a redaction marker instead of its value

#### Scenario: An event exceeds the bound

- **WHEN** one event exceeds the 1 MiB buffer bound
- **THEN** the retained event is truncated with a marker
- **AND** a credential cut at the boundary is not emitted as a partial value

#### Scenario: Existing logs are processed at startup

- **WHEN** active log files contain supported credential patterns
- **THEN** the startup pass streams and redacts those files before normal logging
- **AND** unrelated files are left alone

### Requirement: A Panic Is Recorded On Disk

A panic SHALL append a record to the daily log file and synchronize that file
before the hook returns. The record SHALL carry the panic location, the thread,
and a backtrace. It SHALL NOT carry the panic payload.

#### Scenario: A panic happens in a windowed build

- **WHEN** a panic occurs in a build with no console attached
- **THEN** the daily log file holds a synchronized record at error level naming
  the location, the thread, and a backtrace

#### Scenario: The payload stays redacted

- **WHEN** a panic payload holds a token, a path, or decrypted content
- **THEN** the payload appears nowhere in the log
- **AND** the record carries the fixed redacted message in its place

#### Scenario: A developer runs from a terminal

- **WHEN** a panic occurs in a build started from a terminal
- **THEN** the redacted message is also written to stderr

### Requirement: The Log Is Bounded

The log SHALL keep at most a fixed number of rotated files. It SHALL NOT grow
without limit.

#### Scenario: Rotation passes the cap

- **WHEN** the app has rotated more files than the cap allows
- **THEN** the oldest files are removed and the count stays at or below the cap

#### Scenario: A long-lived install

- **WHEN** the app has run daily for longer than the cap in days
- **THEN** the log directory holds no more files than the cap

### Requirement: A Renderer Failure Reaches The Log

A failure caught by an error boundary SHALL be reported to the Rust log. The
report SHALL carry the error message and the component stack, and SHALL carry no
conversation content, no prompt, no answer, and no credential.

#### Scenario: A boundary catches a render failure

- **WHEN** an error boundary catches a failure
- **THEN** the log holds a record naming the error and the component stack
- **AND** the person still sees the boundary's own recovery surface

#### Scenario: Reporting itself fails

- **WHEN** the report command cannot be reached
- **THEN** the boundary still presents its recovery surface
- **AND** the failure to report does not replace the original failure on screen

### Requirement: A Person Can Produce A Diagnostics Bundle

Settings SHALL offer one action that writes a diagnostics bundle to a location
the person chooses. The bundle SHALL be composed on the Rust side and written
through the existing destination-controlled save path.

#### Scenario: The bundle is written

- **WHEN** the person asks for a diagnostics bundle and chooses a location
- **THEN** a text file is written there holding the recent log contents, the app
  version, and the OS version

#### Scenario: The renderer cannot aim the write

- **WHEN** the renderer requests a bundle
- **THEN** it passes no path, no directory, and no file handle
- **AND** the destination comes only from the native dialog

#### Scenario: The person dismisses the dialog

- **WHEN** the save dialog is dismissed
- **THEN** no file is written and no failure is reported

#### Scenario: The bundle states its own contents

- **WHEN** a bundle is opened
- **THEN** it begins with a section naming every category of information it
  carries

#### Scenario: MCP servers are registered remotely

- **WHEN** a bundle is composed while DA has registered MCP servers
- **THEN** bundle generation makes no registry request
- **AND** the bundle carries no MCP server name, transport, host, or credential

### Requirement: A Bundle Names What This Install Is Configured For

A bundle SHALL state the coordinates this install resolved: which source its
tenant configuration came from, the organization, the workspace origin, the
sign-in issuer, whether the device is signed in, and both loopback ports. It
SHALL state whether a packaged MCP sign-in client, its secret, and a shared MCP
grant key are configured, as presence only and never as a value.

These are this install's own coordinates rather than registry contents, so the
requirement above is unaffected: no registered server is named and no registry
request is made.

Two installs that behave differently are the case a bundle exists for, and a
bundle that cannot say what each one resolved cannot settle it.

#### Scenario: One device lists MCP servers and another does not

- **WHEN** a bundle is composed on each device
- **THEN** each names the configuration source, organization, and workspace
  origin it resolved
- **AND** the two can be compared without reading either device's code

#### Scenario: The tenant configuration did not resolve

- **WHEN** a bundle is composed on an install whose tenant configuration is
  absent or unparseable
- **THEN** the bundle names that source
- **AND** it states that Settings shows no MCP servers for that reason

#### Scenario: A sign-in asks for a client id

- **WHEN** a bundle is composed on a build that packages no MCP sign-in client
- **THEN** the bundle reports no packaged client
- **AND** it states that signing in will ask for a client id

#### Scenario: Secrets are configured

- **WHEN** a packaged client secret or a shared grant key is configured
- **THEN** the bundle reports each as configured
- **AND** neither value appears anywhere in it

### Requirement: A Bundle Carries No Secret

A composed bundle SHALL NOT carry a provider key, an OAuth token, a credential
from the credential store, or a decrypted conversation row.

#### Scenario: A credential is present on the device

- **WHEN** a provider key is stored and a bundle is composed
- **THEN** that key's value appears nowhere in the bundle

#### Scenario: A conversation exists on the device

- **WHEN** conversations are stored and a bundle is composed
- **THEN** no message text from them appears in the bundle

### Requirement: Diagnostics Never Leave On Their Own

No diagnostics record SHALL be sent to any network destination. The app SHALL
NOT upload a crash record, a log, or a bundle.

#### Scenario: A panic is recorded

- **WHEN** a panic is recorded to the log
- **THEN** no outbound request results from it

#### Scenario: A bundle is composed

- **WHEN** a bundle is composed and written
- **THEN** no outbound request results from it
