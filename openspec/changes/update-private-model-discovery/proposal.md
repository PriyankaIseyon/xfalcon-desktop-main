# Change: Discover and configure private models

## Why

Private model setup currently assigns the company Qwen model and limits to every endpoint. Other hosted OpenAI-compatible APIs need their own exact model IDs and settings.

## What Changes

- Start with an API base URL and key. Next requests that base's `/models` list through the existing guarded native transport.
- Offer returned model IDs, with manual entry when discovery is unavailable or the desired model is absent.
- Preserve valid endpoint settings exactly. Fill missing settings with labeled xFalcon suggestions: 32,768 context tokens, 4,096 output tokens, Temperature 0.7, and Top P 0.9. Fit missing token suggestions alongside any reported limit.
- Validate context and output together using the backend's shared reserve and output cap. Explain supported limits, clear errors when fields change, and allow errors to be dismissed without losing settings.
- Block duplicate URL/key/upstream-ID connections in the add dialog and native save path, regardless of display name, and name the existing entry.
- Require a completed greeting from the selected model before creating it or saving connection changes. Explain failures and keep the editor open for correction.
- Store sampling overrides per account on this device and apply them to actual private inference. Keep DA's deployed contract unchanged.
- Limit company transport tuning to its exact host and known Qwen ID. Never substitute that ID for a different selected model.

## Impact

- Affected specs: `desktop-private-custom-models`.
- Affected code: Desktop contracts, Settings editor, custom provider transport, native commands and permissions, DA payload mapping, account-local preferences, and tests.
- Compatibility: existing private model rows and keys remain valid. No other repository, server schema, or release channel changes.
