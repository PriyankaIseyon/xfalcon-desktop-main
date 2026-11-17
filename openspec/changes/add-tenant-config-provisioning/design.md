## Context

`AuthConfig::from_env()` is a plain `std::env::var` read. It works today only
because a developer's `.env`, loaded from a build-machine-absolute path, sits
next to the debug binary. Nothing analogous exists for a distributed build.
The fix has to add a config source that survives being copied to another
machine, without disturbing the existing dev-loop `.env` path.

## Goals / Non-Goals

- Goals: one CI build per tenant, each producing an artifact that
  authenticates against that tenant's Casdoor org out of the box; no change
  to local dev workflow; no change to how MCP servers are configured (already
  runtime/DA-backed).
- Non-Goals: B2C self-serve per-user config (separate change, see
  `proposal.md`); rotating a shipped build's tenant config without a rebuild;
  a single binary serving multiple tenants at runtime; an unattended/
  tag-triggered release path (dropped — see Decisions).

## Decisions

- **Decision: a bundled JSON resource file, not `env!()` compile-time
  constants.** `env!()` bakes values into the compiled binary, which means
  Cargo's build cache is invalidated per tenant and every tenant rebuild pays
  full compile time. A resource file is written once, right before
  `tauri-action` bundles it, and Rust code never rebuilds for a config
  change. This also keeps the diff small: `config.rs` gains one more lookup
  path, not a second config-loading strategy per environment.
  - Alternative considered: keep runtime env vars only, ask each tenant's
    admin to set system environment variables on every install. Rejected —
    this is exactly the manual, per-machine step this change exists to
    remove; still kept as a fallback for local dev, not the primary path.

- **Decision: resolution order is bundled resource file → process env
  (`.env` in dev) → `NotConfigured` error.** Preserves today's dev loop
  unchanged (env still wins if someone sets it, e.g. to override a bundled
  tenant while testing) while giving a distributed build a value on a
  machine with no `.env` at all.

- **Decision: drop the `v*` tag-push trigger; `workflow_dispatch` is the only
  trigger, and it takes the tenant values as plain typed inputs.** A tag push
  carries no way to attach input parameters, so it cannot carry per-build
  tenant values at all — keeping it would mean either a second, differently
  configured trigger path, or a "primary tenant" concept with its own
  storage and its own open question of who maintains it. Dropping it removes
  both: every release is the same one path, five inputs, typed in by
  whoever is cutting that tenant's release. `version` becomes an input too
  (previously read from `github.ref_name`), and the signing-secret check and
  draft-release/tag creation — previously gated on
  `startsWith(github.ref, 'refs/tags/')` — run unconditionally, since every
  `workflow_dispatch` run now is a release build.
  - Alternative considered: keep tag-push for one designated "primary"
    tenant (values in a GitHub Environment), `workflow_dispatch` for
    everyone else. Rejected for now — two release paths to maintain and a
    "who owns the primary tenant's stored values" question, for a benefit
    (unattended releases) nobody asked for yet. Revisit if release cadence
    grows enough that typing five inputs by hand every time becomes the
    bottleneck.

- **Decision: config values are bundled in the clear, not encrypted.** DA
  base URL and Casdoor issuer/client id/org are OAuth client-registration
  info, not credentials — the same values a B2C user would eventually type
  into a first-run screen by hand. They don't belong in the OS
  keyring/sqlite envelope alongside actual provider secrets and MCP grants;
  treating them as secrets would just misclassify them and complicate the
  build for no confidentiality gained.

## Risks / Trade-offs

- N tenants → N artifacts per version, each a full native build (no
  cross-compiling to macOS/Windows from one runner), each a manual run.
  Linear CI cost growth plus linear human-effort growth; acceptable now,
  worth revisiting if tenant count or release cadence grows past a handful.
- No revocation path for an already-shipped tenant build if that tenant is
  offboarded — Casdoor-side deactivation handles access; this change doesn't
  add build-side revocation, matching how the signing-key rotation in
  `add-signed-auto-update` is also non-retroactive.
- A person with filesystem access to an install can read that tenant's
  bundled config values. Accepted per the "not a secret" classification
  above.
- A mistyped input (wrong `da_base_url`, wrong `version`) has no validation
  beyond "the build succeeds, sign-in fails later" — no dry-run/confirm step
  is added here.

## Migration Plan

The already-published `v0.5.0` draft release artifacts are not touched by
this change and remain unable to authenticate. Once this lands, the `v*` tag
trigger is gone — the first working artifact comes from running
`workflow_dispatch` by hand with real values. Existing dev workflows
(`.env` and a debug build) are unaffected — the new resource-file path is
additive.

## Open Questions

None blocking — the primary-tenant/GitHub-Environment question this
originally raised no longer applies now that every run takes its tenant
values as typed inputs.
