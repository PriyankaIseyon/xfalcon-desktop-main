# chat-motion-system Specification

## Purpose

Every animation the chat surface plays, where its timing comes from, and the one
rule that makes reduced motion safe: nothing the surface needs to know may be
reported only by an animation having played.

## Scope

| File                                        | What it owns here                                                        |
| ------------------------------------------- | ------------------------------------------------------------------------ |
| `src/styles/tokens.css:77`                  | the duration scale and the easing set                                    |
| `src/index.css:1803`                        | `.message-enter` and `xf-message-in`                                     |
| `src/index.css:1809`                        | `.badge-pop` and `xf-badge-pop`                                          |
| `src/index.css:1102`                        | `.streaming-caret` and its caret keyframes                               |
| `src/index.css:1073`                        | `.actions-slide` transitions                                             |
| `src/index.css:985`                         | `.ai-collapsible-content` height keyframes                               |
| `src/index.css:1821`                        | the global reduced-motion kill switch                                    |
| `src/features/chat/MessageList.tsx:65`      | `ENTER_REPLAY_GUARD_MS` and the entrance-eligibility rule                |
| `src/features/chat/MessageList.tsx:266`     | the appended-ids-only entrance bookkeeping                               |
| `src/features/chat/MessageRow.tsx:62`       | the caret belongs to the `stream-` row alone                             |
| `src/features/chat/chatStore.ts:186`        | `setStateCoalesced` — at most one notification per frame                 |
| `src/features/chat/ToolActivityCard.tsx:58` | `useDisclosureSettleReporter` — a settle that does not need an animation |

## Boundaries

- **What a disclosure's settle is used FOR** — the virtualizer's measurement cache
  and the remeasure listener — belongs to `chat-tool-trail`. This capability owns
  the requirement that the report is not conditioned on motion.
- **Which row is the streaming row** belongs to `chat-run-activity`. This
  capability owns only the caret that trails it.
- **The reserved height of the actions slot** belongs to `chat-thread-layout`;
  the fade that reveals them is here.
- **Skeleton and ghost shimmer** belong to `workspace-arrival`, which uses the
  same kill switch.
- **The token sheet's colour and type scales** belong to `app-typography` and
  `app-shell-material`; only the duration and easing scales are here.

## Requirements

### Requirement: Motion tokens are the only timing source

The token sheet SHALL define one duration scale — instant 80ms, fast 120ms, base
180ms, slow 240ms, page 320ms — and a closed set of easings: a soft ease-out, a
spring, a panel curve, and a material curve.

All chat-surface animations and transitions MUST reference these tokens. No
hard-coded duration and no inline cubic-bezier may appear in feature CSS.

#### Scenario: New animation uses tokens

- **WHEN** any chat-surface transition or keyframe is inspected
- **THEN** its duration and easing resolve to the shared tokens

### Requirement: New messages enter with a single rise animation

A newly appended message row SHALL play one entrance animation — opacity plus a
small upward translate, at the slow duration.

The entrance MUST play at most once per row. Eligibility MUST be decided from the
ids present on the previous render: only ids that were ADDED animate, and the
animation is suppressed entirely when any previous id vanished (a bulk swap, a
conversation open) or when more than a couple of ids appeared at once (a load
rather than an arrival).

The class MUST be retired shortly after it can have played, so a virtualization
re-mount later renders the row motionless. The retirement MUST use a one-shot
timer, never a repeating interval.

#### Scenario: Scroll-back does not replay entrances

- **GIVEN** a conversation with 50 messages where older rows were unmounted by
  virtualization
- **WHEN** the reader scrolls back to the top
- **THEN** re-mounted rows appear without the entrance animation

#### Scenario: Opening a conversation does not animate every row

- **GIVEN** an open conversation
- **WHEN** a different conversation is opened
- **THEN** no row plays the entrance

#### Scenario: The settle does not re-enter

- **GIVEN** a streamed prose row in the live tail
- **WHEN** its durable row replaces it
- **THEN** the durable row does not play the entrance

### Requirement: Streaming text shows a live caret

While an assistant response streams, the in-flight text SHALL end with a blinking
accent caret. The caret MUST belong to the synthetic in-flight row alone —
identified by that row's own identity, not by a guess from position — and MUST be
absent from every settled turn.

#### Scenario: Caret during and after streaming

- **GIVEN** an assistant response is streaming
- **WHEN** tokens append
- **THEN** a blinking caret trails the text
- **WHEN** the run completes and its durable row lands
- **THEN** no caret remains in the settled message

