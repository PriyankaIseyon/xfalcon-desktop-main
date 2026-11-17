# chat-motion-system — Spec Delta

## MODIFIED Requirements

### Requirement: Reduced motion disables all chat animation

The chat surface SHALL disable every animation introduced by this
capability (entrances, caret pulse, settle fade, hover slides, badge pops)
— or reduce it to opacity-only — whenever `prefers-reduced-motion: reduce`
is set, via the existing global kill switch.

Suppressing motion MUST remain purely presentational. No layout, no
measurement, and no state transition may depend on an animation having
played: an animation event MUST NOT be the only thing that reports a fact
the surface needs, because under reduced motion that event never arrives
and the fact is simply lost. A behaviour that listens for one MUST also
have a path that runs when nothing animates.

The kill switch MUST NOT be narrowed to repair such a dependency. Exempting
an element from it restores the motion the user asked not to see, which
trades a broken surface for a disregarded preference.

#### Scenario: Reduced-motion pass

- **GIVEN** the OS reduced-motion setting is on
- **WHEN** messages stream, settle, and are copied
- **THEN** no transform-based animation plays anywhere in the chat surface

#### Scenario: Nothing is only reported by an animation

- **GIVEN** the OS reduced-motion setting is on
- **WHEN** the chat surface changes size, settles a run, or confirms an
  action
- **THEN** every consequence that would follow with motion on still
  follows, because no consequence was attached to an animation ending

## ADDED Requirements

### Requirement: A row's measured height settles whether or not motion played

A virtualized transcript row MUST report a settle to the message list
whenever a disclosure inside it changes that row's height, and the report
MUST NOT be conditioned on an animation having run. Under
`prefers-reduced-motion: reduce` the disclosure's height change is
instantaneous and complete, and the list MUST learn the new height then —
the grow-only measurement cache never shrinks on its own, so an unreported
collapse leaves a permanent gap in the transcript for exactly the users who
asked for less motion.

Whether to wait for an animation SHALL be determined by inspecting the
disclosure element itself, never by reading the motion media query in
script. The media query is one of several reasons an animation does not
run — keyframes missing from a build, a platform that drops them, a test
environment that does not run them — and a check that names only the media
query leaves the others silently broken and untestable.

The report MUST carry no height of its own. The listener SHALL read the
row's height at the moment it handles the report, so a settle reported more
than once for one toggle converges on the same measurement instead of two
writers disagreeing.

#### Scenario: A collapse under reduced motion leaves no gap

- **GIVEN** the OS reduced-motion setting is on, and a mid-transcript
  assistant turn whose tool disclosure is expanded
- **WHEN** the user collapses it
- **THEN** the row's measured height shrinks to its collapsed height and
  the rows after it sit flush

#### Scenario: An expand under reduced motion is measured at full height

- **GIVEN** the OS reduced-motion setting is on
- **WHEN** the user expands a tool disclosure
- **THEN** the row is measured at its expanded height without waiting for
  an animation that will not play

#### Scenario: An animated settle is measured after the animation, not during

- **GIVEN** motion is enabled
- **WHEN** the user expands a tool disclosure and its height animation runs
- **THEN** the height written for that row is the settled height, and no
  intermediate frame's height is written as if it were settled

#### Scenario: The same settle reported twice

- **GIVEN** a single disclosure toggle that produces two settle reports
- **THEN** the row's height is written from a fresh measurement each time
  and ends correct, rather than one report undoing the other

### Requirement: A settle survives the disclosure body being removed

The settle report MUST be dispatched from an element that outlives the
disclosure body. When a disclosure closes with no animation to wait for,
its body is removed from the document immediately, so a reporter bound only
to that body is detached before it can speak — and the collapse is the
direction that leaves the gap, so binding to the body means the failing
case is the one that cannot report.

#### Scenario: The body is gone before the settle is reported

- **GIVEN** a disclosure whose body is removed from the document as soon as
  it closes
- **WHEN** it closes
- **THEN** the settle still reaches the message list, dispatched from the
  row rather than from the removed body

### Requirement: A nested disclosure settles its row too

Toggling a detail panel inside a single tool-call row MUST report a settle
for the row that contains it, with motion on or off. That panel is its own
disclosure and the grouping shell does not observe its open state, so a
repair that covers only the grouping disclosure leaves every call row's
Parameters/Result panel able to open and close without the list ever
learning — which reads as fixed and is not.

#### Scenario: A call row's detail panel toggles under reduced motion

- **GIVEN** the OS reduced-motion setting is on, and an expanded tool trail
  containing a call row with a detail panel
- **WHEN** the user expands and then collapses that panel
- **THEN** the containing row is re-measured both times, and the rows after
  it sit flush

#### Scenario: A nested settle is attributed to the right row

- **GIVEN** a nested detail panel inside a virtualized row
- **WHEN** its settle is reported
- **THEN** the height written belongs to the row that contains the panel,
  not to a neighbouring row
