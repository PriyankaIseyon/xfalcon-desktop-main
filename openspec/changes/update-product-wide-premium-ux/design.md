## Context

xFalcon Desktop is a React 19 and Tauri application. It has a token sheet and a
large shared UI folder. It also has many direct element implementations and local
utility combinations. This split lets each feature make valid local choices that
do not form a coherent whole.

The code audit found 52 raw button elements, 71 shared `Button` uses, 35
`IconButton` uses, and only 5 explicit `Tooltip` uses in production TSX. It also
found ten radius forms, one-off height values, hard-coded small text, and local
hover, focus, disabled, and transition rules. These counts are signals, not
automatic defects. Each consumer still needs a real interaction review.

Existing visual baselines cover the shell, chat empty and recovery states, project
trust, live activity, and patch approval. They do not cover most datasets, reports,
settings, project file states, canvas states, menus, toasts, or native handoffs.

## Goals

- Make every visible surface feel like one deliberate desktop product.
- Keep data and conversation content visually dominant.
- Use a small and named set of sizes, shapes, depths, type roles, and motions.
- Make the same action look and behave the same in every feature.
- Keep all states legible in light, dark, high contrast, forced colors, reduced
  motion, reduced transparency, narrow windows, and coarse-pointer conditions.
- Make closure measurable with tests, screenshots, and owner review.

## Non-Goals

- Do not redesign product behavior that is already correct.
- Do not add decorative motion without an information or continuity purpose.
- Do not copy a gallery component without adapting it to xFalcon semantics.
- Do not add a new UI library until the current token and primitive set is audited.
- Do not use OS translucency as a requirement for legibility or hierarchy.
- Do not change the font files, font families, font loading, or family-to-role
  mapping. Google Sans remains the product face. Geist Mono remains the existing
  code and data face.

## Decisions

### Decision: One closure ledger

`ux-matrix.md` is the source of truth for coverage. Each visible surface has one
row. Each shared primitive has one row. Each cross-product quality attribute has
one contract. A row moves to `Verified` only when the listed evidence exists and
passes.

### Decision: Four layers

The work moves in this order:

1. Foundation tokens and quality rules.
2. Shared primitives and interaction states.
3. Feature surfaces and their full state sets.
4. Visual, keyboard, responsive, platform, and owner acceptance.

A feature must use the verified layers beneath it. A local exception needs a note
in the matrix and a test that proves why it exists.

### Decision: Named geometry

- Spacing uses the 4 px sequence: 4, 8, 12, 16, 20, 24, 32, 40, and 48 px.
- Controls use 32 px compact, 36 px default, and 40 px prominent heights.
- Coarse-pointer hit areas are at least 44 by 44 px.
- Icon glyphs pair with control size: 14, 16, and 18 px.
- Corners use 6 px compact, 8 px control, 12 px card, and 16 px large-surface
  radii. A pill is valid only for a chip, status, or circular action.
- Borders use hairline, default, and strong tokens only.
- Shadows use three elevation levels. Borders still define edges in high contrast.

The implementation may adjust exact values after real screenshots. It must keep the
named roles and remove one-off alternatives.

### Decision: Typography polish does not change the font system

Typography work may align role, size, weight, line height, wrapping, and spacing.
It MUST keep the current Google Sans and Geist Mono files and mappings unchanged.
No batch may add, replace, remove, regenerate, subset, or remap a font.

### Decision: Complete interaction grammar

Every interactive primitive defines rest, hover, focus-visible, pressed, disabled,
busy, selected, success, warning, and error states where those states apply.
Icon-only and ambiguous controls have an accessible name and a tooltip. A visible
text label is preferred over a tooltip. Destructive actions use danger styling only
at the decision point.

### Decision: A hint and a clipped label are different jobs

`Tooltip` is the one hover-and-focus help contract. It portals, it flips away
from a viewport edge, it closes on Escape, scroll, and resize, and a keyboard
reaches it. `IconButton` shows its own `aria-label` through it by default, so an
icon-only control is discoverable without every call site repeating itself. A
tooltip that only repeats the accessible name does not also describe the control,
because a screen reader would then read the name twice.

Clipped text is not a hint. It repeats words the reader can already see, it has
to survive hundreds of rows in a virtualized grid, and it must not take focus
from the row that owns it. `TruncatedText` keeps the platform `title` for that
one job and attaches it only when the box really clips the string.

