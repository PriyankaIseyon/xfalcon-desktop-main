# Change: Make Try next suggestions optional

## Why

The user requested a Settings control for the Try next chips. Readers should be
able to turn these prompts off and on without changing other chat behavior.

## What Changes

- Add Try next suggestions below Model reasoning in Settings > Appearance.
- Save the device preference through the existing native appearance settings.
- Keep suggestions enabled for new installs and older settings files.
- Hide chips immediately and prevent new suggestion requests while disabled.

## Impact

- Affected specs: settings-appearance.
- Affected code: appearance contract and store, Settings, ChatSurface, test fixtures.
- Existing on-device settings gain one defaulted boolean; no DA contract changes.
