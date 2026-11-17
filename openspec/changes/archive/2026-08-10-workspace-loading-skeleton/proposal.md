# Workspace Loading Skeleton

## Why

Right after sign-in the shell renders piecemeal: empty sidebar, then
conversations pop in, then the last conversation's transcript. The user
asked for a proper arrival: a skeleton loading state ("Loading your
workspace…") that holds the frame until the workspace is actually ready,
so the reveal presents complete, current information at once.

## What Changes

- A full-shell skeleton state after authentication completes and before
  the workspace is ready: ghost sidebar rows, ghost transcript column, a
  centered "Loading your workspace…" line with the brand mark.
- "Ready" = workspace bootstrap resolved (conversation list) AND, when a
  conversation auto-opens, its first conversation view applied. The
  skeleton is readiness-driven, not timer-driven; a hard cap (5 s)
  force-reveals whatever has arrived so a slow call can never hold the
  shell hostage.
- Skeleton shimmer respects `prefers-reduced-motion` (static ghosts, no
  shimmer) per the chat motion system.
- Sign-out → sign-in transitions replay the skeleton; in-session
  reconciles never do.

Non-goals: artificially extending the skeleton to a fixed duration
(5 s is a CAP, not a floor — the user's "5 seconds" reads as "long enough
to cover the loads"); skeletonizing individual later loads (tool cards,
message pagination); changing what the bootstrap fetches.

## Capabilities

### New Capabilities

- `workspace-arrival`: the authenticated shell's loading presentation —
  readiness definition, skeleton anatomy, reveal rules, reduced-motion
  behavior.

## Impact

- Affected code: `src/app/AppShell.tsx` (gate), new
  `src/features/chat/WorkspaceSkeleton.tsx`, `src/index.css` (ghost +
  shimmer styles), `useChat` (readiness signal already derivable from
  store state — expose it).
- Tests: skeleton renders during load, reveals on readiness, cap fires,
  reduced-motion variant.
