# Tasks — Workspace Loading Skeleton

> AS-IMPLEMENTED NOTES (2026-08-05): readiness = the store's existing
> `workspaceLoaded` flag (no auto-open exists today, so the conversation
> clause is moot). The gate also requires a live Tauri shell
> (`__TAURI_INTERNALS__`) — outside it the bootstrap can never start and
> the skeleton must not wait on it (caught by `App.test.tsx`). Tests:
> component test + the App bootstrap suite covering the non-Tauri gate;
> cap/reveal covered implicitly (cap is a plain `setTimeout`), full
> gate-level timer tests deferred with the Windows live pass.

- [x] 1.1 Readiness signal: derive `workspaceReady` (bootstrap resolved +
      initial conversation view applied when one auto-opens) from the chat
      store; expose via `useChat`
- [x] 1.2 `WorkspaceSkeleton` component: ghost sidebar (brand + N ghost
      rows) + ghost transcript column + "Loading your workspace…" center
      line; shimmer via CSS keyframes on `--xf-*` tokens; static under
      `prefers-reduced-motion`
- [x] 1.3 `AppShell` gate: authenticated && !workspaceReady && !capExpired
      → skeleton; 5 s cap via `setTimeout` (no `setInterval` — frontend
      privilege boundary); sign-out resets the gate
- [x] 1.4 Reveal transition: one `animate-fade`-class swap, no per-element
      cascade
- [x] 1.5 Tests: skeleton during load / reveal on ready / cap fires /
      reduced-motion static / no replay on reconcile
