# chat-thread-layout Specification

## Purpose

The reading surface of a conversation: the column it is measured in, how a user
turn differs from an assistant turn, where a response begins, which turn is
marked as newest, and the slot a turn's actions occupy.

Everything here is about a turn that has already been placed. What decides
placement is `chat-tool-trail` (grouping) and `chat-run-activity` (live versus
durable).

## Scope

| File                                    | What it owns here                                                |
| --------------------------------------- | ---------------------------------------------------------------- |
| `src/features/chat/MessageRow.tsx`      | the turn: user bubble, assistant prose, the actions slot         |
| `src/features/chat/UserMessage.tsx`     | the user's words, as an escaped text child                       |
| `src/features/chat/ResponseRibbon.tsx`  | the response boundary                                            |
| `src/features/chat/MessageList.tsx:423` | `latestDurableAssistantIndex` and the tail override              |
| `src/features/chat/MessageList.tsx:171` | `opensResponse` — the incremental ribbon decision                |
| `src/features/chat/runSurface.ts:28`    | `responseAlreadyOpen` — the same fact for the live tail          |
| `src/features/chat/ChatSurface.tsx`     | the section, the notices above the transcript, the composer dock |
| `src/styles/tokens.css:132`             | `--xf-chat-measure`                                              |
| `src/index.css:772`                     | `.chat-column-messages`, `.chat-column-composer`                 |
| `src/index.css:1006`                    | `.user-bubble`                                                   |
| `src/index.css:1014`                    | `.assistant-accent`, `.assistant-accent-latest`                  |
| `src/index.css:1046`                    | `.response-ribbon` and its parts                                 |
| `src/index.css:1073`                    | `.actions-slide`                                                 |

## Boundaries

- **The composer control itself** — its height, its send/stop affordance, its
  keyboard handling — belongs to `chat-composer`. This capability owns only the
  column the composer is measured in.
- **Motion**: entrance animations, the streaming caret, and the reduced-motion
  kill switch belong to `chat-motion-system`. This capability owns the fact that
  the actions slot reserves its height; the motion capability owns how the
  actions fade in.
- **The markdown pipeline and rich fence cards** belong to `chat-rich-content`.
  This capability places a prose row; it does not render its contents.
- **Which assistant row is newest while a run is in flight** is decided by
  `chat-run-activity`; this capability consumes that decision.
- **The titlebar carrying the conversation title** belongs to
  `app-shell-material`.
- **Virtualization and row measurement** belong to `chat-streaming-performance`
  and `chat-tool-trail`.

## Requirements

### Requirement: One token drives the reading column

The chat surface SHALL derive all horizontal sizing from a single
`--xf-chat-measure` token. Below the layout breakpoint both columns span the
window's width with a shared inline padding; at and above it the messages column
is capped at the measure times its own multiplier and the composer column at the
measure times its own, so the composer reads as sitting inside the reading column.

Each multiplier is that column's own decision. Widening the shared measure moves
both columns together and leaves their relationship untouched; neither column may
inherit the other's multiplier.

No component in the chat surface may carry an independent max-width cap. Exactly
one width rule applies to a chat column.

The column width MUST be applied by an explicit class on the element that owns it,
never by a positional selector. Rendering or removing a later sibling — the
tool-approval prompt, a notice — MUST NOT change the chat column's width.

The reading type size is set on the column, and UI chrome inside it (tool cards,
hints, badges) sets its own explicit sizes and is unaffected.

#### Scenario: Messages and composer share one measure

- **WHEN** the chat view renders at or above the layout breakpoint
- **THEN** the message list and the composer are centred on the same measure
  token, with the composer visibly narrower than the message column

#### Scenario: A narrow window uses the full width

- **WHEN** the chat view renders below the breakpoint
- **THEN** both columns span the available width with the shared inline padding
  and no cap

#### Scenario: No competing caps

- **WHEN** the chat column width is inspected
- **THEN** exactly one width rule applies to it, and no utility-class cap
  disagrees with it

#### Scenario: Approval appears without a layout jump

