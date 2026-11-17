# chat-run-activity — Spec Delta

## ADDED Requirements

### Requirement: A round refuses surplus parallel calls instead of failing the run

A round MUST NOT end the run when the model requests more tool calls than
the concurrency bound allows. It SHALL dispatch calls up to the bound and
refuse the surplus with a denial the model can act on, so it re-issues the
remainder on the following round.

The refusal MUST NOT open, authorize, journal, or dispatch the surplus
calls — the concurrency bound is never exceeded — and the round MUST still
return one outcome for every call the model requested, because an assistant
turn left with unanswered tool calls is rejected outright by the providers
on the next request.

The round budget continues to bound the exchange, so a model that keeps
over-requesting still terminates.

#### Scenario: Four calls against a bound of three

- **GIVEN** a round of four tool calls and a concurrency bound of three
- **WHEN** the round runs
- **THEN** three calls execute, the fourth is denied with reason
  `parallel_call_limit`, four outcomes are returned, and the run continues

#### Scenario: The bound is never exceeded

- **GIVEN** any round with more calls than the bound
- **THEN** the executor is never asked to run more than the bound

#### Scenario: The model can recover the refused call

- **GIVEN** a call refused for the parallel-call limit
- **WHEN** the model re-issues it in the next round
- **THEN** it is dispatched normally
