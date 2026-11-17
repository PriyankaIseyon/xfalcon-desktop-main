# app-shell-material Specification

## Purpose

The shell's surface system: how elevation is expressed in each theme, how the
sidebar shows selection, and the focus ring that must survive every restyle.

## Scope

| File                               | What it owns here                                               |
| ---------------------------------- | --------------------------------------------------------------- |
| `src/styles/tokens.css:28`         | the light palette's surface levels                              |
| `src/styles/tokens.css:175`        | the dark palette, and the ordering rule its overrides depend on |
| `src/styles/tokens.css:59`, `:205` | `--xf-focus` per theme                                          |
| `src/index.css:48`                 | the global `:focus-visible` ring                                |
| `src/index.css:1682`               | the high-contrast focus overrides                               |
| `src/app/AppShell.tsx`             | the three-track workspace and which view is on screen           |

## Boundaries

- **Type roles and the faces** belong to `app-typography`.
- **Motion tokens** belong to `chat-motion-system`.
- **The sidebar's tree structure and keyboard model** belong to `sidebar-hierarchy`;
  this capability owns only how a selected row is painted.
- **The chat reading column** belongs to `chat-thread-layout`.
- **Zen mode and OS window effects** are proposed by
  `add-power-composer-and-shell-polish`; nothing of either ships today.

## Requirements

### Requirement: Dark theme uses two surface levels

The dark palette SHALL distinguish a base window surface from brighter elevated
surfaces used by cards, the composer, popovers, and menus. Elevated surfaces MUST be
lighter than the base — never darker, never pure black — and body text on both levels
MUST meet a contrast ratio of at least 4.5:1.

The theme overrides MUST stay after the base declarations they override, because the
palette relies on declaration order rather than on specificity.

#### Scenario: Elevation is visible in dark mode

- **WHEN** the app renders in dark mode
- **THEN** the composer and cards are visibly lighter than the window background,
  without relying on borders alone

### Requirement: Sidebar selection and hover follow the accent system

The sidebar SHALL show the selected item as an accent-tinted rounded rectangle with an
accent-coloured label and icon, and hovered items with a quiet neutral fill.

Selection MUST be visible in light, dark, and high-contrast themes.

#### Scenario: Selected conversation is obvious

- **WHEN** a conversation is active
- **THEN** its sidebar row shows the accent-tinted treatment and no other row does

#### Scenario: Main titles are not selected rows

- **WHEN** Chat, Projects, Reports, or Settings is shown
- **THEN** each uses the same title type and colour
- **AND** none of them uses the accent-tinted selected fill

### Requirement: Keyboard focus is always visible

The shell SHALL show a visible focus ring — accent-coloured, offset from the element —
on every interactive element in the shell and the chat surface: sidebar rows, the
composer, the send and stop control, message actions, disclosure headers.

An outline MUST NOT be removed without an equivalent replacement. A container that
takes focus on behalf of its rows MUST show the ring on itself and mark its cursor row
separately.

#### Scenario: Tab order is traceable

- **WHEN** the reader tabs through the chat view
- **THEN** each stop shows a visible focus ring

#### Scenario: A focus container rings itself

- **GIVEN** a tree that holds focus for its rows
- **THEN** the container shows the ring and the cursor row is marked distinctly

### Requirement: The shell renders opaque, and translucency is not assumed

The shell MUST render its opaque token background, with no functional or legibility
difference, whenever OS window translucency is unavailable — an unsupported OS, a
reader's setting, or an initialization failure.

No OS translucency is implemented in this build: no vibrancy or mica dependency is
declared and none is applied at startup, so the opaque path is the only path that
runs. Should translucency be added, the sidebar MAY render on a translucent tinted
background, and the tint MUST be opaque enough that sidebar text still meets 4.5:1
over bright content behind the window.

#### Scenario: Material unavailable

- **GIVEN** the OS reports no translucency support
- **WHEN** the app boots
- **THEN** the shell renders fully opaque with no missing surface

#### Scenario: No translucency is attempted

- **WHEN** the build's window setup is inspected
- **THEN** no OS material is requested, so no failure path can be reached

### Requirement: Fine paired surface boundaries

Major shell panes SHALL have 1px theme-aware borders separated by a 1px gap,
with a 4px corner radius that keeps rounded junctions compact.
Page and artifact headers SHALL repeat this paired horizontal boundary.
Floating menus, command suggestions, and dialogs SHALL use a 1px border plus a
1px outline with a 2px offset, without changing their measured dimensions.
Ordinary controls and content rows SHALL retain their own existing styles.

#### Scenario: Shared shell across destinations

- **WHEN** the user visits chat, projects, datasets, reports, or settings
- **THEN** the shared navigation and workspace have the same fine pane boundaries
- **AND** a closed artifact panel leaves no empty column or extra gutter

#### Scenario: Aligned headers and bounded scroll regions

- **WHEN** the user opens or resizes an artifact beside the workspace
- **THEN** the page and artifact headers have matching top and bottom edges
- **AND** their content frames have matching top and bottom edges
- **AND** the sidebar brand is centered on the same header row
- **AND** each content renderer owns its scrolling without an additional scrolling panel wrapper
- **AND** app scroll regions and HTML previews use thin neutral scrollbars by default

#### Scenario: Discoverable resize divider

- **WHEN** an artifact panel is open
- **THEN** its divider shows a centered three-dot grip before hover
- **AND** hover and keyboard focus highlight the grip
- **AND** fixed pane boundaries do not show resize grips

## Provenance

Consolidated from the delta of `polish-chat-visual-system`, reconciled against the
code. That delta's window-material requirement is restated as the opaque contract it
actually describes: task 5.4 of the change is marked deferred, and no translucency
dependency is declared.
