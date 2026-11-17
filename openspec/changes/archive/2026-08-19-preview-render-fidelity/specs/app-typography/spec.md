# app-typography — Spec Delta

## ADDED Requirements

### Requirement: The product faces alone, original files, preloaded

All application typography SHALL resolve through exactly two single-family
token stacks — `--xf-font-sans: "Google Sans"` for prose/UI and
`--xf-font-mono: "Geist Mono"` for code/data — with NO fallback families
(owner directive 2026-08-06: a missing face must fail loud as the browser
last-resort serif, never blend into a Segoe/system-ui lookalike). Google
Sans SHALL be the ORIGINAL Google-distributed variable TTF pair,
byte-unmodified (full glyph set, all axes) — subsetting is banned as a
fidelity variable; installer size is accepted. `index.html` MUST preload
the primary sans and mono files so first paint renders in the product
face without a visible swap.

#### Scenario: No visible element escapes the stacks

- **WHEN** the running app's visible elements are audited for computed
  font-family
- **THEN** every element resolves to `"Google Sans"` or `"Geist Mono"`
  exactly (preview documents are governed by spec: preview-protocol
  instead)

#### Scenario: Faces are preloaded

- **WHEN** `index.html` is inspected
- **THEN** it contains preload links for `GoogleSans-Variable.ttf`
  (`type="font/ttf"`) and `GeistMono.woff2` (`type="font/woff2"`)

#### Scenario: A failed face is unmissable

- **GIVEN** the bundled faces cannot load
- **WHEN** the app renders
- **THEN** text falls to the browser's last-resort serif — visibly wrong —
  rather than a metric-similar system sans

### Requirement: Font activation failure is observable

The app SHALL log one structured console error (`font_activation_failed`)
naming the family when, after `document.fonts.ready`, the primary sans
face fails `document.fonts.check` — a wrong-font report must be
diagnosable from the webview console.

#### Scenario: Broken face logs, healthy face stays silent

- **GIVEN** the bundled face cannot activate
- **WHEN** the app boots
- **THEN** exactly one `font_activation_failed` error is logged; when
  the face activates normally nothing is logged

### Requirement: Regression guards pin the contract

Unit tests SHALL fail when: the token stacks gain fallback families or
change their single families; the `@font-face` sources reference files
missing from `public/fonts/`; or the index.html preloads are removed.
The font-inventory check (`npm run check:fonts`) MUST fail if a Google
Sans woff2 subset reappears under `public/fonts/`.

#### Scenario: Deleting a font file fails the suite

- **WHEN** `GoogleSans-Variable.ttf` is removed from `public/fonts/`
- **THEN** the typography guard test fails naming the missing file