#### Scenario: A settled row above the stream has no caret

- **GIVEN** a run whose earlier round prose is on screen while the answer streams
- **THEN** only the arriving row carries the caret

### Requirement: Streaming settles without a double-paint

The handoff from the in-flight row to the durable row SHALL be visually seamless:
the streamed text does not disappear and reappear, does not flash, and does not
replay an entrance when the durable row takes over.

The text MUST be held across the gap between the run's terminal inference event
and the arrival of its durable row. That gap is real — the reconcile that produces
the durable row runs after the terminal event, with retries.

The hold MUST live in the store beside the text it continues, and MUST be released
by the arrival of the durable row rather than by a heuristic over message counts or
roles. A heuristic release can fire early, which blanks the answer, or late, which
shows two copies; both were observed.

A run that died MUST NOT have its partial text held: a partial answer is not an
answer, and the failure notice takes the surface instead.

#### Scenario: The gap is covered

- **GIVEN** a run whose terminal inference event has landed
- **WHEN** its durable row has not arrived yet
- **THEN** the answer is still on screen, unchanged

#### Scenario: The handover does not repaint

- **WHEN** the durable row arrives
- **THEN** the held text retires in the same commit, so the answer is never absent
  and never doubled

#### Scenario: A dead run holds nothing

- **GIVEN** a run that failed mid-answer
- **THEN** its partial text is dropped rather than held

### Requirement: Streamed text commits at most once per frame

Streamed assistant text arrives one token at a time. The store SHALL write each
delta to state synchronously — every reader, including the round-boundary capture
that freezes a round's prose, must see exact values — while notifying subscribers
at most once per animation frame, so the transcript re-renders and re-parses its
markdown once per frame rather than once per token.

Any non-delta state change MUST notify immediately and absorb a pending coalesced
notification, so a run's terminal event and the durable transcript that follows it
are never delayed behind the token stream.

#### Scenario: A burst of tokens costs one render

- **GIVEN** several text deltas arriving within one animation frame
- **THEN** subscribers are notified once, with the accumulated text

#### Scenario: A round boundary reads the exact streamed text

- **GIVEN** deltas that have not yet been flushed to subscribers
- **WHEN** the round's first tool invocation is requested
- **THEN** the captured prose carries all of the streamed text

#### Scenario: Terminal events are not delayed behind the stream

- **GIVEN** a pending coalesced notification
- **WHEN** the run's inference `completed` event arrives
- **THEN** subscribers are notified immediately

### Requirement: Copy actions confirm with a transient badge

Copy affordances in the chat surface SHALL confirm success by swapping to a check
indicator with a one-shot pop, reverting after roughly a second and a half. The
revert MUST use a one-shot timer, never a repeating interval.

A copy that fails MUST leave the affordance unconfirmed rather than showing a
check.

#### Scenario: Copy confirmation

- **WHEN** the reader clicks a message's copy button
- **THEN** the icon swaps to a check with a pop and reverts afterwards

#### Scenario: A failed copy does not claim success

- **GIVEN** a clipboard write that rejects
- **THEN** the affordance stays in its unconfirmed state

### Requirement: A one-shot animation does not replay on re-render

An animation that confirms a transition MUST be keyed to the transition it
reports — a status glyph changing, a copy confirming — so an unrelated re-render
does not replay it.

#### Scenario: No replay on re-render

- **GIVEN** a completed tool row with its status glyph
- **WHEN** the card re-renders for an unrelated event
- **THEN** the glyph does not re-animate

### Requirement: Reduced motion disables all chat animation

The chat surface SHALL disable every animation it introduces — entrances, caret
pulse, disclosure height, hover slides, badge pops, shimmer — whenever
`prefers-reduced-motion: reduce` is set, through one global kill switch that
suppresses animations outright and collapses transition durations to a hair.

Suppressing motion MUST remain purely presentational. No layout, no measurement,
and no state transition may depend on an animation having played: an animation
event MUST NOT be the only thing that reports a fact the surface needs, because
under reduced motion that event never arrives and the fact is simply lost. A
behaviour that listens for one MUST also have a path that runs when nothing
animates.

The kill switch MUST NOT be narrowed to repair such a dependency. Exempting an
element from it restores the motion the reader asked not to see, which trades a
broken surface for a disregarded preference.

#### Scenario: Reduced-motion pass

- **GIVEN** the OS reduced-motion setting is on
- **WHEN** messages stream, settle, and are copied
- **THEN** no transform-based animation plays anywhere in the chat surface

