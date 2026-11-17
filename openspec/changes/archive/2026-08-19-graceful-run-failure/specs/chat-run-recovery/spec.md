# chat-run-recovery — Spec Delta

## ADDED Requirements

### Requirement: A failed run names what ran out

The message shown when a run dies MUST name the cause and the move that
would change the outcome. A single catch-all sentence for every failure is
not acceptable: it names nothing the reader can act on, leaving retrying
blind as the only option.

The system MUST distinguish at least: the tool-step budget exhausted, a
tool that exceeded its time bound, an answer that outgrew what one turn can
hold, the turn deadline, and a provider that stopped answering. Each of
these is retryable and MUST be reported as such.

#### Scenario: The tool-step budget runs out

- **GIVEN** a run that used every tool round without reaching an answer
- **THEN** the notice says so and suggests asking for a narrower slice

#### Scenario: A tool times out

- **GIVEN** a round abandoned on its wall-clock bound
- **THEN** the notice says a tool took too long, and offers a retry

#### Scenario: The provider stops answering

- **GIVEN** an executor failure
- **THEN** the notice points at the provider key in Settings

### Requirement: One failure surface, beside the turn that died

A run's death MUST be reported once. The inference failure and the recovery
notice are two reports of one event; the surface SHALL render a single card
carrying the failure's own account when it has one and the recovery copy
otherwise.

The card MUST render in the transcript's scroll flow, after the run's
activity — beside the turn that died — never pinned above the scroll
region, where it describes a failure the reader must scroll away from to
see and stays on screen over every turn that follows.

Dismissing the card MUST clear both underlying fields, so it cannot
immediately return carrying the other one. Retrying MUST clear both before
the new run starts.

#### Scenario: An interrupted run shows one card

- **GIVEN** a run that raised both an error and a recovery notice
- **THEN** exactly one card is shown, carrying the error's account

#### Scenario: A run that died without an account

- **GIVEN** a recovery notice and no error
- **THEN** the card shows the recovery copy as a quiet status, not an alert

#### Scenario: Dismissing clears the whole card

- **GIVEN** a card fed by both fields
- **WHEN** it is dismissed
- **THEN** it does not return
