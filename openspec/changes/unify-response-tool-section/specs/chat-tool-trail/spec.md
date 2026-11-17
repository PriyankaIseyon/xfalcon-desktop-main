## MODIFIED Requirements

### Requirement: One umbrella holds all tool rows in a response

The transcript SHALL render at most one collapsed tool umbrella between two
user turns. The umbrella SHALL contain every tool call from every assistant tool
round in provider order. Assistant prose carried by a tool-bearing round SHALL
NOT render as a transcript message. The final assistant message SHALL render
once, after the umbrella.

A projected tool-result turn with no text and no calls SHALL render nothing. A
new user turn SHALL close the current response and start a new grouping scope.
The umbrella label SHALL count every call it contains.

#### Scenario: Narrated multi-round response has one tool section

- **GIVEN** a response with one narrated tool call, then four more tool calls,
  then a final answer
- **WHEN** the transcript renders
- **THEN** exactly one collapsed umbrella labelled "5 tools used" renders
- **AND** the final answer renders after it
- **AND** the tool-round narration does not render as a separate response

#### Scenario: Unnarrated multi-round response has one tool section

- **GIVEN** a response with two calls, then three calls, then a final answer
- **WHEN** the transcript renders
- **THEN** exactly one umbrella labelled "5 tools used" renders before the
  final answer

#### Scenario: A user turn separates responses

- **GIVEN** tool activity, then a user message, then more tool activity
- **WHEN** the transcript renders
- **THEN** two separate umbrellas render, one per response

#### Scenario: A tool-result turn draws nothing

- **GIVEN** a projected turn with no text and no calls between two tool rounds
- **THEN** it renders no row
- **AND** both tool rounds remain in the same umbrella

#### Scenario: A tool-only response remains inspectable

- **GIVEN** a response that records tool calls but no final assistant prose
- **WHEN** the transcript renders
- **THEN** one collapsed umbrella renders with every recorded call
- **AND** the transcript invents no assistant answer
