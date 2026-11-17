# PR Review: Project availability and conversation cleanup

**Classification**: Cross-cutting · high risk

**Verdict**: LGTM-with-suggestions

**Blast radius**: Project trust, local and cloud chats, active tools, account isolation, sidebar state, and native filesystem handles.

## Summary

The audit found and corrected unsafe deletion evidence, Windows handle sharing, stale chat restoration, and run ownership defects. Cleanup now requires definitive evidence, stops associated local execution, and preserves retry state. Windows retains unavailable projects and chats because its pending-deletion state is reversible.

## Blocking

No unresolved implementation blocker in the audited paths. The following blocking findings were corrected:

| Finding | Correction | Evidence |
| --- | --- | --- |
| macOS deleted-directory metadata did not expose the Linux unlink signal. | Observe the held directory with a native unlink event and latch deletion evidence for retry. | Release gate reproduced the failure; corrected native gate passed 23 trust tests, one unlink test, and 133 chat tests. |
| Windows pending deletion can be cancelled, including when the link count is zero. | Pending deletion never authorizes chat deletion; legacy unversioned confirmation is ignored. | Native Windows cancellation and rename regression. |
| Retained Windows directory handles prevented rename and deletion. | Observation handles request attribute access and explicitly permit read, write, and delete sharing. | Native Windows trust suite. |
| Delayed cloud responses could recreate deleted chats. | Atomic, account-scoped SQLite deletion fences reject later mutations; frontend fences filter views, lists, and rollbacks. | Delayed HTTP response, transaction rollback, mutation-family, and store tests. |
| A cleanup timeout could drop unfinished parked-run persistence. | Teardown has independent task ownership; timeout ends only the wait. | Real tool-run finalization regression. |
| Approval held the cleanup lock while executing a tool, delaying Stop. | Release admission ownership before execution, signal Stop before waiting, and finalize cancelled runs across all pause paths. | Production Stop and slow-tool cancellation regressions; 135 chat tests passed. |
| Approval resumption used the selected chat instead of the run owner. | Resume and cancellation verify the owning account and conversation. | Selection and account-switch regressions. |
| Partial cleanup could leave a deleted chat displayed. | Emit removal after durable local deletion, before unbinding; retain the binding if delivery fails. | Shell test exercises a removal event followed by cleanup failure. |

## Important

- Manual chat deletion and cloud-not-found handling now share account-pinned removal. Both stop associated execution and preserve unrelated active chats.
- The frontend waits for event registration before requesting project cleanup. Revalidation runs on startup, window focus, and visibility restoration. It does not introduce a browser polling timer.
- Linux cleanup uses a zero link count on the retained directory; macOS uses its explicit unlink notification. Both require observed evidence or a durable prior confirmation. Rename, move, Trash, disconnection, permission failure, and first observation of a missing path do not authorize deletion.
- Automatic OS-folder cleanup is not enabled on Windows. Manual project removal preserves chats; chats can be deleted separately.
- Chat projection schema version advances to 3 with `0012_chat_deletions.sql`. Existing encrypted chat data is preserved.

## Suggestions

Require the native project and chat tests in the release workflow before packaging. The release workflow now enforces this on both supported platforms. Local Linux tests do not establish native macOS behavior.

## Nits

None.

## Praise

The final implementation separates irreversible deletion evidence from temporary unavailability and tests delayed responses against the real projection store. Account and conversation ownership remain explicit throughout cleanup.

## Open questions

None required for this release. A future Windows automatic-cleanup design needs proof of completed deletion; path absence and pending deletion are insufficient.

## Validation

- Full frontend gate: 1,674 tests; formatting, type checking, lint, fonts, and release-script checks passed.
- Native Windows and Linux project trust suites: 23 tests each passed.
- Final Rust workspace tests passed, including 1,630 native library tests and all 11 cleanup regressions; one existing test is ignored. Workspace Clippy passed with warnings denied.
- Final cancellation follow-up: all 135 chat tests passed, including immediate Stop and pending-sibling teardown.
- Generated contracts: 28 snapshot tests passed.
- Final full visual suite: 157 tests passed twice without snapshot updates. Earlier runs exposed incomplete native-event mocks and were interrupted by development-server reloads during edits/builds; those runs were not counted as release evidence. Native macOS execution remains a release-workflow gate.
- Strict OpenSpec validation and dependency policy checks passed.
