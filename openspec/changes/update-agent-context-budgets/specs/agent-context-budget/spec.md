## ADDED Requirements

### Requirement: A run's tool-result budget follows its model's context window

The system MUST size the largest tool result the model sees in one call from the run's
model context window: 5% of the window at 4 bytes a token, clamped between 32 KiB and
256 KiB. The ceiling SHALL equal the DA stored-preview bound. The rendered result,
the durable preview stored on DA, and every page a tool returns SHALL use that one
budget.

#### Scenario: A 1M-token model

- **GIVEN** a run on a model with a 1,000,000-token window
- **THEN** its budget is 200,000 bytes

#### Scenario: A small model

- **GIVEN** a run on a model with a 128,000-token window
- **THEN** its budget is the 32 KiB floor

#### Scenario: A result within the budget

- **WHEN** a result serializes within the budget
- **THEN** the model sees it whole and it is marked complete

### Requirement: An oversized result fills its budget

The system MUST, for a result larger than the budget, keep as much of it as fits: the
leading rows of an array, every small field of an object with the largest field cut
to fit, and the head and tail of a long string. It MUST state exactly what it omitted
(rows, fields, bytes) and MUST mark the preview incomplete.

#### Scenario: A large table

- **GIVEN** a query result of 10,000 rows that does not fit
- **THEN** the preview carries as many leading rows as fit, the total row count, and
  the number omitted

#### Scenario: A page object with a large content field

- **GIVEN** an object whose `content` string exceeds the budget
- **THEN** its other fields appear whole and `content` is cut to fit, with the omitted
  byte count

### Requirement: Oversized tool arguments are an answer, not a failure

The system MUST answer a tool call whose arguments exceed 256 KiB with an
`invalid_arguments` result that states the bound and suggests splitting the call. The
run MUST continue.

#### Scenario: A patch too large for one call

- **WHEN** the model sends a patch whose arguments exceed the bound
- **THEN** that call is answered with the bound and the next round proceeds

### Requirement: A message holds a full answer

The system MUST store a message of up to 512 KiB of text, equal to the run's output
budget, so an answer the run allowed can always be stored and reloaded.

#### Scenario: A long answer

- **WHEN** an assistant answer of 300 KiB completes
- **THEN** it is stored and reopens intact
