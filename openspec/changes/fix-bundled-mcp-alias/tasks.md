## 1. Implementation

- [x] 1.1 Canonicalize registered server rows against the bundled endpoint.
- [x] 1.2 Apply the same exclusion to Settings, run-time servers, workspace
      search, and composer mentions.
- [x] 1.3 Reject create and update attempts that target the bundled endpoint.
- [x] 1.4 Replace the static connection claim with transport-only copy.
- [x] 1.5 Show the canonical bundled server as a read-only authenticated card.

## 2. Verification

- [x] 2.1 Add Rust tests for normalized endpoint identity, list/run exclusion,
      workspace summaries, and mutation rejection.
- [x] 2.2 Update the Settings consumer test for neutral transport copy.
- [x] 2.3 Run focused frontend and Rust tests, typecheck, formatting, and inspect
      the final diff.
- [x] 2.4 Update and verify the Settings visual baselines with the bundled card.
