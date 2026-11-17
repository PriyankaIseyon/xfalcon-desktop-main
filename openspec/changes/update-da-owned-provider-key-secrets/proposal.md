# Change: Use DA-owned provider-key secrets

## Why

Desktop device sealing makes one provider-key row unreadable from another
client or device. Data Analyst now owns provider-key encryption and exposes
authenticated create, rotate, and resolve operations for that shared row.

## What Changes

- Stop writing client-owned envelope markers that DA now rejects.
- Submit cleartext provider keys to DA for server-side sealing.
- Resolve a selected enabled key from DA for each inference call.
- Migrate device-readable `xfdc:v1` rows to DA ownership in the background.
- Keep device sealing for local MCP server credentials.

## Impact

- Affected specs: `provider-key-secrets`
- Affected code: credential service, DA provider-key client, bootstrap, settings status, tests, and the pinned DA OpenAPI client after staging deployment
- External dependency: data-analyst `feature/da-owned-provider-key-secrets`
