## ADDED Requirements

### Requirement: Data Analyst owns stored provider-key encryption

The desktop MUST submit provider secrets through the DA cleartext secret
contract and MUST NOT claim ownership or version of the stored envelope.

#### Scenario: Create a provider key

- **WHEN** a user submits a verified provider credential
- **THEN** desktop sends the cleartext in `secret` over the authenticated DA connection
- **AND** desktop does not send `secret_owner` or `envelope_version` metadata
- **AND** DA returns a row marked `secret_owner: data-analyst`

#### Scenario: Rotate a provider key

- **WHEN** a user replaces the secret for an existing provider-key row
- **THEN** desktop uses `PUT /provider-keys/{id}/secret`
- **AND** it preserves the row identity, provider, label, enabled state, and default state

### Requirement: Provider secrets resolve only for active inference

The desktop MUST resolve the chosen provider-key row from DA and MUST bound the
returned cleartext to the active inference call in zeroizing memory.

#### Scenario: Resolve the preferred key

- **WHEN** multiple rows exist for a provider
- **THEN** desktop selects only enabled rows
- **AND** it prefers the default row
- **AND** it resolves that row through `POST /provider-keys/{id}:resolve`

#### Scenario: DA cannot resolve a row

- **WHEN** DA returns a conflict for a foreign, disabled, or unreadable envelope
- **THEN** desktop reports an actionable unavailable-credential state
- **AND** desktop does not crash or expose the stored value

### Requirement: Desktop migrates readable legacy provider keys

The desktop MUST migrate a legacy `xfdc:v1` provider-key row when the active
device key can open it and MUST leave unreadable rows available for re-entry.

#### Scenario: Current device owns the legacy envelope

- **WHEN** authenticated startup finds a matching legacy device envelope
- **THEN** desktop opens it locally
- **AND** replaces the row secret through DA
- **AND** zeroizes the cleartext after the migration request

#### Scenario: Row is already DA-owned

- **WHEN** authenticated startup finds `secret_owner: data-analyst`
- **THEN** desktop does not send a migration request

#### Scenario: Another device owns the legacy envelope

- **WHEN** the active device cannot open a legacy row
- **THEN** desktop does not modify the row
- **AND** the settings projection indicates that user re-entry is required
