# Tasks: Add Provider-Native Context Compaction

Do not implement this change before proposal approval and DA staging deployment. A checkbox is a verified claim. Mark it only when an automated test or an approved harness proves the stated invariant.

## 0. Approval, Dependencies, and Baseline

- [x] 0.1 Approve this proposal, design, and all five delta specs as one change.
- [x] 0.2 Approve and implement data-analyst change `add-durable-context-compaction` first.
- [ ] 0.3 Deploy the DA change to staging, regenerate its OpenAPI artifact, and record the deployed artifact SHA.
- [ ] 0.4 Pin the deployed DA artifact in XF Desktop before generating any client types.
- [x] 0.5 Reverify current official OpenRouter and Anthropic Messages, compaction, beta-header, reasoning, caching, session, usage, and SSE contracts. Record source URLs and verification date in implementation evidence.
- [ ] 0.6 Run a safe staging route probe with the actual supported OpenRouter model and record request acceptance, response shape, routed provider identity when exposed, and required beta behavior without recording content or credentials.
- [ ] 0.7 Inventory every provider selection and inference call site. Confirm which direct-provider adapters remain for non-agent duties and which old OpenRouter Chat Completions paths must be deleted.
- [ ] 0.8 Inventory every constructor and restore path for `ContextMessage`, tool results, reasoning details, tool calls, images, documents, and history pages.
- [ ] 0.9 Inventory every frontend, Rust contract, IPC, local projection, diagnostic, and test consumer of tool-result `truncated` state.
- [ ] 0.10 Capture baseline provider request fixtures, context byte and message limits, DA calls, long-turn latency, peak memory, token totals, cache totals, and existing full test results.
- [ ] 0.11 Build a cross-repository contract matrix that maps every required DA field, route, status, error code, idempotency behavior, and lifecycle operation to its desktop consumer and test.

## 1. Protocol and State-Machine Freeze

- [ ] 1.1 Define the exact supported OpenRouter Anthropic Messages request schema and headers from the verified staging contract.
- [ ] 1.2 Define the exact SSE event and content-block assembly state machine, including unknown event handling, completion, cancellation, and malformed fragments.
- [ ] 1.3 Define the lossless ordered provider-content representation for text, thinking or reasoning, tool use, tool result, attachment, and compaction blocks.
- [ ] 1.4 Define stable provider request IDs and idempotency identities for initial, tool continuation, compaction-only, resumed, and bootstrap requests.
- [ ] 1.5 Define the route-capability cache key, success evidence, failure evidence, 24-hour expiry, and invalidation triggers.
- [ ] 1.6 Define the versioned context-manifest canonicalization. Pin test vectors shared with DA for empty, Unicode, reordered maps, ordered tools, skills, documents, and reasoning settings.
- [ ] 1.7 Define exact provider-block pass-through and digest fixtures shared with DA.
- [ ] 1.8 Define the context loading, bootstrap compaction, normal compaction, activation barrier, continuation, conflict, retry, cancellation, and terminal state transition table.
- [ ] 1.9 Define tool payload prepare, upload, finalize, bind, range read, cleanup, and reconciliation transition tables.
- [ ] 1.10 Define preview schemas and deterministic algorithms for JSON, text, CSV, table, and binary results.
- [ ] 1.11 Define the provider overhead fixture corpus and hard admission estimator. Prove the estimator never undercounts the fixture set.
- [ ] 1.12 Complete a threat review for prompt injection, route downgrade, block confusion, digest substitution, cursor tamper, cross-conversation payload access, signed URL leakage, temporary bytes, and usage duplication.

## 2. DA Contract Generation and Wrapper

