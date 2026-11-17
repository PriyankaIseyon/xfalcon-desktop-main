# Appearance Settings

## Why

The app has a complete light palette and a complete dark palette, and no
way to choose between them: `installSystemTheme` stamps `data-theme` from
the OS media query and that is the end of it. A user who runs a light
desktop cannot have a dark app, or the reverse.

Reading size is fixed the same way. `--xf-type-reading` is a single
`clamp()` tuned to web's `.prose-chat`, so the transcript reads at one
size on a given window width whatever the reader's eyes want.

Settings itself has no structure — three panels stacked under one heading,
with no statement of what each is for.

## What Changes

- **Theme is an explicit choice, dark or light.** The stored preference
  replaces the OS-follow behavior outright; `installSystemTheme` is
  deleted rather than kept as a third "system" mode, because a third mode
  is a state the owner did not ask for and a fallback path to maintain.
  The default is dark — the app's shipped look.
- **Font size is small, medium, or large**, and **small is the new
  default**. Today's fixed size becomes `medium`. The three sizes are one
  multiplier (`--xf-type-scale`) applied to every type token, so a size
  change moves the whole type ramp together instead of one token drifting
  away from the rest.
- **Both persist on this device** through `<app_local_data>/settings.json`,
  written atomically. The frontend never touches browser storage — that is
  a privilege the boundary test forbids — so both are read and written
  through Tauri commands.
- **Settings is grouped**, each group titled and described: Appearance,
  Providers, MCP Servers, Skills, Workspace.
- **Both controls are pill segmented buttons**, matching xfalcon-web's
  right-rail pills. `SegmentedControl` grows a `pill` variant rather than a
  second component being written beside it.

## Capabilities

### Added Capabilities

- `settings-appearance`: choosing and persisting theme and reading size.

### Modified Capabilities

- `app-typography`: the type ramp scales with the chosen reading size.

## Impact

- New: `crates/contracts/src/settings.rs`, `src-tauri/src/settings/`,
  `src/features/settings/AppearanceSection.tsx`, `src/app/useAppearance.ts`.
- Deleted: `src/app/theme.ts` (`installSystemTheme`) and its test.
- Changed: `src/styles/tokens.css`, `src/ui/SegmentedControl.tsx`,
  `src/features/settings/SettingsView.tsx`, `src/app/AppShell.tsx`,
  `src/lib/generated/contracts.ts` (regenerated).
- Two new Tauri commands: `settings_appearance_get`,
  `settings_appearance_set`.

## Not in this change

**A system-follow theme mode.** Two options were asked for; adding a third
that tracks the OS would reintroduce the media-query listener this change
removes.

**Per-conversation or per-surface type overrides.** One reading size for
the app.