- **GIVEN** a chat thread rendered at the standard measure
- **WHEN** a tool-approval prompt appears below the chat surface
- **THEN** the chat column width is unchanged

### Requirement: User messages render as distinct right-aligned bubbles

User turns SHALL render right-aligned in a bubble with its own surface: dedicated
background and border tokens distinct from card surfaces, an asymmetric corner
radius whose reduced corner points at the speaker, and a width bounded to a
fraction of the message column — a narrower bound at and above the breakpoint than
below it.

The bubble MUST remain legible in every shipped theme at a text contrast ratio of
at least 4.5:1.

The user's text MUST be rendered as a text child and never as markup, preserving
its own line breaks. It must not be escaped a second time, or the entities
themselves appear on screen.

#### Scenario: User bubble is visually distinct

- **WHEN** a user message renders in dark mode
- **THEN** its bubble background and border are distinguishable from both the
  window background and elevated card surfaces

#### Scenario: Bubble width bounds

- **WHEN** a very long user message renders at or above the breakpoint
- **THEN** the bubble wraps at its bound and stays right-aligned

#### Scenario: A user message containing markup stays literal

- **GIVEN** a user message containing HTML and markdown syntax
- **THEN** the characters appear literally, once, with no rendered markup and no
  visible entities

### Requirement: Assistant messages are flat prose with a left rail

Assistant turns SHALL render full width without a bubble, as prose on the page,
marked by a vertical rail in the turn's left gutter. Alignment remains the primary
role signal; the rail is reinforcement and MUST NOT be the only channel
distinguishing roles.

There MUST be no permanent role header and no card around an answer.

#### Scenario: An assistant turn is prose, not a card

- **WHEN** an assistant turn renders
- **THEN** it spans the column, carries no bubble or box, and shows a rail in its
  left gutter

### Requirement: A response ribbon opens every response, exactly once

Every RESPONSE MUST open with a response boundary: an uppercase `RESPONSE` label
followed by a hairline rule that dissolves before the measure ends.

The boundary marks the handover from the user's turn to everything the assistant
produced for it, so it renders on the FIRST assistant prose row after a user turn
and MUST NOT be repeated for later prose rows of the same response.

A response can carry more than one prose row — a round's narration above its tool
umbrella, then the answer below it. Repeating the ribbon on each would mark two
starts for one response.

Both surfaces MUST answer "has this response already opened?" the same way. The
durable transcript maintains the fact incrementally as it walks its turns, because
asking per row would make the build quadratic; the live tail asks a shared
predicate over the durable rows. Both ranges the transcript builds start at a user
message or at the beginning, so both start with no response open.

The answer that follows a tool run and the answer that is a direct reply MUST both
open with the boundary, so the step into a response reads the same everywhere.

User turns MUST NOT carry it. Neither streamed reasoning nor a tool umbrella is an
answer, and neither carries it either.

The ribbon MUST be static — no entrance animation. The in-flight stream row and
the durable row that replaces it paint the identical ribbon, so the settle has
nothing to re-animate and the boundary never flashes.

The ribbon is decorative chrome and MUST be hidden from assistive technology: the
turn already carries its role, and repeating "Response" before every answer is
noise there.

#### Scenario: A tool-led response and a direct reply read the same

- **GIVEN** one response that narrated, called tools and answered, and one that
  answered directly
- **THEN** both open with exactly one response ribbon

#### Scenario: One response, one boundary

- **GIVEN** a settled response of prose₁, a tool umbrella, then the answer
- **WHEN** the transcript renders
- **THEN** the ribbon appears once, above prose₁
- **AND** the answer below the umbrella carries no second ribbon

#### Scenario: The live tail does not open a response the transcript already opened

- **GIVEN** a durable prose row that already opened this response
- **WHEN** the run's next prose row arrives in the live tail
- **THEN** the tail's row carries no ribbon

#### Scenario: The user's turn carries no ribbon

- **GIVEN** a user message
- **THEN** no response ribbon is rendered for it

#### Scenario: The settle does not flash the boundary

- **GIVEN** the in-flight stream row showing the ribbon
- **WHEN** its durable replacement lands
- **THEN** the ribbon is the same static paint, not a re-entrance