- [ ] 2.1 Regenerate `src-tauri/src/clients/da/generated.rs` from the pinned deployed DA artifact. Never edit the generated file by hand.
- [ ] 2.2 Inspect generated context, compaction, payload, model-capacity, range, usage, cursor, quota, and error models against the approved DA spec.
- [ ] 2.3 Add typed DA wrapper methods for first and later context pages.
- [ ] 2.4 Add typed wrapper methods for compaction activation, compaction reconciliation, and active-pointer invalidation where exposed.
- [ ] 2.5 Add typed wrapper methods for payload prepare, local or signed upload, finalize, metadata reconciliation, bind-through-message, and exact range retrieval.
- [ ] 2.6 Map every declared HTTP status to a distinct desktop error where caller behavior differs. Preserve `not_found`, conflict, stale snapshot, quota, storage unavailable, integrity, oversized, unauthorized, and invalid response.
- [ ] 2.7 Apply response byte limits and streaming limits without buffering exact payload bytes through JSON helpers.
- [ ] 2.8 Validate every DA response field as untrusted data before it enters provider context or local state.
- [ ] 2.9 Add WireMock tests that use the real generated request and response types for every success and error branch.
- [ ] 2.10 Add artifact SHA and operation-coverage tests that fail on DA contract drift.

## 3. Model Selection and Capacity

- [x] 3.1 Carry `max_output_tokens` from the DA catalog through model selection and run configuration with checked integer conversion.
- [x] 3.2 Require OpenRouter provider, Claude Sonnet or Opus family, at least 1,000,000 total tokens, and valid output capacity in the primary agent path.
- [x] 3.3 Add typed setup and model-capacity errors. Remove the 200,000-token and generic 4,096-output fallbacks from the supported path.
- [x] 3.4 Implement `hard_input_tokens = context_length - max_output_tokens - 16,384` with checked arithmetic.
- [x] 3.5 Implement exact-usage anchoring and worst-case unmeasured byte plus protocol-overhead accounting.
- [x] 3.6 Remove the fixed 75% four-bytes-per-token prose budget from all supported production call sites.
- [x] 3.7 Remove the fixed 512-message ceiling from all supported production call sites.
- [x] 3.8 Add boundary tests for exact 1M capacity, lower capacity, equal output and context, arithmetic overflow, large tool schemas, Unicode, attachments, and one indivisible oversized item.
- [x] 3.9 Prove message count alone does not evict a valid context and token safety still rejects an unsafe request.

## 4. OpenRouter Anthropic Messages Adapter

- [ ] 4.1 Add the compiled OpenRouter `/api/v1/messages` endpoint and preserve fixed attribution and secret-header handling.
- [ ] 4.2 Serialize system, messages, ordered content blocks, tools, selected model, streaming, catalog output limit, current reasoning shape, context management, provider routing, and stable session ID.
- [ ] 4.3 Implement deterministic same-role merge that keeps all parallel tool results in one valid user message without block reordering.
- [ ] 4.4 Implement stable `session_id` from the canonical conversation UUID on the first request and every retry or continuation.
- [ ] 4.5 Implement `provider.require_parameters` for every semantically required request parameter.
- [ ] 4.6 Implement cache breakpoints on the current supported stable prefix without mutating opaque blocks or exceeding provider limits.
- [ ] 4.7 Implement the exact verified compaction beta header and native edit with 150,000 trigger and paused continuation.
- [ ] 4.8 Implement route capability probe and bounded non-secret capability cache.
- [ ] 4.9 Parse text, thinking or reasoning, tool-use, compaction, usage, stop, error, and completion SSE events through complete block assembly.
- [ ] 4.10 Preserve unknown nested fields in supported opaque blocks and reject unsupported top-level block or protocol versions.
- [ ] 4.11 Expand provider usage events for input, output, cache read, cache write, reasoning, provider request ID, and bounded iterations.
- [ ] 4.12 Add typed native-compaction unsupported, malformed compaction, hard context overflow, and existing provider error classification.
- [ ] 4.13 Delete the OpenRouter Chat Completions agent path and every call site in the same verified pass.
- [ ] 4.14 Prove no caller-supplied provider origin, signed DA URL, provider block, or secret can enter logs or errors.
- [ ] 4.15 Add exact JSON and SSE fixture tests for plain text, tools, parallel tools, signed thinking, cache, one compaction, two compactions, usage iterations, malformed fragments, cancellation, and every error status.

