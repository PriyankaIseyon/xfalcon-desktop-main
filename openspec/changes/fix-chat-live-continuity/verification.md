# Verification record

Date: 2026-10-06. Branch: `staging`. Baseline: `e1e1c22`.

Validation base HEAD: `3c4e9db`. The concurrent
provider changes were committed by other work; their source was already present
before the final native checks and was preserved.

## Behavior evidence

- `agent/local_executor.rs`: the real stream core passes fault injection for a
  blocked history write, painting before that write resolves, ordered final
  draining, record and byte backpressure, and cancellation during a blocked write.
  The blocked-write paint test failed against the original stream core before
  the implementation changed.
- `chatStore.test.ts`, `turnEditStore.test.ts`, `useChat.navigation.test.tsx`:
  adopted background runs retain text, reasoning, activity, and round prose;
  duplicate events are ignored; terminal snapshots retire live state; failed
  revisions restore sequence watermarks; failed and chained opens restore the
  latest authoritative source. Cache eviction drops retired reasoning before
  waiting live runs, with a regression covering 13 completed conversations.
  The real store also applies deltas without
  scanning a 2,000-message durable transcript. Account reset and deletion remain isolated.
- MCP session/runtime tests use the HTTP/session consumer to prove schema refresh
  across a new connection, one concurrent catalog refresh, recovery after a
  failed dirty refresh, and progress cleanup when a call future is dropped.
- Tool-loop tests cover per-call, resumed-call, and round timeout cancellation
  without cancelling the owning run. The Stop integration fixture now cancels
  its explicit owning-run token rather than a per-call executor token; its result
  persistence and terminal ownership assertions are unchanged.
- Mounted caret tests cover wrapped endpoints, surrogate pairs, and finding the
  last painted text without scanning 250 settled paragraphs. Chromium tests
  exercise live conversation switching and duplicate delivery in both motion
  modes, plus the existing scroll, rich-content, Stop, retry, and reopen checks.

## Initial executed checks

| Check                                                                    | Result                                                                                                                    |
| ------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------- |
| `npm test`                                                               | 2,014 Vitest tests in 144 files; 43 release tests; 3 launcher tests passed. Font inventory passed.                        |
| `npm run test:e2e:chat -- --workers=4`                                   | 16 Chromium tests passed.                                                                                                 |
| `npm run build`                                                          | Type checks, production frontend build, style and tooltip checks passed.                                                  |
| `npm run lint`                                                           | Passed with zero warnings.                                                                                                |
| `npm run format:check`                                                   | Passed.                                                                                                                   |
| `npm run check:matrix`                                                   | Passed.                                                                                                                   |
| Windows MSVC targeted tests                                              | Stream core 13; tool loop 38; MCP session 15, runtime/multi-server 11, catalog 7, executor 11; Stop integration 5 passed. |
| Windows `cargo clippy --workspace --all-targets --locked -- -D warnings` | Passed.                                                                                                                   |
| `cargo deny check`                                                       | Advisories, bans, licenses, and sources passed under the existing policy.                                                 |
| Changed Rust files: `rustfmt --edition 2021 --check`                     | Passed.                                                                                                                   |
| Initial code review and `git diff --check`                               | Diff check passed. The later source audit found the rollback and reconnect gaps recorded below.                           |

## Follow-up source-audit fixes

The source re-audit identified two gaps in chat rollback and an existing MCP
session-invalidation race. This follow-up keeps the original desktop-only staging
scope and preserves unrelated concurrent provider work.

- The real store regression for destination text, reasoning, round prose, and
  approval received during a failed reopen failed before the fix. Rollback now
  retains the departing target before replacing its visible state. Full source
  snapshot equality and subsequent approval admission are checked.
- Both failed and interrupted source regressions failed before the fix. A retired
  reasoning entry now cannot overwrite the reconciled restore snapshot's retry,
  recovery, or error fields. Repeated authoritative snapshots also reproduced the
  omission and now keep the same complete source snapshot. The interrupted
  regression uses the native snapshot's active run ID and subsequent state event;
  it failed until the existing matching retry target kept its terminal state.
- The real `useChat` rejected-open and subsequent reopen path retains destination
  approval, reasoning, and round prose, while preserving the source and the
  command's failure notice.
