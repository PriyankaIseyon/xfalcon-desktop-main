# Change: Classify provider failures by the recovery they need

## Why

`ProviderError` has five variants and the status table maps everything it does
not name to `Unavailable`, which `retry.rs` retries three times. Two of those
unnamed statuses are deterministic — 402 payment required and 413 payload too
large — so a run that cannot succeed spends two backoffs before saying so, and
then reports "unavailable", which is not what went wrong. A TLS verification
failure is worse: it reproduces the identical handshake on every attempt, it is
the single most likely failure on a managed fleet behind an inspecting proxy,
and the user is told the provider is down.

Audit source: Hermes Agent `agent/error_classifier.py`, which classifies each
failure by the recovery it needs rather than by its status family.

## What Changes

- **BREAKING** `ProviderError` gains `PaymentRequired`, `PayloadTooLarge`, and
  `TlsVerificationFailed`. Every `match` over it is exhaustive, so every arm is
  updated in this change.
- 402 and 413 classify to the new deterministic variants instead of falling
  through to `Unavailable`.
- Transport errors are classified before they collapse to `Unavailable`: a
  certificate verification failure becomes `TlsVerificationFailed`.
- `delay_before_retry` returns `None` for all three new variants — a
  deterministic failure is reported, never retried.
- `Display` strings name the fix, not the symptom.

## Impact

- Affected specs: `provider-failure-recovery` (new)
- Affected code: `src-tauri/src/providers/adapter.rs`,
  `src-tauri/src/providers/http.rs`, `src-tauri/src/providers/retry.rs`,
  `src-tauri/src/providers/openai_compat.rs`, and every call site matching on
  `ProviderError`