## 5. Context Snapshot Loader and Fingerprint

- [ ] 5.1 Add a context snapshot loader that starts at page one and follows cursors through explicit completion.
- [ ] 5.2 Validate page conversation, pointer, revision, frozen head, sequence order, uniqueness, cursor continuity, compaction descriptor, payload descriptors, and response bounds.
- [ ] 5.3 Restart the whole snapshot after `context_snapshot_stale` with a bounded conflict retry count.
- [ ] 5.4 Reject duplicate, missing, mismatched, or incomplete pages before any provider call.
- [ ] 5.5 Replace `list_messages(limit=200)` and conversation bootstrap in every inference-history call site.
- [ ] 5.6 Preserve those bounded routes for UI uses that still own them. Add a test that inference cannot call them.
- [ ] 5.7 Build the canonical context manifest from model, provider protocol, system prompt, agent, skills, tools, MCP identity, reasoning, documents, and instruction version.
- [ ] 5.8 Validate the active compaction's protocol, block format, model, fingerprint, digest, boundary, and lineage before use.
- [ ] 5.9 Rebuild complete post-boundary messages with equivalent live and restored tool-result representations.
- [ ] 5.10 Preserve provider-valid tool pairing and ordered opaque blocks.
- [ ] 5.11 Prove offline local history can render but cannot authorize provider inference, compaction, or payload retrieval.

## 6. Native Compaction Driver

- [ ] 6.1 Extend the provider-neutral event port with a complete opaque compaction event and expanded usage without leaking it into frontend IPC.
- [ ] 6.2 Add internal run phases for context loading, long-context preparation, compaction persistence, compaction resume, and payload persistence.
- [ ] 6.3 Capture request parent, revision, frozen head, source boundary, source digest, fingerprint, model, protocol, and stable IDs before provider send.
- [ ] 6.4 Stop normal continuation when `CompactionReady` arrives.
- [ ] 6.5 Activate through DA with one stable compaction ID and idempotency key.
- [ ] 6.6 Resume exactly once only after DA confirms that record as active.
- [ ] 6.7 Reconcile ambiguous activation by ID before retry.
- [ ] 6.8 On context conflict, discard the block and reload authoritative context. Restart only after side-effect and message idempotency prove safety.
- [ ] 6.9 Support several ordered compactions in one logical turn with correct parent lineage.
- [ ] 6.10 Record each compaction-only and continuation request's usage exactly once.
- [ ] 6.11 Integrate cancellation before provider response, during block assembly, during activation, during reconciliation, and before continuation.
- [ ] 6.12 Prove no terminal public run state publishes while an ambiguous activation is unresolved.
- [ ] 6.13 Add crash-recovery tests before send, after block completion, before activation, after activation before response, before continuation, and after continuation start.

## 7. Progressive Bootstrap Compaction

- [x] 7.1 Detect a complete uncheckpointed snapshot that exceeds the safe input limit without dropping history.
- [x] 7.2 Select the largest oldest prefix that fits the hard estimator and ends at a complete provider tool group.
- [x] 7.3 Issue a compaction-only request that cannot execute historical tool calls or answer the user turn.
- [x] 7.4 Activate, reload, and advance until the full retained history and current turn fit.
- [x] 7.5 Keep every step idempotent, durable, cancellable between requests, and resumable from the last active DA record.
- [ ] 7.6 Expose a bounded user state that explains billed long-conversation preparation without leaking content or internal protocol text.
- [ ] 7.7 Add tests for empty history, exact-fit history, two and many steps, parallel tool boundaries, rewind during bootstrap, cancellation, provider failure, DA conflict, crash, and resume.
- [x] 7.8 Prove historical tool calls and side effects never execute during bootstrap.

