## ADDED Requirements

### Requirement: A provider key can be removed

The desktop MUST let a person delete a stored provider key through
`DELETE /provider-keys/{id}` after an explicit destructive confirmation.

The confirmation MUST state that the key is removed for every signed-in xFalcon
client, not only this desktop.

The desktop MUST confirm the row exists in a fresh list before it sends the
delete, and MUST NOT send a delete for an id the fresh list does not contain.
Removal is idempotent: a key already gone counts as removed.

#### Scenario: Remove a key

- **WHEN** a person confirms removal of a listed key
- **THEN** desktop sends one authenticated `DELETE /provider-keys/{id}`
- **AND** the key list reloads without that row
- **AND** workspace status is re-read, so chat turns off when no usable key remains

#### Scenario: A key already gone

- **WHEN** the id no longer names a row in the fresh list, or DA answers not found
- **THEN** no other row is deleted and the removal succeeds

#### Scenario: The delete's answer is lost

- **WHEN** DA does not answer a sent delete
- **THEN** desktop reports that the result must be reviewed and reloads the list

#### Scenario: DA answers for another row

- **WHEN** DA's delete response names a different row
- **THEN** desktop reports that the result must be reviewed and reloads the list
