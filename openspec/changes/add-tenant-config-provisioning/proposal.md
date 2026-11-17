# Change: Provision tenant auth config at CI build time (B2B)

## Why

There is currently no working mechanism for a downloaded copy of the app to
learn which Casdoor/DA tenant it belongs to. `AuthConfig::from_env()`
(`src-tauri/src/auth/config.rs`) reads `XFALCON_DA_BASE_URL`,
`XFALCON_CASDOOR_ISSUER`, `XFALCON_CASDOOR_CLIENT_ID`,
`XFALCON_CASDOOR_ORGANIZATION` from the process environment. In a debug build
that's populated by `load_workspace_dotenv()` (`src-tauri/src/lib.rs`), which
loads `.env` from a path baked in via `env!("CARGO_MANIFEST_DIR")` — the
build machine's absolute path, meaningless on any other machine. In a release
build that loader is a no-op entirely (`#[cfg(not(debug_assertions))]`).
`.github/workflows/desktop-build.yml` never sets any `XFALCON_*` var during
the CI build either. Net effect: every exe/dmg/msi produced by CI today,
including the artifacts already attached to the `v0.5.0` draft release, has
no auth config and fails sign-in identically on every machine it's installed
on, for every teammate.

## What Changes

- Runtime config resolution gains a third source, checked before the existing
  ones: a JSON resource file bundled into the app at build time. `.env` (dev)
  and process env vars remain valid fallbacks so local development is
  unaffected.
- The `v*` tag-push trigger is dropped. The build runs only via
  `workflow_dispatch`, which now takes five typed inputs: `version` (the
  release tag, since there's no tag push to read it from) plus the four
  tenant values (`da_base_url`, `casdoor_issuer`, `casdoor_client_id`,
  `casdoor_organization`). These are typed in on the Actions UI (or via
  `gh workflow run` flags) each run and rendered into the bundled resource
  file before the app is built. No pre-stored secrets, no GitHub
  Environments — these four values aren't secrets (public OAuth client
  info), so plain dispatch inputs are enough.
- Every run is now a release build: the signing-secret check and the draft
  release/tag creation, previously gated on `startsWith(github.ref,
'refs/tags/')`, run unconditionally using the `version` input.

## Out of Scope

Self-serve, per-individual-user config (B2C) is a different problem: it
can't be solved by baking values in at CI time, since CI doesn't build once
per signup. That needs a runtime first-run "which org" flow backed by a
tenant-discovery endpoint on DA, which is real design work of its own and is
tracked as a separate follow-on change, not designed here.

## Impact

- Affected specs: `desktop-tenant-config` (new)
- Affected code: `src-tauri/src/auth/config.rs`, `src-tauri/src/lib.rs`,
  `src-tauri/tauri.conf.json` (`bundle.resources`),
  `.github/workflows/desktop-build.yml`
- Affected process: publishing a build for a tenant is now always a manual
  `workflow_dispatch` run with the version and four tenant values typed in;
  pushing a `v*` tag no longer triggers anything.
- Not affected: `AuthServiceError`/OAuth flow itself, DA contract, MCP server
  config (already runtime/DA-backed, untouched by this change).

## Risks

- One build per tenant, always manual: no unattended release path anymore.
  Every release, for every tenant, is a person filling in five inputs by
  hand. Simpler to reason about at current volume; revisit if release
  frequency or tenant count grows enough that this becomes the bottleneck.
- Typing the five inputs wrong (e.g. a stale `da_base_url`) produces a build
  that looks fine and fails at sign-in — same failure mode as today, just
  moved from "nobody set anything" to "somebody fat-fingered an input".
  Nothing in this change catches that beyond what sign-in already reports.
- The bundled config values (DA base URL, Casdoor issuer/client id/org) are
  public-ish OAuth client info, not secrets requiring confidentiality at
  rest — same class of thing a B2C user would type in by hand. They are
  bundled in the clear; this is a deliberate, recorded decision, not an
  oversight (see `design.md`).
- Already-published `v0.5.0` release artifacts are not fixed by this change
  retroactively; a new `workflow_dispatch` run must be made once this lands.