#### Scenario: Reasoning and tools carry no boundary of their own

- **GIVEN** a run that reasoned, called a tool, and answered
- **THEN** neither the reasoning disclosure nor the tool umbrella carries a ribbon

### Requirement: Only the newest answer is accented

The system MUST carry the brand accent on the newest assistant answer alone, on
both of its marks — the vertical rail and the response ribbon's rule. Every earlier
answer SHALL wear the neutral border colour on both.

The two marks MUST agree. A row whose rail is grey and whose rule is accent tells
the reader two different things about the same answer.

Exactly one turn holds the treatment at any moment. While the live tail is showing
this response's newest prose, the accent belongs there and no durable row may
carry it.

The accent belongs to answers only. A tool umbrella and a reasoning disclosure
MUST carry neither mark, whatever they hold. The accent is how a reader finds the
newest answer in a long transcript, and an accented card of raw thinking sitting
directly above the real answer would send them to the wrong text.

#### Scenario: A transcript of several answers

- **GIVEN** a transcript with three assistant answers
- **THEN** the newest carries accent on its rail and its rule, and the other two
  carry the neutral colour on both

#### Scenario: A new answer arrives

- **WHEN** a further answer lands
- **THEN** the accent moves to it and the previous answer turns neutral

#### Scenario: The live tail takes the accent while it streams

- **GIVEN** a run streaming prose in the live tail below a settled transcript
- **THEN** the tail's newest prose row is accented and no durable row is

#### Scenario: The umbrella is unmarked

- **GIVEN** a tool umbrella above the newest answer
- **THEN** the umbrella wears no accent, and the answer below it is still the only
  accented turn

### Requirement: The accent marks whisper

The system MUST keep both marks at a quarter-strength mix of their colour and one
device pixel of stroke. A sub-pixel stroke SHALL NOT be used: it does not paint on
this engine — pixel-sampled at device scale 1 and 2 the column comes back pure
background — so it would remove the mark rather than thin it.

The rail SHALL fade at both ends rather than running the full height of the turn,
and the ribbon's rule SHALL dissolve within roughly the first third of the measure.
The mark is where the answer starts, not a divider drawn across it.

#### Scenario: The rule fades early

- **THEN** the ribbon's rule is transparent well before the measure ends

#### Scenario: The rail is one device pixel

- **THEN** the rail's stroke is one pixel and its colour a quarter mix, not a
  sub-pixel box

### Requirement: Turn actions appear on hover in a reserved slot

A turn's actions MUST occupy a slot whose height is reserved whether or not the
actions are visible, so revealing them never moves the text around them.

Actions MUST be revealed by pointer hover over the turn and by keyboard focus
reaching them, and MUST be permanently visible on a coarse-pointer device, where
there is no hover to reveal them with.

Every action MUST be reachable by keyboard and MUST name itself to assistive
technology. A hover-only affordance is no affordance at all for a reader who does
not hover.

The transcript currently offers one action per turn: copy the turn's text, which
confirms by swapping to a check and reverting on its own.

#### Scenario: The transcript does not move when actions appear

- **WHEN** the pointer enters a turn
- **THEN** no text in the transcript changes position

#### Scenario: Every action is reachable without a pointer

- **WHEN** the reader moves focus through a turn
- **THEN** each of its actions takes focus and is announced by its own name

#### Scenario: A touch device shows them outright

- **GIVEN** a coarse pointer
- **THEN** the actions are visible without hover

### Requirement: Vertical rhythm follows the 4/8px grid

Spacing inside the thread SHALL follow a 4/8px scale: 8–12px between elements of
the same turn, 16–24px between turns.

#### Scenario: Turn spacing is on the scale

- **WHEN** the transcript renders
- **THEN** the gap between turns and the gaps inside a turn resolve to the scale

### Requirement: A row whose height changes is remeasured

A row that changes height in place MUST tell the transcript to remeasure it.

The virtualizer's measurement cache is keyed by row and MUST accept positive
measurements that grow or shrink. Keeping an old height after reading-size,
font, or width changes leaves gaps or positions later rows over the text.

