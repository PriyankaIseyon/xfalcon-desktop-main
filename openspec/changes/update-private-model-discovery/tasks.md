## Verification

- [x] URL/key-first discovery and explicit/manual model selection pass rendered Settings tests.
- [x] Missing metadata, invalid limits, unsupported sampling, and edit/clear flows pass rendered Settings tests.
- [x] Native discovery retains unknown metadata as null; rendered Settings tests prove labeled suggestions, exact reported values including zero, partial metadata, and preserved saved settings.
- [x] Shared native/frontend budget checks prove exact reserve/output-cap boundaries, immediate invalid-prefill errors, and recovery after correcting either limit.
- [x] Rendered Settings tests prove explicit error dismissal, automatic correction cleanup, complete replacement-key entry, rapid-click guards, and ignored save results after an account change.
- [x] Native service and rendered Settings tests prove URL/key/upstream-ID duplicate detection independent of display name and manual entry, with exact-key binding and update-row exclusion.
- [x] Actual DA catalog failures preserve pre-mutation error handling, including sign-in guidance, and rendered add/edit tests prove settings remain available for retry.
- [x] Cancellation and account changes cannot apply stale results in rendered Settings tests.
- [x] Guarded discovery HTTP tests prove exact IDs, bounded responses, safe failures, public-address rules, and redirect refusal.
- [x] DA wire tests prove explicit model settings and partial updates without changing the pinned DA contract.
- [x] Account-local storage and real provider-wire tests prove sampling persistence, isolation, clearing, and omitted defaults.
- [x] Exact company Qwen profile matching passes native regression tests.
- [x] Real provider HTTP tests prove completed nonempty greetings, bounded requests, safe failures, and rejection of partial or timed-out responses.
- [x] Native service tests prove failed connection checks cannot mutate DA or local state and stored-key checks use the exact model binding.
- [x] Rendered Settings tests prove pending checks block duplicate saves and test failures preserve the editor for correction.
- [x] Generated contracts and native command admission/permissions pass verification.
- [x] Frontend/native checks and browser layout/interaction tests pass.

## Evidence

- `bash scripts/gate.sh --frontend`: passed, including 144 Vitest files and 2,067 tests. `CustomModelsGroup.test.tsx` covers 39 rendered form cases, including duplicate selection/manual entry, key changes, labeled missing-value suggestions, exact reported metadata, partial limits, unsupported sampling in both entry paths, add/edit preflight failure retention, error dismissal/correction, full-key retry, budget boundaries, and account/rapid-click guards.
- `npm run build`: passed.
- `npx playwright test tests/visual/settings.spec.ts tests/visual/private-models.spec.ts --project=chromium`: 10 passed against the production preview. Updated dialog baselines were visually inspected before the final comparison run.
- `bash scripts/gate.sh --rust`: passed on Windows with default parallel tests, including formatting, Clippy, 2,250 passing tests, 24 ignored tests, and dependency policy. Native tests include guarded discovery, actual HTTP greeting streams, DA wire mapping, storage, generated contracts, command parity, shared policy exports, and exact context-budget boundaries.
- `chat::service::custom_binding_tests` passes all nine actual-service cases. Duplicate checks use WireMock catalog and exact-key resolve routes. Creation and colliding updates stop before inference/model mutation; editing the same row excludes itself. Failed greeting or catalog checks issue only reads and preserve local sampling. Catalog 401 preserves session-expiry guidance.
- The release audit uses the delivered staging v0.5.32 commit `a4efd89` as its baseline. It reproduced a parallel file-edit failure caused by repeated SQLite WAL initialization. Concurrent first-open/proposal tests, durability-profile checks, the two-dashboard paging/edit/readback regression, and the complete parallel suite pass with the shared store initialization fix.
- Runtime regressions prove that confirmed durable history writes advance the watchdog, oversized text fails before presentation/persistence, and corrupted tool streams execute no partial batch. A real HTTP two-call test preserves nullable and empty optional OpenAI chunk fields while requiring complete final call IDs and names. Oversized project CSV charts are refused without truncating source data; rendered viewer/Canvas tests cover the bounds and complete category aggregation.
- Strict OpenSpec validation passes for `update-private-model-discovery`.
- The Windows staging app was rebuilt with fresh production assets and relaunched. Startup and its responsive window were verified. Provider-wire validation uses HTTP test servers; no new live provider inference claim is made.
