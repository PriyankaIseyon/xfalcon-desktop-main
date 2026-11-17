# app-typography Specification

## Purpose

Two faces, one ramp. Every type role in the app derives from a single reading-size
multiplier, and every visible element resolves to one of exactly two families — with
no fallback, so a missing face fails loudly rather than blending into a lookalike.

## Scope

| File                               | What it owns here                              |
| ---------------------------------- | ---------------------------------------------- |
| `src/styles/tokens.css:99`         | `--xf-type-scale` and every `--xf-type-*` role |
| `src/styles/tokens.css:167`        | the three `[data-font-size]` steps             |
| `public/fonts/`                    | the shipped faces and their licences           |
| `index.html:10`                    | the preload links                              |
| `src/app/typography.ts`            | boot-time activation reporting                 |
| `scripts/check-font-inventory.mjs` | the inventory guard                            |

## Boundaries

- **Which reading size is in effect, and persisting it** belong to
  `settings-appearance`.
- **Typography inside a previewed document** belongs to `preview-protocol`, which
  serves its own copies of these faces on its own origin.
- **The chat column's reading size** is read from this ramp by `chat-thread-layout`.
- **Colour and surface tokens** belong to `app-shell-material`.

## Requirements

### Requirement: One type ramp, scaled as a whole

Every type role MUST derive from one reading-size multiplier, so a size change moves
every role together. A ramp whose roles scale independently stops being a ramp.

The fluid reading size MUST keep its clamped shape at every step, so the
window-width response survives the multiplier.

#### Scenario: Scaling preserves hierarchy

- **WHEN** the reading size changes
- **THEN** every type role changes proportionally and their ratios hold

#### Scenario: The reading size stays fluid at every step

- **GIVEN** any reading-size step
- **THEN** the reading role still responds to window width

### Requirement: The product faces alone, original files, preloaded

All application typography SHALL resolve through exactly two single-family token
stacks — one sans for prose and UI, one mono for code and data — with NO fallback
families. A missing face must fail loud as the browser's last-resort serif, never
blend into a system-sans lookalike.

The sans MUST be the original upstream variable font files, byte-unmodified, with the
full glyph set and all axes. Subsetting is banned as a fidelity variable; installer
size is accepted.

The primary sans and mono files MUST be preloaded, so first paint renders in the
product face without a visible swap.

#### Scenario: No visible element escapes the stacks

- **WHEN** the running app's visible elements are audited for computed font family
- **THEN** every element resolves to one of the two families exactly

#### Scenario: Faces are preloaded

- **WHEN** the document head is inspected
- **THEN** it carries preload links for the primary sans and mono files, each with its
  own type

#### Scenario: A failed face is unmissable

- **GIVEN** the bundled faces cannot load
- **WHEN** the app renders
- **THEN** text falls to the browser's last-resort serif rather than a metric-similar
  system sans

### Requirement: Font activation failure is observable

The app SHALL log one structured error naming the family when, after fonts settle, a
required face fails its activation check. A wrong-font report must be diagnosable from
the webview console alone.

A healthy activation MUST log nothing.

#### Scenario: Broken face logs, healthy face stays silent

- **GIVEN** a bundled face that cannot activate
- **WHEN** the app boots
- **THEN** exactly one activation error is logged naming that family
- **AND** when every face activates, nothing is logged

### Requirement: Regression guards pin the contract

Tests SHALL fail when the token stacks gain a fallback family or change their single
families, when a font-face source references a file missing from the shipped font
directory, or when the preload links are removed.

The font inventory check MUST fail if a subset copy of the sans reappears alongside the
originals.

#### Scenario: Deleting a font file fails the suite

- **WHEN** a shipped face is removed from the font directory
- **THEN** the typography guard fails naming the missing file

#### Scenario: A subset copy fails the inventory

- **WHEN** a subset of the sans is added to the font directory
- **THEN** the inventory check fails

## Provenance

Consolidated from the deltas of `preview-render-fidelity` and `appearance-settings`.
Family names and file names are stated by role rather than by literal, because the
guard tests are what pin the literals and a spec repeating them adds a second place to
update.
