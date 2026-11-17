# provider-failure-recovery Specification

## Purpose

Which provider failures are worth a retry, and the rule that a failure whose cause
does not change with time is reported once, immediately, in words that name the fix.

## Scope

| File                                  | What it owns here                             |
| ------------------------------------- | --------------------------------------------- |
| `src-tauri/src/providers/http.rs:152` | `classify_response`                           |
| `src-tauri/src/providers/http.rs:165` | `classify_status_code` — 402, 413             |
| `src-tauri/src/providers/http.rs:199` | `classify_transport_error` — TLS verification |
| `src-tauri/src/providers/retry.rs`    | the backoff loop this classification gates    |

## Boundaries

- **What the reader sees when a run dies** belongs to `chat-run-recovery`, which
  reads the classification this capability produces.
- **429 and 503 backoff timing itself** belongs to `provider-adapters`.

## Requirements

### Requirement: A deterministic provider failure is never retried

The system MUST NOT retry a provider failure whose cause is unchanged by waiting.
Payment required, payload too large, and TLS certificate verification failure SHALL
each be reported on the first attempt.

Retrying these is misleading in two directions at once. The reader waits through
backoffs for an outcome already decided, and the failure finally shown is the one the
retry loop gives up with rather than the one that occurred. A deterministic failure
carries its own remedy — top up the account, send less, fix the certificate chain —
and none of those remedies are "wait".

#### Scenario: Payment required

- **WHEN** a provider answers with a payment-required status
- **THEN** the turn fails immediately as payment required, with no further attempt

#### Scenario: Payload too large

- **WHEN** a provider answers with a payload-too-large status
- **THEN** the turn fails immediately as payload too large, with no further attempt

#### Scenario: Certificate verification failure

- **WHEN** the TLS handshake fails certificate verification
- **THEN** the turn fails immediately as a TLS verification failure, because the next
  handshake is identical

#### Scenario: Transient failures still retry

- **WHEN** a provider answers rate-limited or unavailable
- **THEN** the turn is still retried under the existing backoff

### Requirement: A failure names its own remedy

The system MUST describe a provider failure by what the reader can do about it. The
user-facing string for each deterministic failure SHALL name the condition that
produced it rather than a generic unavailability.

A managed desktop fleet is the case that makes this concrete: behind a
TLS-inspecting proxy every provider call fails identically, and a generic
"unavailable" sends the reader to a status page while the cause is on their own
machine. The message is the only diagnostic a desktop reader has.

#### Scenario: TLS failure is distinguishable from an outage

- **WHEN** a TLS verification failure is shown
- **THEN** its text identifies certificate verification as the cause and is not the
  text an unreachable provider produces

#### Scenario: Payment required is distinguishable from a rate limit

- **WHEN** a payment-required failure is shown
- **THEN** its text identifies the account rather than the request rate

## Provenance

From the delta of `classify-provider-failures`.