- MCP invalidation now compares the captured session generation while holding
  the runtime lock. A replaced session cannot clear its successor's session or
  catalog, including after the successor is itself cleared. The latest
  successful generation remains recorded after session removal so its late
  authorization refusal still forgets the credential and requests sign-in.
- Source review identified a further gap between checking the generation and
  deleting its credential. Cleanup and notification now finish under the same
  lock for handshake, catalog, and tool-call refusals. The real HTTP dispatcher
  fixture sends concurrent mixed failures and blocks credential deletion while
  directly polling a reconnect. Unlocked cleanup failed with two authentication
  entries instead of one; guarded cleanup passes and reconnect succeeds after
  deletion is released. The fixture waits have explicit deadlines.

The final frontend source passes 107 focused chat tests, all 2,018 Vitest tests
in 144 files, 43 release tests, 3 launcher tests, and the font inventory. Type
checks, the production build, style and tooltip checks, lint with zero warnings,
and repository Prettier checks also pass.
All 16 Chromium chat checks pass on the final frontend source, including both
motion modes. Independent source review found no remaining frontend findings.
Independent final MCP source review also found no remaining scoped findings.
The final native MCP filters pass: multi-server 12, reauthorization 12, and all
MCP tests 165. The guarded concurrent-refusal regression passes independently.
The final full serial Windows MSVC workspace run passes: 2,109 desktop library
tests, both contract suites (9 and 31), and the remaining binary/doc test targets.
The library retains its existing 24 ignored tests. A final test-only cleanup
adjustment releases the blocked credential-deletion thread before assertions;
the exact concurrent-refusal test passes after that adjustment. Windows MSVC
`cargo clippy --workspace --all-targets --locked -- -D warnings`, whole-workspace
Rust formatting, repository Prettier checks, and strict OpenSpec validation pass.

## Review of `33e75d8`: stale refusals and catalog lock contention

This follow-up fixes review findings 1 and 2 on `staging`. The frontend, provider
code, generated contracts, and timeout behavior are unchanged.

- Two real HTTP dispatcher regressions failed before the production fix: an old
  session's delayed refusal returned `AuthorizationRefused` instead of
  `Disconnected`, both while its replacement was connected and after the
  replacement was cleared. Both now pass. A subsequent actual tool call succeeds
  without deleting the replacement's credential or requesting sign-in.
- A real `tools/list_changed` refresh is paused inside its HTTP listing. Before
  the fix, directly polling session invalidation returned `Pending`. It now
  completes immediately, and replacement authentication starts while the old
  listing is still paused. After release, the replacement obtains its own schema
  and rejects arguments that only the old schema accepted.
- The reconnect regression also closes a session through its lifetime token and
  verifies that a fresh connection gets a fresh catalog. Removing explicit cache
  eviction preserves the existing exact-generation validation.
- All 168 native MCP tests pass on the final production behavior. Independent
  source review found no remaining scoped finding in the dispatcher, runtime,
  cache, refusal cleanup, or regression fixtures.
- The full serial Windows MSVC workspace run passes: 2,112 desktop library tests,
  9 and 31 contract tests, and the remaining binary/doc test targets. The existing
  24 ignored library tests remain unchanged. Serial execution preserves the
  reported default-parallel gate exception below; no test assertion was changed.
