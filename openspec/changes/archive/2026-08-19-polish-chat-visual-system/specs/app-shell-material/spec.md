# app-shell-material — Spec Delta

## ADDED Requirements

### Requirement: Dark theme uses two surface levels

The dark palette SHALL distinguish a base window surface from a brighter
elevated surface used by cards, the composer, popovers, and menus. Elevated
surfaces MUST be lighter than the base (never darker, never pure black),
and body text on both levels MUST meet ≥ 4.5:1 contrast.

#### Scenario: Elevation is visible in dark mode

- **WHEN** the app renders in dark mode
- **THEN** the composer and cards are visibly lighter than the window
  background, without relying on borders alone

### Requirement: Sidebar selection and hover follow the accent system

The sidebar SHALL show the selected item as an accent-tinted rounded
rectangle with accent-colored label/icon, and hovered items with a quiet
neutral fill. Selection state MUST be visible in light, dark, and
high-contrast themes.

#### Scenario: Selected conversation is obvious

- **WHEN** a conversation is active
- **THEN** its sidebar row shows the accent-tinted selected treatment and
  all other rows do not

### Requirement: Keyboard focus is always visible

The shell SHALL show a visible `:focus-visible` ring (accent, offset from
the element) on every interactive element in the shell and chat surface
(sidebar rows, composer, send/stop control, message actions, disclosure
headers). Outlines MUST NOT be removed without an equivalent replacement.

#### Scenario: Tab order is traceable

- **WHEN** the user tabs through the chat view
- **THEN** each stop shows a visible focus ring

### Requirement: Window material is optional and fallback-safe

The shell MUST render the current opaque token background — with no
functional or legibility difference — whenever OS window translucency
(Windows mica/acrylic, macOS vibrancy) is unavailable (unsupported OS, user
setting, initialization failure). When translucency is enabled, the sidebar
MAY render on a translucent tinted background.

#### Scenario: Material unavailable

- **GIVEN** the OS reports no translucency support
- **WHEN** the app boots
- **THEN** the sidebar renders opaque exactly as before this change

#### Scenario: Material active preserves legibility

- **GIVEN** OS translucency is active
- **WHEN** bright content sits behind the window
- **THEN** sidebar text still meets ≥ 4.5:1 contrast (tint opacity high
  enough to guarantee it)
