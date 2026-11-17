## Context

Desktop currently seals provider keys with a device-bound key before DA stores
them. DA now owns this encryption boundary so one tenant-user row can be used by
all authenticated clients.

## Goals / Non-Goals

- Goals: one shared key, server-owned encryption, silent migration where this
  device can open the legacy row, bounded plaintext lifetime, and clear failures.
- Non-Goals: change MCP credential storage or DA implementation.

## Decisions

- Send `secret` only to DA create and secret-replacement operations. Keep
  client-side provider verification before writes.
- Select enabled credentials in the current default-first order, then resolve
  the selected row through DA with the existing OIDC bearer.
- Hold the resolved value in `Zeroizing<String>` for one inference call.
- Keep `NeedsReentry` only for a legacy row that this device cannot migrate.
- Migrate only `xfdc:v1` rows whose `device_key_id` matches the active device.
  Skip rows marked `secret_owner: data-analyst`.
- Do not change the immutable generated DA client until the reviewed operations
  exist in the deployed staging OpenAPI document.

## Risks / Trade-offs

- Resolution adds one authenticated DA round trip per inference call. The route
  is not deployed to staging yet, so its live latency is not measurable. Record
  list-plus-resolve latency in an authenticated staging smoke test before release.
- Cleartext crosses the authenticated desktop-to-DA connection and exists in
  process memory. Redacted wrappers and zeroization bound its lifetime.

## Migration Plan

1. Stop sending DA-owned metadata markers.
2. Cut create, replace, and inference resolution to the DA secret endpoints.
3. On authenticated startup, migrate readable device envelopes in place.
4. Advance the generated-client pin only after the DA staging contract exposes
   all three reviewed operations.
