# Font inventory

Version 0.5.28 · reviewed 2026-10-02 · hashes last recomputed 2026-10-02 · **Reference** · audience: release engineers, legal review

Release evidence for every font binary the desktop application ships. Runtime CSS
must reference only the three files marked **Ship**. Hashes are SHA-256 over the
exact repository bytes.

Enforced by `npm run check:fonts`, which runs as part of `npm test` (and as the
first step of the "vitest + font inventory" step of `scripts/gate.sh`).

---

## Shipped

| File | Family / version | Bytes | SHA-256 | Source | Licence |
| --- | --- | ---: | --- | --- | --- |
| `public/fonts/GoogleSans-Variable.ttf` | Google Sans Regular 13.002 — variable, `wght` 400–700, `opsz`, `GRAD` | 4,785,696 | `e4400e3891cb4677fb21834d29de2371d4a4831a47d462e5b9433e5b23ba5c73` | Official Google Fonts family download, **unmodified** (`GoogleSans-VariableFont_GRAD,opsz,wght.ttf`) | SIL OFL 1.1 — `public/fonts/GoogleSans-OFL.txt` |
| `public/fonts/GoogleSans-Italic.ttf` | Google Sans Italic 13.002 — variable, `wght` 400–700, `opsz`, `GRAD` | 5,081,488 | `588905eac48d6e62d323d4caf9be2a8d2e3a9874a69406b582a614b894b78037` | Official Google Fonts family download, **unmodified** (`GoogleSans-Italic-VariableFont_GRAD,opsz,wght.ttf`) | SIL OFL 1.1 — same file |
| `public/fonts/GeistMono.woff2` | Geist Mono Regular 1.700 — `wght` 100–900 | 71,368 | `fba8f577f38a2bbcbe818efa6348dd58f36303a10b8737c42fefad275be563ab` | `geist@1.7.2`, `dist/fonts/geist-mono/GeistMono-Variable.woff2` (recorded at the 2026-08-06 audit; the package is not in `package-lock.json`, so the repository cannot confirm it); package integrity `sha512-Gu5lDFa3pLRyoBlBPf0QIFHVdWAnpco7fS1bJm41jyLPFoguBgiubseUN2oLXMgqZ7uxAxDoXcHMhCY/fOTTgg==` | SIL OFL 1.1 — `public/fonts/Geist-OFL.txt` |

Both licence text files are **release artifacts**, not development-only
documentation: OFL 1.1 requires each distributed copy to carry its copyright
notice and licence.

---

## Why the full TTFs, not subsets

**Owner decision, 2026-08-06** — reverses the earlier "WOFF2 subsets only" rule.

Google Sans ships as the original Google-distributed variable TTF pair,
byte-unmodified: the full glyph set (7,525 glyphs in the upright file, 7,520 in the
italic file) and all axes (`wght` 400–700, `opsz` 17–18, `GRAD` −50–200).
The font tables confirm the glyph counts, the axes, version 13.002 and the
2025 Google Sans Project Authors copyright (read with `fontTools` on 2026-10-02).

Reasoning: a desktop installer has no page-weight budget, and using the
unmodified upstream files removes subsetting as a rendering-fidelity variable
after repeated wrong-font reports.

### Superseded — removed 2026-08-06

The Latin-subset WOFF2 pair borrowed from `xfalcon-web` shipped between
2026-07-27 and 2026-08-06.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `public/fonts/GoogleSans-Variable.woff2` | 90,108 | `950bdc4b14618db16fd0e19c36303687b47eb035e076a204ac1a2c3b19e1cf1d` |
| `public/fonts/GoogleSans-Italic.woff2` | 97,364 | `f65908a7306fc67f7f58e40c6a5b82dd60bf15394e25d4339ecb78fa1193b041` |

`npm run check:fonts` **fails if a `GoogleSans*.woff2` reappears**, because a
stray subset would silently shadow the originals in a future refactor.

---

## The check

`scripts/check-font-inventory.mjs` asserts that `public/fonts/` contains exactly
these five required files:

```
GoogleSans-Variable.ttf
GoogleSans-Italic.ttf
GeistMono.woff2
GoogleSans-OFL.txt
Geist-OFL.txt
```

and that no `GoogleSans*.woff2` exists. It runs before `vitest` in `npm test`, so
a missing licence file or a reintroduced subset fails the suite.

The hashes and byte counts in this document are **not** machine-checked — the
script verifies presence and absence only. On 2026-10-02 the byte counts and
SHA-256 values in the *Shipped* table were recomputed from the files in
`public/fonts/` and matched. Re-verify the hashes by hand when a font file
changes, and update the table in the same change.

---

## Redistribution controls

- Google Sans metadata identifies the 2025 Google Sans Project Authors and embeds
  an OFL 1.1 licence URL. `xfalcon-web`'s provenance record documents the earlier
  subset method, the preserved axes and features, the official Google Fonts
  specimen and licence sources, and the exact WOFF2 hashes above.
- Geist Mono metadata identifies the Geist Project Authors, version 1.700, and
  OFL 1.1. The vendored file size matches the variable Mono asset in the locked
  `geist@1.7.2` npm artifact exactly.
- Both families may be bundled, embedded, modified, and redistributed with the
  application under OFL 1.1, provided each distributed copy includes its
  copyright notice and licence.
- **Fonts are self-hosted.** No font CDN, no runtime package dependency, and no
  network fallback — the app CSP's `font-src 'self' data:` would refuse them
  anyway.
- The `xf-preview` origin serves these same faces over its `/fonts/<file>` route
  (the files are compiled into the binary with `include_bytes!`;
  `src-tauri/src/preview/protocol.rs`), so a previewed document can reach them
  under its own `font-src`, which names the preview origins and `data:`.

---

## Changing a font

1. Replace the file in `public/fonts/`.
2. Record the new byte count and SHA-256 in the table above, with the source and
   licence.
3. If the family changes, add the licence text file and add it to the `required`
   list in `scripts/check-font-inventory.mjs`.
4. Update the `@font-face` declarations in `src/styles/tokens.css`, and the
   `FONT_ROUTES` list and `@font-face` strings in
   `src-tauri/src/preview/protocol.rs`.
5. Confirm `npm run check:fonts` passes. Run the Playwright visual baselines
   (`npm run test:visual`) as well, since a font change is a visual change; they
   are off by default.
6. Move the outgoing file into the *Superseded* table with its hash. Do not
   delete the row; the shipped-bytes history is release evidence.
