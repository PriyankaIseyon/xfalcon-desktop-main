## 1. Compatibility precondition

- [x] 1.1 Stop writing `secret_owner` and `envelope_version`; retain `device_key_id`.
- [x] 1.2 Reduce legacy device-envelope compatibility to prefix and device-key identity.
- [x] 1.3 Prove the metadata shape and compatibility rules with tests.

## 2. DA-owned secret cutover

- [x] 2.1 Submit cleartext through `secret` on create and `PUT /{id}/secret` on replacement.
- [x] 2.2 Verify returned row identity and DA ownership without comparing ciphertext.
- [x] 2.3 Resolve the selected enabled default-first row through `POST /{id}:resolve`.
- [x] 2.4 Keep cleartext in zeroizing values and map DA failures to actionable credential errors.
- [x] 2.5 Reduce `NeedsReentry` to rows that genuinely require legacy recovery.

## 3. Migration

- [x] 3.1 Migrate matching `xfdc:v1` rows in the authenticated startup path.
- [x] 3.2 Skip DA-owned and unreadable rows, isolate row failures, and never log secrets.
- [x] 3.3 Prove migration idempotency and partial-failure behavior.

## 4. Contract and validation

- [x] 4.1 Advance the immutable DA OpenAPI pin after staging exposes all reviewed operations.
      Live staging exposed both reviewed routes and all three secret DTOs at
      SHA-256 `39946fd3a1325974088e40f18c1cd13c8974c10fe4d8f8abfb6013e8767f0b57`.
      `clients::da::tests::committed_contract_matches_the_approved_immutable_pin`
      verifies the committed bytes and provenance.
- [ ] 4.2 Run targeted tests and `bash scripts/gate.sh`.
      Targeted DA-client, credential, and chat-service tests pass. The full gate's
      typecheck, lint, contracts, 1,612 frontend tests, Windows Clippy, 1,594 Rust
      tests, and `cargo deny` pass. This remains unchecked because the repository
      format gate still reports 10 unrelated files that were already unformatted
      on `origin/main`.
- [x] 4.3 Inspect the final diff for stale device-sealing provider paths and secret leakage.
- [ ] 4.4 Run an authenticated staging smoke test and record list-plus-resolve latency.
