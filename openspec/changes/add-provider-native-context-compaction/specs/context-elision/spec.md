# Delta Spec: context-elision

## ADDED Requirements

### Requirement: Native Compaction Precedes Destructive Elision

For a supported conversation with a valid native-compaction route, the context manager SHALL use native compaction as the normal response to the provider trigger or projected hard-limit pressure. It SHALL NOT silently clear old tool previews or evict complete transcript groups after native compaction is active.

#### Scenario: Active native context compacts instead of evicting

- **GIVEN** a valid active provider compaction and enough safe prefix for another compaction request
- **WHEN** new context needs room
- **THEN** the driver SHALL request and durably activate native compaction
- **AND** it SHALL not remove a transcript group only to satisfy the local builder

#### Scenario: Compaction failure remains visible

- **WHEN** native compaction or durable activation fails
- **THEN** the run SHALL report the typed failure
- **AND** it SHALL not hide the failure by silently taking an elided context path

### Requirement: Emergency Elision Keeps the Durable Payload Descriptor

Emergency elision MAY run only before native state is active while selecting a legal progressive-bootstrap prefix, or when one unmeasured addition would otherwise prevent a legal compaction request. It SHALL operate on model previews only. It SHALL retain payload ID, media and format identity, exact length, digest, completeness state, schema or column identity, head and tail evidence, and an explicit omission notice. It SHALL never delete or alter exact DA payload bytes.

#### Scenario: Emergency table elision remains useful

- **GIVEN** a table preview must shrink before a bootstrap compaction request
- **WHEN** emergency elision runs
- **THEN** it SHALL retain the table identity, columns, row count when known, head and tail evidence, payload descriptor, and omitted amount
- **AND** the internal range-read tool SHALL remain able to access the exact ready payload

#### Scenario: Elision cannot claim full content

- **WHEN** any preview bytes or rows are omitted
- **THEN** the model-visible result SHALL mark itself incomplete
- **AND** no success field SHALL describe the preview as the exact full result

### Requirement: Emergency Elision Preserves Complete Groups and Makes Progress

Each emergency elision SHALL strictly reduce estimated context cost. An already minimal preview SHALL not be selected again. Whole-group removal, when no legal preview reduction remains, SHALL preserve tool-call pairing and SHALL be used only to construct a safe prefix that the provider will immediately compact.

#### Scenario: Minimal previews terminate the loop

- **GIVEN** every retained preview is already at its minimum structural form
- **WHEN** more room is required
- **THEN** preview elision SHALL report no progress
- **AND** the caller SHALL choose a smaller complete prefix or fail explicitly

#### Scenario: Group removal is not a normal continuation context

- **GIVEN** a complete old tool round is excluded to create a legal bootstrap prefix
- **WHEN** that prefix fits
- **THEN** the next operation SHALL be a compaction-only provider request
- **AND** the desktop SHALL not ask for the user's final answer from that incomplete raw prefix
