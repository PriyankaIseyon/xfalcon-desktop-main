# workspace-arrival — Spec Delta

## ADDED Requirements

### Requirement: The authenticated shell arrives whole

After authentication completes, the shell SHALL present a workspace
skeleton — ghost sidebar rows, ghost transcript column, and a centered
"Loading your workspace…" indicator — until the workspace is ready, then
reveal the real UI in one transition. Readiness means the workspace
bootstrap (conversation list) has resolved AND, when a conversation
auto-opens, its conversation view has been applied.

#### Scenario: Skeleton until ready

- **GIVEN** sign-in just completed
- **WHEN** the bootstrap and initial conversation view are still in flight
- **THEN** the skeleton renders (no partially-populated shell)
- **WHEN** both resolve
- **THEN** the real sidebar and transcript replace the skeleton together

#### Scenario: Hard cap

- **GIVEN** the bootstrap hangs
- **WHEN** 5 seconds elapse from skeleton mount
- **THEN** the shell reveals with whatever state exists (the skeleton can
  never hold the app hostage)

#### Scenario: Reduced motion

- **GIVEN** `prefers-reduced-motion: reduce`
- **WHEN** the skeleton renders
- **THEN** ghosts are static (no shimmer animation)

#### Scenario: No skeleton on in-session reconciles

- **GIVEN** the workspace already revealed
- **WHEN** a sidebar refresh or conversation reconcile runs
- **THEN** the skeleton does not reappear