Everything else loses its native `title`. It cannot be styled, it never reaches
a keyboard, its delay belongs to the operating system, and the visual suite
cannot see it. `<iframe title>` stays, because there `title` is the accessible
name the platform requires. `scripts/check-tooltip-coverage.mjs` holds the line
and reports icon-action naming.

### Decision: Two focus roles, and only two

A control -- button, link, tab, menu item, row, slider -- takes the ring
`index.css` declares: 2 px of `--xf-focus` at 2 px offset, on keyboard focus
only. A primitive does not re-declare it. No rule on `:focus-visible` may set a
property that changes the element's own shape.

A text field is the other role, and it differs on purpose: accent border plus an
`--xf-accent-muted` halo, on ANY focus rather than keyboard focus alone, because
a person typing needs to see which box the characters are going into.
`ui/controlStyles.ts` owns it.

### Decision: Motion explains change

Motion uses the existing 80, 120, 180, 240, and 320 ms duration tokens. Small
feedback uses 120 ms. Surface changes use 180 to 240 ms. Page or panel continuity
may use 320 ms. Movement uses transform and opacity where possible. Reduced motion
removes nonessential movement and retains state feedback.

### Decision: A 90/100 exit gate

The scorecard weights system consistency, hierarchy, interaction clarity, state
completeness, accessibility, responsive and platform behavior, motion, and perceived
craft. A score below 90 is not complete. No weighted score can hide an open critical
or high-severity issue.

## Reference Direction

- [Beautiful UI](https://www.beautifului.dev/) gives relevant AI-native patterns
  for thinking, tool steps, approvals, chat, composer, tables, diffs, and sidebars.
- [21st.dev](https://21st.dev/) shows a broad React component library. It is useful
  for comparing composition, finish, and states across many implementations.
- [beUI](https://beui.dev/) is useful for motion continuity in modals, toasts,
  palettes, tabs, drawers, switches, and tooltips.
- [Rare UI](https://www.rareui.com/) is useful as a restraint check for distinctive
  motion and form controls. Its effects are not a default product language.
- [Transitions.dev](https://transitions.dev/) is useful for state swaps, streaming,
  skeleton reveal, menu origin, panel change, and error feedback.
- [shadcn/ui](https://ui.shadcn.com/) supports the source-owned primitive model:
  start with accessible primitives, then make the system its own.
- [Apple Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/)
  inform desktop hierarchy, familiarity, direct manipulation, focus, accessibility,
  and platform fit.

The Exa research connection was unavailable during matrix creation. The direct
primary pages above were inspected. A broader competitive sweep remains optional and
does not change the closure structure.

## Risks and Mitigations

- A broad restyle can break working states. Implement one vertical slice at a time
  and run its real consumer tests before moving on.
- More animation can reduce quality. Require a continuity or feedback purpose for
  every transition.
- A global token change can cause hidden regressions. Capture the current baseline,
  change tokens in small groups, and review every matrix row that consumes them.
- Native WebView and operating-system differences can escape browser tests. Add
  Windows and macOS smoke evidence for native dialogs, window chrome, DPI, and
  material fallback.
- Visual approval can become subjective. Use the rubric, screenshots, interaction
  tests, and final owner review together.

## Migration Plan

1. Freeze the baseline and assign each matrix row.
2. Consolidate overlapping active OpenSpec tasks.
3. Add the named token and primitive contracts.
4. Migrate all consumers in product-area waves.
5. Delete obsolete local visual paths after the last caller moves.
6. Run the complete evidence gate and score the product.
7. Complete owner review at 90/100 or higher.

## Delivery

The owner approved small verified commits directly to `main`. Each batch MUST pass
its targeted tests and proportional repository checks before it is pushed. Only the
files owned by that batch may be staged. Unrelated worktree changes remain untouched.

## Resolved Scope Decisions

- Native Mica and macOS vibrancy do not enter this change. The product keeps the
  opaque shell that is already specified and implemented. This avoids a second
  platform-specific surface path before the opaque product reaches the 90/100 gate.
  No OS material is requested, so unsupported, reduced-transparency, and
  initialization-failure branches do not exist in this build.
- Completion sounds do not enter this change. A sound would add a new preference,
  notification path, and accessibility contract without improving an existing UX
  defect in the approved polish scope. The product remains silent by default and
  adds no audio dependency or hidden setting.
