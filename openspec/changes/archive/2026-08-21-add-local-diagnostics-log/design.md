## Context

The existing panic hook discards the payload on purpose. A panic message is
built from whatever was in scope — an `unwrap` on a decrypted row, an `expect`
carrying a path or a token — so the payload is untrusted for disclosure. That
judgement was right and this change keeps it.

What the hook confuses is the payload with everything else about the panic. The
location is a file and a line in this repo. The thread name is one this repo
chose. A backtrace is a list of symbols in this binary. None of that is user
data, and all of it is what makes a crash diagnosable.

## Goals / Non-Goals

- Goals: a crash is diagnosable from a file on the device; the log cannot grow
  without limit; a person can hand the evidence over deliberately.
- Non-Goals: any network egress; a crash-reporting service; usage analytics;
  session replay; automatic upload of anything, ever; capturing prompt or answer
  content.

## Decisions

- **Decision: record the frame, redact the payload.**
  The hook logs `panic` at error level with `location`, `thread`, and a captured
  backtrace, and substitutes the existing `REDACTED_PANIC_MESSAGE` for the
  payload. The constant stays exported so the existing test that asserts
  redaction keeps asserting it.
  - Alternative considered: log the payload behind an environment flag.
    Rejected: a flag that discloses secrets is one support instruction away from
    being set on a real device, and the instruction will read "just set this to
    debug it".

- **Decision: the hook appends and synchronizes its record directly.**
  stderr on a windowed Windows build goes nowhere. The regular `tracing` file
  layer uses a non-blocking writer, so a release panic with `panic = "abort"`
  can also end the process before its buffer reaches disk. The hook therefore
  appends its complete redacted record to the current daily log and calls
  `sync_data` before it returns. It keeps the stderr write for developers who
  run the app from a terminal.

- **Decision: diagnostics are separate from the audit trail.**
  `audit/` answers "what did this person do", is queued, and is published to the
  DA plane. Diagnostics answer "what did this program do" and must never leave
  the device on their own. Merging them would put backtraces into an egress path
  and would put an unbounded local log behind a publisher. They stay separate.

- **Decision: the bundle is composed in Rust and written through
  `document_export_save`.**
  The renderer never reads a log file and never names a path. It asks for a
  bundle; Rust composes the text and hands it to the one save path that already
  opens the native dialog and sanitizes the name. This keeps the property
  `security/tests.rs` records — the renderer may ask for a write, never aim one.
  - This means the bundle is text, not a zip. Accepted: text is reviewable
    before sending, which is the point.

- **Decision: the person can read the bundle before it goes anywhere.**
  It is saved to a file they chose, and nothing sends it. A diagnostics artefact
  that a person cannot inspect is one they should not be asked to send.

- **Decision: the bundle omits MCP registry data.**
  The authoritative MCP server list comes from DA's remote registry. Bundle
  generation is local and must cause no network request. The local encrypted
  credential database is not a server registry and must not be inspected for
  display data. The bundle therefore carries no MCP names, transports, hosts,
  or credentials.

- **Decision: cap the log by file count, not by age.**
  `RollingFileAppender::builder().max_log_files(n)` bounds the count
  deterministically. Age-based cleaning needs a sweeper and a clock, and gets the
  same result less predictably.

- **Decision: a renderer failure is reported through one command that takes a
  message and a component stack, and nothing else.**
  Not the props, not the store, not the transcript. A component stack is a list
  of component names from this repo, which is the same class of data as a
  backtrace.

## Risks / Trade-offs

- A backtrace names internal symbols. That is a modest disclosure to whoever
  reads the bundle, and the person choosing to share it is the one who reads it
  first. Accepted, and stated in the threat model.
- A log line written elsewhere in the app might already carry user content, and
  the bundle would carry it along. Mitigation: the bundle states what it
  contains, and a test asserts that a known secret placed in the credential
  store never appears in a composed bundle. That test is the real control, not
  the prose.
- Capping by file count loses the oldest days. Accepted: a defect reported weeks
  later is reported from memory anyway.

## Migration Plan

Nothing stored changes shape. An existing log directory keeps its files, and the
count cap applies from the next rotation. No rollback concern: reverting
restores the previous hook.

## Resolved Questions

- The log cap is seven daily files.
- MCP registry data is omitted because its truthful source is remote and bundle
  generation must stay local.
