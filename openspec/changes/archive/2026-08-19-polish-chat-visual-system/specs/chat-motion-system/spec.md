# chat-motion-system — Spec Delta

## ADDED Requirements

### Requirement: Motion tokens are the only timing source

The token sheet SHALL define a duration scale (instant 80ms, fast 120ms,
base 180ms, slow 240ms, page 320ms) and two easings (a soft ease-out and a
spring). All chat-surface animations and transitions MUST reference these
tokens; no hard-coded durations or cubic-beziers in feature CSS.

#### Scenario: New animation uses tokens

- **WHEN** any chat-surface transition or keyframe is inspected
- **THEN** its duration and easing resolve to the shared tokens

### Requirement: New messages enter with a single rise animation

A newly appended message row SHALL play one entrance animation (opacity +
small upward translate, slow-token duration). The entrance MUST play at
most once per message: virtualization re-mounts, conversation re-opens, and
scroll-back MUST NOT replay it.

#### Scenario: Scroll-back does not replay entrances

- **GIVEN** a conversation with 50 messages where older rows were unmounted
  by virtualization
- **WHEN** the user scrolls back to the top
- **THEN** re-mounted rows appear without the entrance animation

### Requirement: Streaming text shows a live caret

While an assistant response streams, the in-flight text SHALL end with a
blinking accent caret. The caret exists only during streaming and is gone
in the persisted turn.

#### Scenario: Caret during and after streaming

- **GIVEN** an assistant response is streaming
- **WHEN** tokens append
- **THEN** a blinking caret trails the text
- **WHEN** the run completes
- **THEN** no caret remains in the settled message

### Requirement: Streaming settles without a double-paint

The handoff from the streaming container to the persisted message row SHALL
be visually seamless: the streamed text does not disappear-and-reappear,
flash, or re-play an entrance when the persisted row takes over.

#### Scenario: Settle handoff

- **GIVEN** a streaming response reaches its final token
- **WHEN** the run finalizes and the persisted row renders
- **THEN** the text remains continuously visible with no flash or re-animation

### Requirement: Copy actions confirm with a transient badge

Copy affordances in the chat surface SHALL confirm success by swapping to a
check indicator with a small pop animation, reverting after ~1.5 seconds.
The confirmation MUST NOT use `setInterval`.

#### Scenario: Copy confirmation

- **WHEN** the user clicks a message's copy button
- **THEN** the icon swaps to a check with a pop and reverts afterwards

### Requirement: Reduced motion disables all chat animation

The chat surface SHALL disable every animation introduced by this
capability (entrances, caret pulse, settle fade, hover slides, badge pops)
— or reduce it to opacity-only — whenever `prefers-reduced-motion: reduce`
is set, via the existing global kill switch.

#### Scenario: Reduced-motion pass

- **GIVEN** the OS reduced-motion setting is on
- **WHEN** messages stream, settle, and are copied
- **THEN** no transform-based animation plays anywhere in the chat surface
