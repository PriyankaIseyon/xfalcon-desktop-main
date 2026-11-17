## ADDED Requirements

### Requirement: The reader controls Try next suggestions

Settings SHALL offer a Try next suggestions switch in Appearance, immediately
after Model reasoning. The switch SHALL persist on this device through the native
appearance settings and apply immediately. New installations and older settings
documents without the field SHALL enable suggestions by default.

When disabled, chat SHALL hide the chips and SHALL NOT start suggestion requests.
Results from a request already in flight SHALL NOT reappear after disabling.
Re-enabling SHALL offer fresh suggestions for an otherwise eligible settled
answer. Existing streaming and availability guards SHALL still apply.

#### Scenario: Older settings retain the existing experience

- **GIVEN** a settings document without the follow-up preference
- **WHEN** the app reads it
- **THEN** suggestions remain enabled and other stored preferences are preserved

#### Scenario: Disable and restart

- **WHEN** the reader disables Try next suggestions and restarts
- **THEN** the switch remains off and no suggestion request starts

#### Scenario: Disable during a request and enable again

- **GIVEN** a suggestion request in flight
- **WHEN** the reader disables the setting
- **THEN** the chips disappear and the old response cannot display
- **WHEN** the reader enables the setting again
- **THEN** a fresh request can populate an eligible answer's chips

#### Scenario: A failed save is visible

- **WHEN** saving the switch fails
- **THEN** the choice stays applied and a message says it could not be saved
