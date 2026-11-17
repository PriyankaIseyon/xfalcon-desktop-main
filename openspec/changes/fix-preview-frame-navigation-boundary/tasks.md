# Tasks

## 1. Navigation boundary

- [x] 1.1 Admit the preview origin in `allows_navigation`, in this platform's
      spelling only. Proof: `navigation_policy_admits_the_preview_origin_and_nothing_beside_it`
      passes, and asserts refusal for the inactive spelling, a foreign host, a
      port, credentials, and a neighbouring scheme.
- [x] 1.2 Move both origin matchers from `#[cfg]` to `cfg!`, so the macOS arm
      type-checks under the Windows gate. Proof: clippy over the workspace and
      all targets, with warnings denied, compiles both arms on one host.
- [x] 1.3 Exercise both platform spellings from one host in the navigation
      tests. Proof: `platform_forms` selects with `cfg!`, and both existing and
      new navigation tests pass.

## 2. Document policy

- [x] 2.1 Name the preview origin as the font source in both document
      policies, in both spellings. Proof:
      `document_policies_name_the_font_origin_rather_than_self` asserts the
      source list on the served header and asserts no `'self'` remains.
- [x] 2.2 Compose both policies from one origin literal so the two cannot
      drift. Proof: the policies are `concat!`ed from `preview_origin_sources!`,
      and `preview_http_host_matches_the_scheme` pins the Windows host to the
      scheme name.

## 3. Gate

- [x] 3.1 `bash scripts/gate.sh --rust` passes.