#### Scenario: Nothing is only reported by an animation

- **GIVEN** the OS reduced-motion setting is on
- **WHEN** the chat surface changes size, settles a run, or confirms an action
- **THEN** every consequence that would follow with motion on still follows,
  because no consequence was attached to an animation ending

### Requirement: A row's measured height settles whether or not motion played

A virtualized transcript row MUST report a settle whenever a disclosure inside it
changes that row's height, and the report MUST NOT be conditioned on an animation
having run. Under reduced motion the disclosure's height change is instantaneous
and complete, and the list MUST learn the new height then.

Whether to wait for an animation SHALL be determined by inspecting the disclosure
element itself — whether it is hidden, and whether any animation is declared for
its current state — never by reading the motion media query in script. The media
query is one of several reasons an animation does not run: keyframes missing from
a build, a platform that drops them, a test environment that does not run them. A
check that names only the media query leaves the others silently broken and
untestable.

The report MUST carry no height of its own; the listener reads the row's height
when it handles the report.

An animated settle MUST be measured after the animation, never during it, so no
intermediate frame's height is written as though it had settled.

#### Scenario: A collapse under reduced motion leaves no gap

- **GIVEN** the OS reduced-motion setting is on, and a mid-transcript assistant
  turn whose tool disclosure is expanded
- **WHEN** the reader collapses it
- **THEN** the row's measured height shrinks to its collapsed height and the rows
  after it sit flush

#### Scenario: An expand under reduced motion is measured at full height

- **GIVEN** the OS reduced-motion setting is on
- **WHEN** the reader expands a tool disclosure
- **THEN** the row is measured at its expanded height without waiting for an
  animation that will not play

#### Scenario: An animated settle is measured after the animation, not during

- **GIVEN** motion is enabled
- **WHEN** the reader expands a tool disclosure and its height animation runs
- **THEN** the height written for that row is the settled height

#### Scenario: A detached element answers "nothing will animate"

- **GIVEN** a disclosure body already removed from the document
- **THEN** it is treated as settled rather than as waiting for an animation

### Requirement: A settle survives the disclosure body being removed

The settle report MUST be dispatched from an element that outlives the disclosure
body. When a disclosure closes with no animation to wait for, its body is removed
from the document immediately, so a reporter bound only to that body is detached
before it can speak — and the collapse is the direction that leaves the gap, so
binding to the body means the failing case is the one that cannot report.

The element it falls back to MUST be captured while the body is still attached,
because the body cannot look up its own ancestors once removed.

#### Scenario: The body is gone before the settle is reported

- **GIVEN** a disclosure whose body is removed from the document as soon as it
  closes
- **WHEN** it closes
- **THEN** the settle still reaches the message list, dispatched from the row
  rather than from the removed body

### Requirement: A nested disclosure settles its row too

Toggling a detail panel inside a single tool-call row MUST report a settle for the
row that contains it, with motion on or off. That panel is its own disclosure and
the grouping shell does not observe its open state, so a repair that covers only
the grouping disclosure leaves every call row's detail panel able to open and close
without the list ever learning — which reads as fixed and is not.

One reporter MUST be able to serve the grouping disclosure and every nested panel
under it, because the cleanup is per attached node.

#### Scenario: A call row's detail panel toggles under reduced motion

- **GIVEN** the OS reduced-motion setting is on, and an expanded tool trail
  containing a call row with a detail panel
- **WHEN** the reader expands and then collapses that panel
- **THEN** the containing row is re-measured both times, and the rows after it sit
  flush

#### Scenario: A nested settle is attributed to the right row

- **GIVEN** a nested detail panel inside a virtualized row
- **WHEN** its settle is reported
- **THEN** the height written belongs to the row that contains the panel, not to a
  neighbouring row

## Provenance

Consolidated from the deltas of `polish-chat-visual-system`,
`retire-live-run-activity`, `one-surface-per-answer` and
`fix-reduced-motion-remeasure`, reconciled against the shipped code.

Notes on what the code actually does, where a delta said otherwise:

- The easing set has four curves, not two. `polish-chat-visual-system` named a soft
  ease-out and a spring; a panel curve and a material curve shipped alongside them.
- The kill switch suppresses animation outright and shortens transitions to
  0.01ms. It is not an "opacity-only" reduction.
- "The answer does not blank between streaming and durable" arrived as a MODIFIED
  delta in `one-surface-per-answer` with no ADDED predecessor. It is stated here in
  full as "Streaming settles without a double-paint".