## 8. Exact Tool Payload Domain

- [ ] 8.1 Define a runtime exact-result value that preserves media type, format version, encoding, exact bytes, and provenance up to 8 MiB.
- [ ] 8.2 Remove the current 64 KiB structured-result discard as the source-of-truth path while retaining bounded projections where required.
- [ ] 8.3 Generate stable payload IDs from invocation identity and result version so retries reuse one logical resource.
- [ ] 8.4 Implement DA prepare for inline and object-backed payloads with declared length, SHA-256, media, format, and identity.
- [ ] 8.5 Implement bounded local or signed upload without exposing a signed URL outside the storage client.
- [ ] 8.6 Implement finalize and metadata reconciliation after ambiguous outcomes.
- [ ] 8.7 Bind the ready payload, database tool call, and tool message atomically through the generated DA contract.
- [ ] 8.8 Stop provider continuation until every required payload is ready and bound.
- [ ] 8.9 Handle quota, storage unavailable, integrity, unauthorized, not found, conflict, and oversized failures explicitly.
- [ ] 8.10 Add temporary-file or memory-spool bounds and integrate cancellation, account change, clear-local-data, and cleanup failure reporting.
- [ ] 8.11 Prove exact bytes and digest match after inline and object-backed round trips at every boundary.
- [ ] 8.12 Add failure injection for prepare, upload, finalize, bind, message append, timeout, process exit, cleanup, and retry.

## 9. Structured Tool Preview

- [ ] 9.1 Define internal `ToolResultPreviewV1` and its maximum 64 KiB serialized size.
- [ ] 9.2 Implement deterministic text head-tail preview with UTF-8-safe cuts and omitted-byte count.
- [ ] 9.3 Implement deterministic JSON structural preview that remains valid JSON and reports omitted structure.
- [ ] 9.4 Implement deterministic CSV and table preview with columns, row count when known, head, tail, and omitted-row count.
- [ ] 9.5 Implement binary metadata-only preview.
- [ ] 9.6 Include payload ID, exact bytes, digest, media, format, strategy, and `complete` in every preview descriptor.
- [ ] 9.7 Remove `MAX_TOOL_RESULT_CONTEXT_BYTES = 4_096` and audit every sibling live, failure, denied, restored, internal, MCP, and project-tool construction path for equivalent clipping.
- [ ] 9.8 Replace or redefine invocation `truncated` semantics so preview completeness and durable exact-payload availability cannot be confused.
- [ ] 9.9 Keep frontend projection bounded and inert. Add typed availability and explicit retrieval or export only if product review approves that surface.
- [ ] 9.10 Add boundary tests at 4 KiB, 64 KiB, multibyte cuts, deeply nested JSON, very wide tables, unknown row count, empty result, and 8 MiB exact source.

## 10. Internal Exact-Range Tool

- [x] 10.1 Register `__internal__read_tool_result` through the real tool registry and effective run allowlist.
- [x] 10.2 Admit it only when the active context contains at least one ready payload descriptor.
- [x] 10.3 Validate payload ID against the current conversation, run, admitted descriptor set, length, digest, and media identity before DA retrieval.
- [x] 10.4 Validate offset, overflow, end, and 64 KiB maximum range length.
- [x] 10.5 Retrieve through the typed DA client without returning signed URL, object key, tenant, host path, or credential.
- [x] 10.6 Return exact bounds, total bytes, media, digest, and continuation state as untrusted tool data.
- [x] 10.7 Keep the tool read-only, approval-free, and unable to modify admission or policy.
- [ ] 10.8 Add real tool-loop tests for first, middle, last, overlapping, repeated, invalid, unadmitted, cross-conversation, stale, and integrity-failed ranges.
- [x] 10.9 Add prompt-injection tests that prove payload text cannot grant approval, change tools, modify system authority, or admit another payload.

## 11. Elision and Context Builder Cutover

