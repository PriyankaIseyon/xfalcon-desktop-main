# Tasks for Power Composer & Minimalist Shell Polish

- [x] 1. Power Composer Input System
  - [x] 1.1 Add trigger listener for `@` key in `Composer.tsx` rendering fuzzy dropdown for project files, MCP tools, and reports.
  - [x] 1.2 Add trigger listener for `/` key in `Composer.tsx` rendering slash command menu (`/clear`, `/skill`, `/project`, `/mcp`, `/help`). `/skill` and `@skill:<name>` attach an imported skill to the draft.
  - [x] 1.3 Implement token estimation counter and context capacity bar above the composer.

- [x] 2. Minimalist Visual Shell & Native Window Effects
  - [x] 2.1 SUPERSEDED — native Mica/Vibrancy is now owned by
        `update-product-wide-premium-ux` task 8.3. It must first make the
        product decision, then prove opaque, reduced-transparency,
        unsupported-platform, and initialization-failure behavior as one
        native boundary contract.
  - [x] 2.2 SUPERSEDED — the token palette was already restyled by `polish-chat-visual-system`, archived 2026-08-19; see `app-shell-material` and `chat-thread-layout`.
  - [x] 2.3 Implement Zen / Focus mode toggle shortcut (`Cmd+Shift+F` / `Ctrl+Shift+F`) collapsing all sidebars.

- [x] 3. Keyboard Shortcuts Overlay & Polish
  - [x] 3.1 Build global shortcut cheat sheet dialog (`Cmd+/` / `Ctrl+/`).
  - [x] 3.2 SUPERSEDED — completion sounds are now owned by
        `update-product-wide-premium-ux` task 8.4. That task decides whether
        sound belongs in the product before it can add a setting, default, and
        accessibility evidence.

- [x] 4. Verification & Tests
  - [x] 4.1 Unit tests for `@` mention parsing and `/` slash command selection handlers.
  - [x] 4.2 SUPERSEDED — the fallback suite is part of
        `update-product-wide-premium-ux` task 8.3, after the material decision.
