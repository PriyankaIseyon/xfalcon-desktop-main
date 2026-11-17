## ADDED Requirements

### Requirement: Every user-visible surface is tracked to verified closure

The product MUST maintain one inventory of every reachable page, section, component,
overlay, state, and native handoff. Each inventory row MUST name its source files,
required quality lanes, implementation wave, status, and verification evidence.

A surface MUST NOT be called complete while any required lane is unverified. A newly
added user-visible surface MUST enter the inventory in the same change that adds it.

#### Scenario: A new visible dialog is added

- **WHEN** a change adds a user-visible dialog
- **THEN** the closure ledger gains a row for the dialog and its loading, error,
  keyboard, narrow-window, and theme states
- **AND** the change cannot close until that row is verified

### Requirement: Shared primitives define one visual and interaction grammar

Every recurring control MUST use a shared primitive or a documented semantic
extension. The system MUST define named roles for control size, spacing, radius,
border, depth, type, color, icon size, and motion.

Every interactive primitive MUST define all applicable rest, hover, focus-visible,
pressed, disabled, busy, selected, success, warning, and error states. An icon-only
or ambiguous control MUST have an accessible name and visible-on-hover-or-focus help.

#### Scenario: The same action appears in two product areas

- **WHEN** two surfaces present the same action at the same hierarchy level
- **THEN** both use the same primitive, size, shape, icon rule, and state behavior

#### Scenario: An icon-only control is reached by keyboard

- **WHEN** focus reaches an icon-only control
- **THEN** the control has a visible focus indicator, an accessible name, and a
  tooltip that is available from focus as well as hover

### Requirement: Content hierarchy stays consistent across the product

Every surface MUST use the shared type ramp, spacing sequence, content measure, and
surface hierarchy. A feature MUST NOT introduce an arbitrary type size, control
height, radius, border, shadow, or animation duration when a named role applies.

Dense data surfaces MAY use compact roles. They MUST keep labels legible and controls
large enough for the active pointer mode.

#### Scenario: A dense table and a settings form are compared

- **WHEN** both surfaces render
- **THEN** each uses the correct named density role
- **AND** equal hierarchy levels still share type, color, radius, and interaction
  semantics

### Requirement: State feedback is complete and calm

Every asynchronous or destructive workflow MUST present its applicable empty,
loading, busy, success, warning, error, offline, retry, and disabled states. Feedback
MUST state what happened and the next safe action. It MUST NOT depend on color,
animation, hover, or a transient toast alone.

#### Scenario: A visible operation fails

- **WHEN** a user action fails
- **THEN** the initiating surface keeps the user's work when possible
- **AND** it names the failure and a safe recovery action

### Requirement: Motion preserves continuity and respects preference

Motion MUST explain state, hierarchy, or spatial continuity. It MUST use shared
duration and easing roles. It MUST favor transform and opacity over layout-triggering
animation.

Reduced-motion mode MUST remove nonessential travel, bounce, shimmer, and rotation.
It MUST keep immediate state feedback and focus visibility.

#### Scenario: A panel opens with reduced motion enabled

- **WHEN** the user opens a panel under reduced motion
- **THEN** the panel appears without nonessential travel or scale
- **AND** focus moves to the correct destination

### Requirement: Every surface works across theme, input, window, and platform modes

Every reachable surface MUST remain usable in light, dark, high-contrast, forced
colors, keyboard-only, narrow-window, text-expanded, and relevant coarse-pointer
modes. Native window material MUST have an opaque fallback with equal legibility.

#### Scenario: A narrow high-contrast window is used by keyboard

- **WHEN** a user navigates a critical workflow in that mode
- **THEN** every control remains reachable, named, visible, and correctly ordered
- **AND** no action depends on hover or shadow alone

### Requirement: Premium quality has an evidence gate

The change MUST score at least 90 out of 100 on the weighted rubric in
`ux-matrix.md`. It MUST have no open critical or high-severity issue. Every reachable
surface row MUST be `Verified`, and the owner MUST complete the final acceptance
review.

A task checkbox MUST remain open until its listed automated test or verified harness
passes.

#### Scenario: Implementation looks complete but lacks state coverage

- **WHEN** a surface has only a resting-state screenshot
- **THEN** its row remains below `Verified`
- **AND** the product-wide change cannot close