- [ ] 11.1 Make native compaction the normal quality and space-recovery path after route activation.
- [ ] 11.2 Limit emergency elision to progressive-bootstrap prefix selection or the documented pre-activation hard-safety case.
- [ ] 11.3 Make emergency preview reduction retain descriptor, schema or columns, counts, head, tail, digest, and explicit omission state.
- [ ] 11.4 Prove each reduction strictly lowers the hard estimate and an already minimal preview is never selected again.
- [ ] 11.5 Preserve complete tool groups during any prefix selection or last-resort exclusion.
- [ ] 11.6 Prevent a normal user-answer request from using an incomplete emergency-elided raw prefix. Require immediate compaction-only use.
- [ ] 11.7 Keep exact DA payloads unchanged by every local context reduction.
- [ ] 11.8 Remove obsolete builder fields, counters, comments, and tests only after every caller uses the new contract.

## 12. UI, IPC, and Local Projection

- [ ] 12.1 Decide whether existing public run states can represent context preparation without a contract change. Prefer existing states unless user action needs a distinct stable state.
- [ ] 12.2 If a frontend contract changes, edit `crates/contracts/`, regenerate `src/lib/generated/contracts.ts`, and type every Tauri invocation against the generated contract.
- [ ] 12.3 Keep full provider blocks, exact payload bytes, signed URLs, and reasoning signatures out of IPC and module-store snapshots.
- [ ] 12.4 Add bounded user copy for unsupported route, context preparation, storage failure, quota, integrity, conflict, item too large, and upgrade-required restore.
- [ ] 12.5 If full-result retrieval or export is exposed, add one dedicated typed command, capability registration, origin validation, destination policy, cancellation, and bounded progress.
- [ ] 12.6 Mount component tests inside the real chat container and use the real event admission path.
- [ ] 12.7 Capture and compare the full optimistic store snapshot on every new rollback test.
- [ ] 12.8 Prove large-result conversation open and scroll remain inside existing UI performance budgets.

## 13. Security, Privacy, and Diagnostics

- [ ] 13.1 Extend threat-model documentation for native provider blocks, long-context prompt injection, exact payload retrieval, route downgrade, and temporary bytes.
- [ ] 13.2 Audit every `Debug`, error, log, trace, metric, audit, test failure, and diagnostic bundle path for provider blocks, reasoning signatures, tool bytes, previews, signed URLs, object keys, arguments, and credentials.
- [ ] 13.3 Add redaction tests that use sentinel secrets in each forbidden content class.
- [ ] 13.4 Validate every external URL against compiled provider or DA-origin policy. Never follow a signed URL to an unapproved origin.
- [ ] 13.5 Validate every size before allocation, every integer conversion, digest, enum, block depth, cursor, sequence, and resource identity.
- [ ] 13.6 Add memory-pressure tests for 8 MiB results, many 64 KiB ranges, long snapshot pages, and concurrent tool results.
- [ ] 13.7 Add diagnostics for safe route capability, context tokens, compaction count and latency, cache usage, conflicts, bootstrap steps, payload bytes, quota, storage, integrity, and retry.
- [ ] 13.8 Prove diagnostics contain no user or provider content with snapshot and property tests.
- [ ] 13.9 Run dependency, license, vulnerability, and secret scans. A new dependency needs a written reason that current dependencies cannot meet.

## 14. Real Consumer and Failure Verification