- Windows MSVC `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  whole-workspace Rust formatting, changed-document Prettier checks, and strict
  OpenSpec validation pass.

## v0.5.30 release audit

The source audit covers the MCP runtime and dispatcher, catalog generations,
Qwen request construction, streaming reveal and caret, composer tooltip, and
release cleanup. Independent read-only reviews found no remaining scoped
production defect. The release security guard was updated to check the exact
jobs allowed to write release contents, rather than an obsolete global count.
Both workflows are also checked for staging-only cleanup after successful
delivery, read-only defaults, and shared cleanup concurrency.

- The full frontend gate passes: 144 files and 2,022 Vitest tests, 61 release
  script tests, formatting, type checking, lint, and UI contract checks.
- All 17 Chromium chat checks pass, including normal and reduced-motion
  streaming, settling, conversation switches, reopening, and caret geometry.
- The full serial Windows MSVC workspace run passes: 2,114 desktop library
  tests, 9 and 31 contract tests, and binary/doc targets. The existing 24 ignored
  library tests remain unchanged. The default-parallel exception below remains
  recorded; test concurrency was not changed.
- Whole-workspace strict Clippy and Rust formatting, the production frontend
  build, cargo-deny policy checks, and strict OpenSpec validation pass.
- A live GitHub cleanup dry run correctly keeps five of 23 draft releases and
  sends no deletions. The workflow will run cleanup after successful delivery.

Qwen request tests prove that guidance is sent while answer and reasoning events
remain separate. Live model adherence to that instruction was not verified and
is not guaranteed. No response buffering or heuristic text filtering was added.

## Native parallel gate exception

The initial default parallel Windows workspace run has one failing test:
`mcp::tests::reauth::diagnostics_export_keeps_failure_category_without_registered_server_details`.
It fails to capture the expected discovery warning. The same test fails at the
same assertion in a clean `git archive HEAD` control using the same toolchain,
workspace command, lockfile, and test-profile override. The test passes in two
isolated runs. Its root cause remains unproven; no assertion was changed or
suppressed.

- Updated tree: 2,102 desktop library tests passed, 1 failed, 24 ignored; both
  contract suites passed (9 and 31 tests).
- Clean staging control: 2,091 desktop library tests passed, the same 1 failed,
  24 ignored; both contract suites passed (9 and 31 tests).
- All 11 added native tests passed in the parallel run.

The initial full serial Windows workspace run passed with `-- --test-threads=1`:
2,103 desktop library tests, 9 and 31 contract tests, and the remaining binary/doc
test targets passed. The library retains its existing 24 ignored tests.
Repository test concurrency was not changed.

## Execution boundaries

The default test executable is held by an unrelated process. Native tests use
the command-only `--config profile.test.package.xfalcon-desktop.debug=1` override
to select a separate executable. No repository profile or gate configuration was
changed.

The initial whole-workspace Rust format check flagged concurrent provider debug
additions in `providers/custom_openai.rs` and `providers/openai_compat.rs`. Those
changes belong to other work and were preserved. The final whole-workspace Rust
format check passes on the current tree.

No macOS execution, installed native-app smoke test, live gateway inference, or
production MCP call was performed. Chromium tests use mocked native events with
the real mounted frontend. The `33e75d8` changes were committed and pushed after
the earlier checks. The review follow-up was validated locally. No app release or
deployment was performed as part of this validation.

## Chat, Settings, compaction, and multi-file follow-up

Date: 2026-10-06. Branch: `staging`. Baseline: `3414650` (app version `0.5.30`).
This follow-up is local Desktop work. No DA, generated contract, dependency,
version, release, or installed-app change was made.

### Source findings and behavior

- `MessageList` now keeps the virtualizer callbacks stable while the live answer
  changes. The real 10,000-message mounted fixture reproduced 80 re-estimates of
  a settled offscreen row during 80 updates; the fixed path performs zero. A real
  history replacement still refreshes measurements.
- All five Settings removal actions share the same visible `Remove` label,
  danger styling, icon, and control behavior. Confirmation dialogs use `Cancel`
  and retain specific final action labels. Initial Cancel focus and return to
  the opener use the shared modal lifecycle, including descriptions with links.
- Luna/Sol client summaries are admitted only after a nonempty, naturally
  completed response. Truncated, tool-use, context-limit, refusal, and cut-off
  responses cannot replace history or enter the summary cache. Existing bounded
  hard-limit recovery remains available when a summary cannot make history fit.
- Sonnet malformed SSE returns promptly and cancels only its child read.
  Delayed-header cancellation is tested through the real HTTP transports. Strict
  Sonnet thinking preserves the exact pre-turn prompt on ordinary and recursive
  continuation rounds. Native compaction invalidates old thinking lineage while
  retaining fresh post-compaction thinking and tool/text/result content.
- File modify, delete, and rename recheck their base hashes immediately before
  apply. Renames recheck existing destinations and refuse newly occupied ones.
  Both directory mutation variants now recheck the existing platform filesystem
  identity through a revalidated capability handle. A Windows fixture gave a
  replacement directory identical creation time and attributes: the old guard
  wrongly deleted it; the fixed path refuses both deletion and rename.
- Approval resolution, encrypted backup capture, file mutation, and terminal
  approval persistence run in one owned blocking job. One shared mutation gate
  covers duplicate and nested project roots; reads and proposal preparation
  remain independent. Aborting the async caller cannot detach a started write
  from its status persistence. Queued agent writes observe cancellation before
  touching disk; started writes drain and journal their actual result on Stop,
  per-call timeout, and round timeout. A round deadline still terminates the run.
- A real multi-operation durability fault reproduced unused staged files and
  parent directories surviving a failed parent flush. Cleanup now removes only
  the unapplied tail and closes its handles before removing empty new directories.
  The landed operation remains counted, with durability explicitly unconfirmed.
- Desktop prompt guidance requires named and automatically relevant skills,
  complete instruction paging, needed reference files, and refetch when exact
  instructions are lost after compaction. The real driver reproduced old skill
  pages becoming generic stubs. Ordinary pruning now preserves instruction,
  continuation, and reference results exactly. Full summarization or emergency
  shedding still respects the hard context limit; skill selection and refetch
  remain model-directed, not a guarantee of model adherence or extra authority.

An independent source review found the directory identity and durability-tail
issues above. Both were reproduced before their fixes. Its bounded follow-up
found no further scoped bugs. Review findings were checked against production
code and real consumers, not accepted from test results alone.

### Reference comparison

The sibling `references/` checkout was inspected read-only. Useful production
paths were compared, without copying a framework or changing their sources:

| Reference source                                           | Relevant behavior and Desktop result                                                                                                                         |
| ---------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `codex/codex-rs/core/src/tools/parallel.rs:205`            | Parallel-capable calls share a read gate; other calls take an exclusive gate. Desktop now serializes local mutation jobs while preserving independent reads. |
| `opencode/packages/opencode/src/tool/edit.ts:35`           | Resolved-file edits use a semaphore. Desktop covers overlapping roots with one mutation gate and rechecks staged hashes before apply.                        |
| `opencode/packages/opencode/src/session/compaction.ts:297` | Skill outputs are excluded from ordinary pruning. Desktop now preserves its skill-fetch instruction and reference pages during the equivalent step.          |
| `opencode/packages/opencode/src/tool/read.ts:16`           | Reads use bounded line pages. Desktop already pages with an explicit continuation and a model-scaled JSON byte budget.                                       |
| Codex apply-patch and OpenCode `tool/apply_patch.ts`       | Files are applied sequentially; these paths do not provide a cross-file rollback transaction. Desktop explicitly reports applied and pending work.           |

The `pi-mono` checkout was absent. No comparison against it is claimed.

### Verified limits and practical effect

| Surface                | Existing bound or behavior                                                                                                                                                                       |
| ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Model input            | Uses the catalog's context/output capacity, protocol reserve, conservative initial estimate, and provider-usage anchors. There is no fixed file-count context window.                            |
| Model tool-result page | Scales with context; 32 KiB floor and 256 KiB ceiling, including JSON escaping and response overhead.                                                                                            |
| Project text page      | At most 2,000 lines, with a 20,000-byte per-line clipping limit and explicit partial-page metadata. Whole-file UI reads have a separate 1 MiB cap.                                               |
| Mutation plan          | At most 200 operations and 128 MiB aggregate file content; editable files and encrypted backups have a 64 MiB per-file bound. Model argument/output budgets can impose smaller practical limits. |
| Tool run               | At most 8 parallel calls, 64 rounds, 512 total calls, and 64 calls to an individual tool.                                                                                                        |

For a ten-file task, read the relevant sections in parallel, apply small exact
edits through the serialized writer, and read back the changed sections. A file
that changed underneath an edit is refused and must be reread. A larger model
window can help with broader reasoning, but raising disk or output limits alone
does not improve correctness. No global limit was raised in this follow-up.

### Executed checks

Final Windows `cargo clippy --workspace --all-targets --locked -- -D warnings`
passed after the review fixes.

| Check                                 | Result                                                                                                                                                                                                                       |
| ------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Frontend unit suites                  | 2,024 Vitest tests in 144 files, 61 release tests, and 3 launcher tests passed.                                                                                                                                              |
| Frontend static/build checks          | Type checks, production build, ESLint with zero warnings, style/tooltip/font/UX checks passed. The first combined gate reported a Diagnostics formatting issue; it was fixed, and the full standalone Prettier check passed. |
| Production Chromium chat and Settings | 20 tests passed, including full final answer text, retained composer draft, bounded history, terminal caret removal, and matching removal controls in light/dark and narrow/large-reading layouts.                           |
| Settings visual checks                | 6 passed; only the five affected Settings baselines were updated and inspected.                                                                                                                                              |
| Windows MSVC native suites            | The final desktop library run passed 2,141 tests with the existing 24 ignored. Both unchanged contract suites also passed (9 and 31), along with binary/doc targets in the preceding full workspace run.                     |
| Rust formatting                       | Final workspace format check passed.                                                                                                                                                                                         |
| Cargo dependency policy               | Advisories, bans, licenses, and sources passed; no dependency change was made.                                                                                                                                               |
| Scoped OpenSpec                       | `local-project-execution`, `run-prompt-assembly`, `chat-streaming-performance`, `add-provider-native-context-compaction`, and `fix-chat-live-continuity` passed strict validation.                                           |
| Final diff                            | `git diff --check` passed; generated contracts, other repositories, and unrelated source remained unchanged.                                                                                                                 |

The production Chromium profile used 10,000 saved messages and a complete
120-paragraph answer delivered over 136 animation frames with 4x CPU throttling.
It recorded a 16.8 ms p95 frame gap, a 33.4 ms maximum, no long tasks, and nine
mounted history rows. These are one controlled browser run's diagnostics, not a
native-app latency guarantee. Production timings are not compared against the
development build. The deterministic 80-to-zero offscreen re-estimate count is
the before/after performance evidence.

One full native attempt failed to read existing files over the Windows/WSL share
with OS error 59; the same command succeeded on retry. The final library run
above passed after the review fixes. The earlier baseline parallel-test exception
remains unchanged; serial execution does not claim that unrelated shared-state
test has been fixed. The whole OpenSpec sweep also has unrelated baseline
failures, proven against a clean archive of `3414650`; scoped checks above pass.

### Remaining execution boundaries

Multi-file application is not a transaction. An external process can still race
between the final precondition check and a filesystem syscall; no cross-platform
atomic compare-and-swap/no-replace primitive was introduced. Read continuation
pages reopen the file and are not revision snapshots; reread relevant text before
editing. A clipped very long line cannot be reconstructed from line continuations.
Hard-limit shedding explicitly omits context, so it does not guarantee lossless
memory. These bounds were retained rather than silently presenting partial data
as a complete read.

No live OpenRouter inference, production MCP call, installed Windows WebView
smoke test, or macOS execution was performed. Browser checks use the real
production frontend with mocked native IPC. The existing two OpenRouter
protocols serve Sonnet native compaction and Luna/Sol client compaction; no
configured provider path was removed. Live multi-file tasks with skills,
compaction, Stop, and reopen on each model remain the qualification needed for a
stronger product-level rating. This follow-up has not been committed, pushed,
released, or installed.

### Repeat source audit — 2026-10-06

The requested second code audit found three additional defects. They were fixed
within the existing staging change, without changing public contracts, generated
schemas, dependencies, model routing, or file/context limits.

- Both complete hash readers stopped at a grow beyond their checked size bound
  and returned success with a partial buffer/hash. They now return an explicit
  `InvalidData` error. Whole-file reads surface that error; indexing retains its
  earlier record as a transient failure. Real disk tests grow an already opened
  file after its metadata check, and also verify empty and exact-limit hashes.
- Partial project reads treated a NUL beyond the initial sniff as UTF-8 text.
  They now return binary metadata with no text, including when the binary line
  was scanned before the requested start. The real project tool dispatcher
  regression covers a first page, the binary line, and text after that line.
- Hard-guard native compaction summarized only durable history, then discarded
  an unfinished assistant continuation when it reloaded that history. It now
  restores the actual unsaved prose tail in order, without obsolete thinking or
  status reminders. It does not restore earlier prose already summarized by
  threshold compaction. Driver and Messages-serializer checks cover both Sonnet
  routes, output-limit/stream-cut continuations, fresh signed tool rounds, and a
  threshold compaction followed by a hard guard.

The buffered read, late binary marker, and lost continuation scenarios were
reproduced before their fixes. An independent source reviewer checked the native
compaction and local mutation paths and caught the skipped-line binary sibling.
The root checked the corrected code and its callers, along with the frontend
virtualizer, Settings actions, skill assembly/pruning, and deadline handling.
The final reviewer found no further continuation or signed-prefix defect.

The targeted Windows MSVC checks passed: 23 project read tests, 45 project tool
dispatcher tests, and two hard-guard driver tests. The final complete desktop
library run passed 2,146 tests with zero failures and the existing 24 ignored.
Workspace/all-targets Clippy also passed with `-D warnings`. No frontend behavior
changed in this repeat audit; the prior frontend results remain applicable.
Rust formatting, changed-document Prettier, the three affected scoped OpenSpec
validations, and the final diff whitespace check passed. The full native suite
ran serially on Windows MSVC; live provider and installed-app qualification
remain unperformed as disclosed above.

The `80 → 0` claim counts additional size estimates for one unchanged offscreen
history row during 80 live-tail updates in a mounted 10,000-message fixture.
Previously a newly allocated virtualizer key callback invalidated cached
measurements on each update. Stable callbacks keep those measurements cached.
Initial layout still estimates sizes; a real history replacement still updates
them. This does not claim zero renders or an 80-fold application speedup.

Skills remain account-scoped and are discovered at each run's assembly. Every
enabled skill is advertised; the prompt directs the model to fetch named or
relevant instructions completely and refetch after compaction when necessary.
Ordinary tool-result pruning preserves skill pages. Relevance selection remains
the model's decision, rather than a deterministic per-question router. Correct
file selection and semantic edits likewise require model judgment. Compaction
is a summary, not lossless context, and multi-file writes can report an applied
prefix on failure rather than provide a cross-file rollback transaction.

### Project navigation and final release audit — 2026-10-06

The final change remains on staging and prepares version 0.5.31. Project tree
selection backgrounds now start beyond each actual curved connector endpoint.
Project chat titles, subtitles, and rename controls fit the shared fixed-height
header at all three reading sizes. Regular and project sidebar chat names share
the available width and reserve space for their actual Pin/Delete controls only
when those controls appear. Clipped names retain the full-name tooltip.

Desktop automatic naming now requests concise plain title text, matching the
working web flow, and accepts both that text and the existing JSON response.
Completed-response validation and manual-title precedence remain enforced. The
observed Luna title failure was traced to the previous JSON-only parser; no live
provider naming call was performed in this pass.

An independent source audit compared every changed production file with the
previous release commit, `3414650b977e60459873653aaecc4dcd1629d18d`, and traced its
real callers. Three confirmed findings were fixed and re-audited:

- Native hard-guard compaction stopped at replay-only status/recursive prefixes
  before later saved Sonnet tool rounds. It now retains the exact signed wire
  prefix through the last durable message. Candidate boundaries end at saved
  messages and complete tool groups; source counts and hashes cover only saved
  identities. The true unsaved continuation stays outside source coverage and
  returns once after compaction. A real driver regression first reproduced
  `IndivisibleOverhead`, then passed for both Sonnet routes with actual tool
  results and Messages serialization.
- Bounded line reads discarded long-line suffix bytes without checking them for
  NUL. They now classify every consumed chunk while retaining the same memory
  bound. Real disk regressions first returned text incorrectly, then passed for
  returned, skipped, and continuation lines containing a late binary marker.
- Keyboard focus could leave rename controls without committing, and Escape on
  a control could reach shell cancellation. The whole edit region now owns blur
  and Escape. Real Tab/Enter/Escape tests cover save, cancel, and outside blur.

The independent re-audit found no further confirmed defect. The locked Windows
MSVC workspace run passed: 2,150 desktop tests, 9 contract unit tests, and 31
contract snapshots, zero failures, with the existing 24 desktop tests ignored.
Binary and doc-test targets also passed. Rust formatting, workspace/all-targets
Clippy with `-D warnings`, and the dependency policy check passed. Frontend
validation passed: typecheck, lint,
formatting, command matrix, production build, script/launcher tests, and 2,026
Vitest tests. All 22 production-build browser tests passed, including project
tree geometry, title widths, hover actions, header bounds, unsaved project chats,
Settings actions, streaming, scrolling, and reopen behavior. The old sidebar
width defect was reproduced against the prior production build before the fix.

The controlled long-chat browser check measured about 16.8 ms at the 95th
percentile under four-times CPU throttling, with no long tasks and nine mounted
history rows. This measures frontend behavior with mocked native IPC, not live
inference. The affected scoped compaction and local-project specifications and
the final diff whitespace check passed. Live OpenRouter/MCP multi-file tasks and
installed-app qualification remain outside this evidence; macOS execution is
left to the release workflow. No provider routes, dependencies, generated
contracts, file limits, or model context limits were changed.
