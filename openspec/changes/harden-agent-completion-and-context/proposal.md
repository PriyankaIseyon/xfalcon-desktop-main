# Change: An agent finishes multi-file work, and long chats keep working on every model

## Why

A person asks the assistant to change several files in a project. The run often
stops after some of them, with no error. The causes are in this app:

- A model reply cut off by its output limit was never detected. A half-written
  tool call failed as "unexpected response", or its broken arguments were sent
  back with no hint to write less. `FinishReason::Length` existed and nothing
  produced it.
- Nothing checked that the work was done, and nothing told the model to list
  its targets or to write one file per call.
- A failed write said "2 of 5 completed" but not which two. Sending the same
  plan again was refused because the files that had landed "had nothing to
  write".

Long conversations have a second problem. The guard that keeps a request
inside the context window counted one byte as one token, so it fired three to
four times too early. Only one model family had compaction at all; every other
model failed the run once the estimate was exceeded, and the conversation
stayed stuck. A provider saying "the prompt is too long" ended the run.

## What Changes

- Every provider adapter reports why a turn stopped. A cut-off tool call is
  discarded and never run. The run answers it with a notice and retries, at
  most three times in a row.
- A run that ends its reply with checklist items open is sent back to work, at
  most three times per run. The model gets a `project.update_plan` checklist
  tool and prompt guidance to list targets and write one file per call.
- A write result lists every operation with its path. A plan with several bad
  operations reports all of them at once. A modification that already matches is
  left out, not an error. A partly applied plan names what landed and what did
  not, and its error is marked retryable.
- The model cannot write credentials, key material, `.git` or `.xfalcon`.
  `.env.example` and its kin are ordinary files.
- Paths are stored in one canonical spelling, so `a/./b.html` and `b.html/` are
  the same target. A whole-file replace is allowed only for a file one read
  returns in full. Line endings of a CRLF file survive a model's edit and a
  viewer save.
- The token estimate uses about three bytes per token, charges an image its
  documented ceiling, and is anchored on the provider's own count after every
  response. A provider's "prompt too long" is recognised on every adapter and
  answered by compacting harder, at most twice.
- A model without native compaction has its old tool results stubbed, and then
  its oldest history summarized by the same model. A compaction that cannot be
  replayed as a provider block is shown as its text instead of failing the run.
- Thinking blocks and attachments survive the snapshot reload between tool
  rounds.
- The viewer keeps unsaved edits across file and tab switches, and replaces an
  unedited open file when it changes on disk.

## Impact

- Capabilities: `agent-run-completion`, `project-agent-writes`,
  `client-context-compaction` (all new).
- `ProviderStreamEvent` gains `Stopped` and `PromptSize`.
  `project.propose_patch` results drop `unified_diff`, `risk`,
  `requires_second_confirmation` and `expires_at_ms` and gain per-operation
  paths. Conversations stored before this change still render.
- No new Tauri command, contract, or permission. No DA change.
- Not changed, on purpose: Windows reparse-point handling (a containment control
  that needs testing on Windows) and the native compaction wire.