The remeasure path is the one `chat-tool-trail` defines; any in-place height change
in a row uses it rather than inventing a second.

#### Scenario: An in-place height change does not overlap the rows below

- **GIVEN** a row in a long transcript whose content changes its height in place
- **THEN** the transcript remeasures that row
- **AND** no row overlaps another

#### Scenario: Reading size changes without leaving stale gaps

- **GIVEN** a mounted historical answer measured at the large reading size
- **WHEN** the reader changes to the small reading size
- **THEN** the row shrinks and subsequent rows remain adjacent
- **AND** previously unmounted rows refresh their measurements when revisited

### Requirement: Streaming preserves the reader's scroll intent

While the reader follows the newest response, content growth MUST keep its end
visible. A delayed scroll event from a programmatic bottom correction MUST NOT
be mistaken for the reader leaving the bottom merely because more text arrived.
Upward reader scrolling MUST release following, and the jump control MUST restore
it. Browser scroll anchoring MUST NOT compete with automatic bottom corrections.

#### Scenario: A large streamed answer remains in view

- **WHEN** a response arrives in large bursts without reader scrolling
- **THEN** the view continues to follow its latest text
- **AND** the same behavior holds with reduced motion enabled

#### Scenario: Reading history during a stream

- **WHEN** the reader scrolls up while an answer is arriving
- **THEN** subsequent text does not pull the view back to the end
- **AND** activating the new-response control restores following

### Requirement: The transcript carries no header of its own

The reading surface MUST be the transcript, then the run in flight, then the
composer, and MUST NOT repeat the conversation's title in a second header above the
transcript. The title lives in the application titlebar.

Cancelling a run lives in the composer's send/stop control and MUST NOT be a
separate pill above the transcript.

Notices about the conversation — an offline copy, an unavailable project folder —
render above the transcript in the messages column. A notice about the RUN renders
in the scroll flow where that run's turn lands, not pinned above the transcript, so
the reader does not have to scroll away from the failure to see it.

#### Scenario: No second header

- **WHEN** a conversation renders
- **THEN** the transcript is not boxed under a title header

#### Scenario: A run's failure renders where the run was

- **GIVEN** a run that died
- **THEN** its notice renders after the run's blocks in the scroll flow

### Requirement: The empty conversation is a welcome, not an empty transcript

A conversation with no turns and no live run SHALL present the welcome surface —
the mark, a personalized greeting, the composer centred between them, then the
starter suggestions — rather than an empty transcript with a docked composer.

Anything the run has produced counts as content: a run that reaches the screen
before its optimistic user row lands MUST show the transcript, not the welcome
screen, and that question MUST be asked through the same predicate the live tail
uses.

#### Scenario: A new chat welcomes

- **GIVEN** a conversation with no messages and no run
- **THEN** the welcome surface renders with the composer centred

#### Scenario: A run that arrives first still shows the transcript

- **GIVEN** a submitted message whose run has produced a block before any row
  landed
- **THEN** the transcript renders, not the welcome surface

## Provenance

Consolidated from the deltas of `polish-chat-visual-system`,
`refine-composer-and-response-boundary`, `leaner-response-accents`,
`chat-reasoning-panel` and `refactor-run-timeline-classification`, reconciled
against the shipped code and resolved in that order.

Resolved contradictions, all in favour of the code:

- "The assistant accent rail is a sub-pixel hairline"
  (`refine-composer-and-response-boundary`) is removed. It required 0.5px;
  `leaner-response-accents` established that a sub-pixel box does not paint on
  this engine, and the rail is one device pixel at a quarter mix.
- The measure multipliers named by `polish-chat-visual-system` (×1.05 and ×0.90)
  and the 1280px step are not what shipped. There is one breakpoint, and the
  multipliers are each documented at their own rule. The comment on
  `--xf-chat-measure` in `tokens.css:126` still names the old pair and is stale.
- `add-turn-revision` carries MODIFIED deltas for "Turn actions appear on hover in
  a reserved slot" and "A row whose height changes is remeasured". Both
  requirements are stated above as the code has them today — one action, copy —
  so that delta now has a base to apply onto.