- [ ] 14.1 Test the real OpenRouter adapter against staging or a provider-faithful local SSE server with exact captured redacted fixtures.
- [ ] 14.2 Test the real DA generated client against live DA staging for context paging, activation, payload lifecycle, range reads, quota, RLS-hidden resources, and all errors.
- [ ] 14.3 Test the complete turn driver with real context builder, provider adapter port, DA wrapper, tool registry, journal, local projection, and store event path.
- [ ] 14.4 Test new conversation, existing short conversation, active compaction, long tail, legacy oversized history, rewind, hard truncation, restart, account switch, and clear-local-data.
- [ ] 14.5 Test one and several native compactions in one user turn, including tool use before and after each barrier.
- [ ] 14.6 Test route unsupported, malformed block, unknown nested field, stale activation, lost response, provider retry, payment, rate limit, outage, TLS, context overflow, and cancellation.
- [ ] 14.7 Test exact payload inline, object backed, maximum size, over limit, quota race, missing object, digest mismatch, expired URL, partial range, and repeated range.
- [ ] 14.8 Test crash injection at every durable boundary and prove restart converges without duplicate tool side effects, messages, compactions, payloads, usage, or UI events.
- [ ] 14.9 Test all existing sibling provider adapters and provider selection paths that remain in scope. Remove unreachable code instead of weakening tests.
- [ ] 14.10 Prove the old Chat Completions endpoint, 4 KiB cap, 512-message cap, 75% formula, and 200-message inference call have no production caller.

## 15. Performance, Quality, and Cost Gates

- [ ] 15.1 Measure first-context-load latency over 1, 10, and 100 DA pages and keep it within the approved run-start budget.
- [ ] 15.2 Measure provider request serialization and peak memory for maximum safe context and 8 MiB exact payload handling.
- [ ] 15.3 Measure 64 KiB range-read latency and repeated-range tool-loop overhead.
- [ ] 15.4 Measure automatic compaction trigger behavior, before and after tokens, time to first token, continuation latency, cache reads and writes, and billed usage.
- [ ] 15.5 Run long-context retrieval evals with critical BI facts at early, middle, late, tool-result, table-tail, and post-compaction positions.
- [ ] 15.6 Compare answer correctness before and after compaction. Block release on a material regression in approved eval thresholds.
- [ ] 15.7 Compare preview strategies for tables, JSON, and text. Prove range retrieval recovers facts omitted from the first preview.
- [ ] 15.8 Set and verify alarms for compaction failure, repeated conflicts, route incompatibility, bootstrap loops, payload integrity, quota, storage, and token-cost regression.

## 16. Repository Gates and Release

- [ ] 16.1 Run immediate Rust targeted tests after each provider, context, DA, tool-result, reducer, or IPC contract batch.
- [ ] 16.2 Run `npm run typecheck` and targeted unit tests immediately after any schema, reducer, generated TypeScript, or IPC change.
- [ ] 16.3 Run Rust formatting, clippy with repository flags, targeted tests, and the full Rust suite on the required Windows toolchain.
- [ ] 16.4 Run frontend lint, typecheck, build, generated-contract checks, all Vitest tests, and relevant real-container browser tests in WSL.
- [ ] 16.5 Run `git diff --check`, OpenSpec strict validation, contract snapshot checks, capability command-list checks, and repository security gates.
- [ ] 16.6 Review the final diff against every AGENTS architectural guardrail and every task in this file.
- [ ] 16.7 Deploy against the pinned DA staging artifact and run the full cross-repository end-to-end matrix.
- [ ] 16.8 Enable only for probed supported Sonnet and Opus routes. Observe a full soak window for restore, correctness, latency, cache, cost, conflict, payload, and integrity metrics.
- [ ] 16.9 Verify rollback refuses unreadable active state rather than silently using a partial transcript.
- [ ] 16.10 Update architecture, operator, provider, context, tool-result, recovery, privacy, and troubleshooting documentation.
- [ ] 16.11 Archive this OpenSpec change only when every checked item has attached automated or verified evidence.

## Resume Notes

Start at the first unchecked item. Read this proposal, design, all five delta specs, the approved DA `add-durable-context-compaction` change, `src/lib/generated/contracts.ts`, and `crates/contracts/` before editing. Regenerate DA types from the deployed pinned artifact. Do not hand-edit generated code. Do not restore any old inference limit as a hidden fallback. Do not continue a provider turn from an uncommitted compaction block or an unbacked truncated tool preview. Preserve complete rollback snapshots and provider-valid tool groups.
