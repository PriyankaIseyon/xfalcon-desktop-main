# Change: Product-wide premium UX system

## Why

The current interface does not read as one product. User-visible surfaces use
different control sizes, shapes, density, type, depth, state feedback, and help
patterns. The owner rates the current experience at 4/10 and requires a verified
9/10 target.

Previous polish changes improved isolated chat and shell areas. They did not set
one product-wide quality contract or prove coverage of every reachable surface.
This change creates that contract and a closure ledger before implementation.

## What Changes

- Inventory every user-visible surface, state, overlay, native handoff, and shared
  primitive in `ux-matrix.md`.
- Define one product-wide standard for geometry, spacing, type, color, borders,
  radius, depth, iconography, control hierarchy, state feedback, motion, copy,
  accessibility, responsive behavior, and platform behavior.
- Replace one-off visible controls with the shared primitive contract. No caller
  may remain on an obsolete visual contract.
- Polish the shell, chat, projects, datasets, reports, settings, canvas, dialogs,
  menus, notifications, startup, authentication, recovery, and native handoffs.
- Add state-complete visual regression coverage. Each critical surface must cover
  light, dark, narrow, keyboard focus, and relevant error, empty, loading, and busy
  states.
- Use the weighted scorecard in `ux-matrix.md`. Closure requires at least 90/100,
  no open critical or high-severity UX issue, and verified evidence for every
  reachable matrix row.

## Scope and Coordination

- `add-workspace-destination-rail` remains the navigation behavior dependency.
  This change owns its final visual and interaction quality review.
- `add-power-composer-and-shell-polish` already delivered composer commands,
  mentions, token status, focus mode, and shortcut help. Its unfinished native
  material, sound, and visual-test tasks must be closed, moved here, or removed
  before implementation starts. Duplicate implementation is not allowed.
- Active upload, dataset, locale, and update changes keep their functional scope.
  This change owns the final UX acceptance of their visible surfaces.
- Third-party galleries are reference material only. They are not new runtime
  dependencies.
- The font system is fixed. Google Sans remains the product face. Geist Mono
  remains limited to the existing code and data roles. This change MUST NOT add,
  replace, remove, regenerate, subset, or remap font files or families.

## Impact

- Affected specs: `experience-quality` (new), then targeted deltas to existing
  capabilities as implementation reaches each surface.
- Affected frontend: all user-visible files under `src/app`, `src/features`,
  `src/ui`, `src/index.css`, and `src/styles/tokens.css`.
- Affected tests: component interaction tests, visual baselines, keyboard tests,
  contrast checks, reduced-motion checks, and desktop platform smoke checks.
- API and IPC behavior is out of scope unless a visible state cannot be made
  correct with the current generated contract.

## Approval Gate

The owner approved this proposal, the matrix, the batch delivery model, and direct
commit and push to `main` on 2026-08-26.
