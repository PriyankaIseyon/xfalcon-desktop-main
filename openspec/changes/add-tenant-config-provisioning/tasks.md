## 1. Config loader

- [x] 1.1 Decide bundled resource file path/name (e.g.
      `resources/tenant-config.json`) and add it to `bundle.resources` in
      `src-tauri/tauri.conf.json`.
- [x] 1.2 Extend `AuthConfig` resolution (`src-tauri/src/auth/config.rs`) to
      check the bundled file first, then process env (`.env` in dev via the
      existing `load_workspace_dotenv`), then fail with the existing
      `NotConfigured` message.
- [x] 1.3 Unit tests: bundled file present and valid; bundled file absent with
      env fallback present; neither present (existing `NotConfigured` path
      unchanged).

## 2. CI build

- [x] 2.1 Remove the `push: tags: v*` trigger from
      `.github/workflows/desktop-build.yml`; `workflow_dispatch` becomes the
      only trigger.
- [x] 2.2 Add five required `workflow_dispatch` inputs: `version`,
      `da_base_url`, `casdoor_issuer`, `casdoor_client_id`,
      `casdoor_organization`.
- [x] 2.3 Add a step that renders those inputs into the bundled resource file
      before the `tauri-action` build step.
- [x] 2.4 Change `tagName`/`releaseName` to use the `version` input instead of
      `github.ref_name`.
- [x] 2.5 Drop the `startsWith(github.ref, 'refs/tags/')` conditionals on the
      signing-secret check and the release/tag creation — every
      `workflow_dispatch` run is now a release build, unconditionally.

## 3. Docs

- [x] 3.1 Record the config-sensitivity classification (not a secret vault)
      in `docs/threat-model.md`.

## 4. Verify

- [x] 4.1 `cargo test --workspace --locked`, `cargo clippy --all-targets -- -D
warnings`, `cargo fmt --all -- --check`, `vitest run`, `npm run
format:check`, `npm run lint` all green.
- [ ] 4.2 Dry run: `workflow_dispatch` twice with two different tenants'
      values; confirm the two resulting artifacts carry different bundled
      config and each signs in only against its own tenant's Casdoor org.
