# Change: Make a crash diagnosable without sending anything anywhere

## Why

The logging that exists is good and is not the problem. `src-tauri/src/lib.rs:91`
installs a daily rolling `xfalcon-desktop.log` through `tracing-appender`, with
an `XFALCON_LOG` filter and a stderr layer beside it. Four things around it are
broken or missing, and each one is small.

**A panic leaves no record.** `install_redacted_panic_hook` is
`set_hook(Box::new(|_| eprintln!("{REDACTED_PANIC_MESSAGE}")))`
(`src-tauri/src/security/panic_boundary.rs:4`). It discards the panic payload —
correct, payloads can carry secrets — but it writes the replacement to stderr,
and a windowed Windows build has no console attached to stderr. It also never
reaches `tracing`, so nothing lands in the log file. The one event most worth
recording is the one event that records nothing, anywhere.

**The log grows without limit.** `rolling::daily` with no `max_log_files` keeps
every day's file forever. On a device that also holds sealed conversation rows
and device credentials, an unbounded and unreviewed log is both a disk problem
and a slowly growing pile of retained data.

**A renderer failure is not recorded.** `AppErrorBoundary` and
`ChatErrorBoundary` catch and present a failure, and tell the Rust side nothing.
A crash that a person saw does not appear in the log a person can send.

**Nobody can hand the log over.** There is no affordance to reveal or package
it. An analyst who hits a defect cannot produce evidence, so a report arrives as
a description of a screen.

## What Changes

- The panic hook appends and synchronizes a structured record to the daily log:
  location, thread, and a backtrace. The panic message itself stays redacted.
- The log is capped by file count, so it cannot grow without limit.
- A renderer failure caught by an error boundary is reported to the Rust log
  through one command, with the component stack and no user content.
- Settings gains one action that writes a diagnostics bundle to a place the
  person chooses, through the existing save path.
- The bundle is composed on the Rust side and its contents are stated. It
  carries recent logs, the app version, and the OS version. It omits the remote
  MCP registry because local bundle generation must not make a network request.
- **No network egress. No crash reporting service. No telemetry.** This is a
  recorded decision, not an omission, and the threat model says so.

## Impact

- Affected specs: `local-diagnostics` (new)
- Affected code: `src-tauri/src/security/panic_boundary.rs`,
  `src-tauri/src/lib.rs`, a new diagnostics module, `src-tauri/build.rs`,
  `src-tauri/capabilities/default.json`,
  `src-tauri/src/security/capabilities.rs`,
  `src-tauri/src/security/tests.rs`, `src/app/AppErrorBoundary.tsx`,
  `src/features/chat/ChatErrorBoundary.tsx`, `src/features/settings/`,
  `crates/contracts/src/`, `docs/threat-model.md`
- Reuses: the bundle is written through `document_export_save`, which is already
  the only path that writes outside app-local data and already sanitizes the
  name. No second destination-holding command is introduced.
- Not affected: the audit trail. `audit/` records what a person did and ships to
  the DA plane. Diagnostics record what the program did and go nowhere. Two
  different jobs; keeping them separate is deliberate and is argued in
  `design.md`.
