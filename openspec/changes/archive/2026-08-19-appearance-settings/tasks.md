# Tasks

## 1. Store

- [x] 1.1 `crates/contracts/src/settings.rs`: `ThemePreference`,
      `ReadingSize`, `AppearanceSettings` (defaults dark + small).
- [x] 1.2 `src-tauri/src/settings/`: JSON store at
      `<app_local_data>/settings.json`, atomic write, defaults on missing or
      unreadable file.
- [x] 1.3 `settings_appearance_get` / `settings_appearance_set` commands
      registered in `lib.rs`.
- [x] 1.4 Snapshot test + regenerate `src/lib/generated/contracts.ts`.

## 2. Apply

- [x] 2.1 `tokens.css`: `--xf-type-scale` multiplies every type token;
      `[data-font-size]` sets it. The reading `clamp()` keeps its shape.
- [x] 2.2 `src/app/useAppearance.ts` stamps `data-theme` and
      `data-font-size` on the document element; delete `theme.ts` and its test.

## 3. Surface

- [x] 3.1 `SegmentedControl` gains a `pill` variant (xfalcon-web right-rail
      pills: full-radius, accent-tinted when active).
- [x] 3.2 `AppearanceSection.tsx` — theme and reading size, both pills.
- [x] 3.3 `SettingsView.tsx` regrouped: Appearance, Providers, MCP Servers,
      Skills, Workspace, each titled and described.

## 4. Verification

- [x] 4.1 Component tests: default state, choosing each option, persistence
      call, corrupt-file fallback.
- [x] 4.2 `vitest run`, `tsc -b`, `cargo test --lib`, `cargo clippy -- -D warnings`.
- [x] 4.3 Live: switch theme and each size, restart, confirm both held.
