# chat-composer — Spec Delta

## ADDED Requirements

### Requirement: The send control doubles as the stop control

While a run is in flight, the composer's circular send control SHALL become a
stop control: Square (filled) icon, always enabled, labelled "Stop the
assistant's response", invoking the run-cancel path. No separate cancel
affordance appears anywhere else on the chat surface. When no run is in
flight the control is the send arrow, enabled only with a non-empty draft.

#### Scenario: Stop during generation

- **GIVEN** a run is streaming
- **WHEN** the user clicks the composer's circular control
- **THEN** the active run is cancelled
- **AND** no "Cancel run" pill exists above the transcript

### Requirement: Pending runs do not masquerade as disconnection

The composer SHALL show its disconnected copy ("Chat is not connected yet")
only when the secure runtime is actually unavailable. While a run is merely
in flight, the placeholder stays "Ask anything" and typing remains possible;
only submission is gated.

#### Scenario: Typing while the assistant responds

- **GIVEN** a run is streaming and the runtime is connected
- **WHEN** the user focuses the composer
- **THEN** the placeholder reads "Ask anything" and keystrokes land in the
  draft
