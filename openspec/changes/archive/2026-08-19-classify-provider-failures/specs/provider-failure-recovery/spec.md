## ADDED Requirements

### Requirement: A deterministic provider failure is never retried

The system MUST NOT retry a provider failure whose cause is unchanged by
waiting. Payment required, payload too large, and TLS certificate verification
failure SHALL each be reported on the first attempt.

Retrying these is not merely wasteful, it is misleading in two directions at
once. The user waits through backoffs for an outcome that was already decided,
and the failure they are finally shown is the one the retry loop gives up with
rather than the one that actually occurred. A deterministic failure carries its
own remedy — top up the account, send less, fix the certificate chain — and none
of those remedies are "wait".

#### Scenario: Payment required

- **WHEN** a provider answers 402
- **THEN** the turn fails immediately as payment required
- **AND** no further attempt is made

#### Scenario: Payload too large

- **WHEN** a provider answers 413
- **THEN** the turn fails immediately as payload too large
- **AND** no further attempt is made

#### Scenario: Certificate verification failure

- **WHEN** the TLS handshake fails certificate verification
- **THEN** the turn fails immediately as a TLS verification failure
- **AND** no further attempt is made, because the next handshake is identical

#### Scenario: Transient failures still retry

- **WHEN** a provider answers 429 or 503
- **THEN** the turn is still retried under the existing backoff, because waiting
  is a real remedy for these

### Requirement: A failure names its own remedy

The system MUST describe a provider failure by what the user can do about it.
The user-facing string for each deterministic failure SHALL name the condition
that produced it rather than a generic unavailability.

A managed desktop fleet is the case that makes this concrete. Behind a
TLS-inspecting proxy every provider call fails identically, and "the provider is
unavailable" sends the user to a status page while the cause is on their own
machine. The message is the only diagnostic a desktop user has.

#### Scenario: TLS failure is distinguishable from an outage

- **WHEN** a TLS verification failure is shown to the user
- **THEN** its text identifies certificate verification as the cause
- **AND** it is not the same text an unreachable provider produces

#### Scenario: Payment required is distinguishable from a rate limit

- **WHEN** a payment-required failure is shown to the user
- **THEN** its text identifies the account rather than the request rate
