# Implementation Evidence

## Provider contract review

Verified on 2026-09-01 against primary documentation.

- OpenRouter Anthropic Messages API: <https://openrouter.ai/docs/api/api-reference/anthropic-messages/create-messages>
  - Confirms `POST /api/v1/messages`.
  - Confirms `context_management`, `provider`, `session_id`, `thinking`, tools, streaming, and usage fields.
  - Confirms a 256-character maximum for `session_id`.
- OpenRouter provider routing: <https://openrouter.ai/docs/guides/routing/provider-selection>
  - Confirms `provider.require_parameters` for rejecting routes that cannot honor required request parameters.
- OpenRouter prompt caching: <https://openrouter.ai/docs/guides/best-practices/prompt-caching>
  - Confirms stable `session_id` use for provider stickiness and prompt-cache continuity.
- Anthropic server-side compaction: <https://platform.claude.com/docs/en/build-with-claude/compaction>
  - Confirms server-side compaction as the recommended long-running agent strategy.
  - Confirms beta `compact-2026-01-12` and edit type `compact_20260112`.
  - Confirms that the complete compaction block must be appended on later requests.
  - Documents that a compaction sampling step can call a declared tool and return a null summary. The request uses replacement instructions that require a summary-only response and prohibit tool calls during that step.
- Anthropic server tools: <https://platform.claude.com/docs/en/agents-and-tools/tool-use/server-tools>
  - Confirms that `pause_turn` continuation must pass the prior response back without changing its ordered blocks or tool set.
- Anthropic pricing: <https://docs.anthropic.com/en/docs/about-claude/pricing>
  - Confirms that long-context input can use a higher price tier.
  - Confirms that cache read and cache creation tokens count toward the long-context input threshold.
- Anthropic compaction usage: <https://platform.claude.com/docs/en/build-with-claude/compaction#understanding-usage>
  - Confirms that top-level usage excludes the compaction sampling iteration.
  - Confirms that billed totals must sum bounded `usage.iterations` entries.

The OpenRouter reference exposes the required request surface. It does not prove that every routed model and provider accepts the current compaction beta. The implementation therefore keeps the route-specific, fail-closed capability probe. A safe staging probe with the deployed credential and selected model remains a release gate.

## Local implementation evidence

Verified on 2026-09-01 across the paired feature branches.

- DA branch: `codex/add-durable-context-compaction`.
  - Full test suite: 5,725 passed, 2 skipped, 207 deselected.
  - Focused contract and security suite: 361 passed.
  - Context endpoint suite: 24 passed.
  - Ruff lint and format checks passed.
  - Bandit passed on all changed Python security surfaces.
  - `pglast` parsed all 12 SQL bundle files.
  - Full OpenAPI SHA-256: `ef8db76e21bdd8043e206ecfeaf12150010c54508179b148538a0d028c7241da`.
- XF branch: `codex/add-provider-native-context-compaction`.
  - Full Rust library suite: 1,553 passed, 0 failed, 1 ignored.
  - Rust clippy with warnings denied passed.
  - Progressive turn-driver tests: 36 passed.
  - Context and capacity tests: 27 passed.
  - Chat tool-run tests: 8 passed.
  - Exact-payload recovery WireMock tests: 8 passed.
  - Exact-range registry, ToolLoop, and WireMock tests: 8 passed.
  - Frontend Vitest suite: 1,584 passed across 114 files.
  - Frontend lint, typecheck, production build, styles, tooltips, fonts, and release-script tests passed.
  - `git diff --check` passed in both repositories.

The verified local path now keeps the authoritative DA snapshot lossless. It hard-admits the complete provider request. It progressively compacts oversized legacy history through complete tool boundaries. It commits each block through DA before continuation. It persists exact JSON tool results up to 8 MiB. It exposes only admitted ready payloads through the bounded internal range tool. XF uses typed DA HTTP endpoints and makes no direct DA database call.

## Unavailable and release-only evidence

- OpenSpec CLI validation is unavailable because the CLI is not installed on this host.
- Docker and PostgreSQL are unavailable. Fresh-install, upgrade, run-twice, live RLS race, and object-storage fault harnesses remain unchecked.
- The DA feature is not deployed to staging. The XF contract pin is the reviewed local branch artifact, not a staging deployment claim.
- The live OpenRouter route probe, live DA staging matrix, production-shaped latency and memory benchmarks, long-context quality evals, soak window, alarms, and release deployment remain unchecked.
