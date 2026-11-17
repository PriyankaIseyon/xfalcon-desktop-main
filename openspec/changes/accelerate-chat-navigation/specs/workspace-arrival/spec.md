## MODIFIED Requirements

### Requirement: The authenticated shell arrives whole

After authentication completes, the shell SHALL start workspace status and chat
workspace bootstrap concurrently. Neither request SHALL wait for the other to
settle before it starts.

The shell SHALL present a workspace skeleton — ghost sidebar rows, ghost
transcript column, and a centered "Loading your workspace…" indicator — until
the first arrival is ready, then reveal the real UI in one transition. For a
chat-ready workspace, readiness means both independent startup requests have
resolved and any auto-opened conversation view has been applied. For a workspace
that resolves as not chat-ready, readiness means workspace status has resolved;
the speculative chat result MUST NOT hold the shell or surface a chat failure.

All startup results SHALL be scoped to the authenticated account generation. A
late result from an earlier account MUST NOT populate the current workspace.

A successful chat workspace bootstrap SHALL establish chat connectivity even
when the parallel workspace-status summary fails. An explicit not-chat-ready
status SHALL remain authoritative and SHALL keep chat actions disabled.

After the conversation list resolves, the bounded startup-recovery conversation
set SHALL perform its independent cloud reconciles concurrently. Local
projection mutation and zombie finalization SHALL remain ordered.

#### Scenario: Startup requests overlap

- **GIVEN** sign-in just completed in the desktop shell
- **WHEN** neither workspace status nor chat bootstrap has resolved
- **THEN** both native commands have already been invoked
- **AND** resolving either one alone does not reveal a chat-ready workspace

#### Scenario: Skeleton until ready

- **GIVEN** a chat-ready workspace whose startup requests are in flight
- **WHEN** one request resolves and the other remains in flight
- **THEN** the skeleton remains
- **WHEN** both resolve and any initial conversation is applied
- **THEN** the real sidebar and transcript replace the skeleton together

#### Scenario: Bounded recovery overlaps cloud reads

- **GIVEN** several recent conversations require startup recovery
- **WHEN** their cloud reconciles start
- **THEN** each reconcile starts without waiting for the prior reconcile to finish
- **AND** no more than the fixed startup-recovery limit are in flight concurrently
- **AND** local recovery mutations remain ordered

#### Scenario: Not-ready workspace ignores speculative chat failure

- **GIVEN** chat bootstrap was attempted while workspace status was pending
- **WHEN** chat bootstrap fails and status resolves as not chat-ready
- **THEN** the normal shell reveals
- **AND** the speculative bootstrap error is not shown as a chat error

#### Scenario: Status-summary failure does not erase a proven chat connection

- **GIVEN** chat workspace bootstrap succeeds for the current account
- **WHEN** the parallel workspace-status summary fails
- **THEN** the conversation collection is applied
- **AND** chat actions remain enabled
- **AND** the composer does not say that chat is disconnected

#### Scenario: Hard cap

- **GIVEN** either startup request hangs
- **WHEN** 5 seconds elapse from skeleton mount
- **THEN** the shell reveals with whatever state exists
- **AND** the skeleton does not hold the app hostage

#### Scenario: Reduced motion

- **GIVEN** `prefers-reduced-motion: reduce`
- **WHEN** the skeleton renders
- **THEN** ghosts are static

#### Scenario: No skeleton on in-session reconciles

- **GIVEN** the workspace already revealed
- **WHEN** a sidebar refresh or conversation reconcile runs
- **THEN** the full workspace skeleton does not reappear

#### Scenario: Account changes during startup

- **GIVEN** startup requests belong to account A
- **WHEN** the user signs out and account B becomes current before those requests resolve
- **THEN** account A results do not update account B state
