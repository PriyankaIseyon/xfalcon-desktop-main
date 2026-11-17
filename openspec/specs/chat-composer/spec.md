# chat-composer Specification

## Purpose

The one control a reader uses to say something: its shell, its growth, its
keyboard contract, and the fact that the same circular button both sends a message
and stops the run that message started.

## Scope

| File                                 | What it owns here                                 |
| ------------------------------------ | ------------------------------------------------- |
| `src/features/chat/Composer.tsx`     | the control: shell, autosize, keyboard, send/stop |
| `src/features/chat/Composer.tsx:10`  | `COMPOSER_MAX_HEIGHT_PX`                          |
| `src/features/chat/ComposerDock.tsx` | the draft binding, and the only subscriber to it  |
| `src/index.css`                      | `.composer` — the card and its focus border       |
| `src/index.css:139`                  | the placeholder's own type and ink                |
| `src/index.css:148`                  | the accent caret                                  |
| `src/index.css:152`                  | `.composer-dock` and the dock fade                |

## Boundaries

- **The draft store itself** belongs to `chat-streaming-performance`, which owns
  the requirement that typing costs the transcript nothing. This capability owns
  the binding.
- **The column the composer is measured in** belongs to `chat-thread-layout`.
- **Whether a run is in flight** is `isRunPending`, owned by `chat-run-activity`.
  The composer reads it and defines no predicate of its own.
- **What cancelling a run does** belongs to `run-execution`; the composer owns only
  the affordance that asks for it.
- **The welcome screen's centred placement of this same control** belongs to
  `chat-thread-layout`.
- **Attachments** are not in this baseline. The attach affordance is present and
  permanently disabled; `add-power-composer-and-shell-polish` proposes what would
  make it live.

## Requirements

### Requirement: Minimal composer surface

The composer SHALL present as a single soft-rounded card holding a
borderless multi-line input, then one action row with an attach affordance at the
left and a circular filled send control at the right.

No model badge, border-heavy input, or extra label may appear inside the card. The
active-project caption is the sole quiet annotation above the input, and it carries
no trust or approval semantics — it never gates typing or submission.

The attach affordance is present and disabled, and says so in its accessible name.
A control that looks live and does nothing is worse than one that admits it is not
ready.

#### Scenario: Resting appearance

- **WHEN** the chat view renders with no draft
- **THEN** the composer is one rounded card with its placeholder, an attach
  control, and a circular send control disabled while empty

#### Scenario: The project caption does not gate anything

- **GIVEN** an active project named above the input
- **THEN** typing and submission behave exactly as they do without one

### Requirement: The composer names the task in the product's language

The composer's placeholder MUST name the task in the product's own terms whenever
the input accepts typing, including while a run is in flight. When the secure
runtime is unavailable the placeholder says so instead, and typing is off.

The placeholder MUST render one step down from the reading size and on the faintest
ink, so it recedes against a draft typed over it rather than competing with it, and
it MUST NOT rely on a browser's default placeholder dimming — the token is the
value.

The input MUST carry a programmatic label and a keyboard hint for assistive
technology, and the hint MUST say why input is unavailable when it is.

#### Scenario: The resting composer

- **GIVEN** a connected chat surface
- **THEN** the composer's placeholder invites a question about the reader's data

#### Scenario: The placeholder does not change under a running turn

- **GIVEN** a run in flight
- **THEN** the placeholder is unchanged and keystrokes land in the draft

#### Scenario: A disconnected runtime says so

- **GIVEN** the secure runtime is unavailable
- **THEN** the placeholder says chat is not connected, typing is off, and the hint
  explains when it becomes available

### Requirement: The composer scrolls only once the draft outgrows it

The input MUST grow with the draft up to a fixed cap and MUST NOT present a
scrollbar below that cap — an empty composer in particular. Scrolling SHALL turn on
at the line the draft outgrows the cap and off again when the draft shrinks back
inside it.

Growth MUST be measured from a collapsed box, so the measurement is of the content
rather than of the box's current height.

The input's line box MUST be an integer height. The reading size is a fluid clamp,
and a fractional line box makes the measured integer height fall short of the
content, which is what surfaces a scrollbar on a composer with nothing in it.

Resizing MUST NOT fight IME composition: no resize happens mid-composition, and one
settling pass runs when composition ends.

The input MUST return to its one-line height after a send.

#### Scenario: An empty composer has no scrollbar

- **GIVEN** a composer with an empty draft
- **THEN** its input does not scroll

#### Scenario: Growth up to the cap stays scrollbar-free

- **GIVEN** a draft of a few lines, still under the cap
- **THEN** the input is exactly as tall as its content and does not scroll

#### Scenario: Past the cap, the input scrolls

- **GIVEN** a draft taller than the cap
- **THEN** the input stops at the cap and scrolls internally

#### Scenario: Deleting back under the cap retires the scrollbar

- **GIVEN** a scrolling input
- **WHEN** the draft shrinks back inside the cap
- **THEN** the input shrinks with it and stops scrolling

#### Scenario: Reset after send

- **WHEN** a multi-line draft is sent
- **THEN** the composer returns to its one-line height

#### Scenario: An IME candidate window is not resized under

- **GIVEN** an active composition
- **THEN** no resize runs until the composition ends

### Requirement: The composer shell reads as the primary control

The composer SHALL carry one crisp 2px border without a shadow, halo, or decorative
edge. On focus-within, that same border SHALL use the brand cyan in both light and dark
themes, returning to its neutral color on blur without changing geometry.

The empty input SHALL leave two spaces between the caret and the placeholder text.
These spaces SHALL NOT be added to the draft or submitted message.

A gradient dock fade SHALL sit above the docked composer so scrolled content
dissolves behind it instead of hard-clipping against it.

The caret SHALL carry the brand accent — the one mark that says the surface is live
and waiting.

#### Scenario: Focus treatment

- **WHEN** the reader focuses the composer
- **THEN** the single border turns brand cyan, and returns to neutral on blur

#### Scenario: Content fades behind the dock

- **GIVEN** a long conversation scrolled to the bottom
- **WHEN** content scrolls beneath the composer
- **THEN** it fades under the dock gradient rather than being hard-clipped

### Requirement: Interaction contract

The composer SHALL keep one keyboard contract: Enter sends, Shift+Enter inserts a
newline, and an Enter that is committing an IME candidate never sends.

Submission MUST trim the draft, MUST refuse an empty or whitespace-only draft, and
MUST clear the input on success.

The draft MUST be controllable from outside the component, and the component MUST
work uncontrolled as well, so it is testable both docked and on the welcome screen
without global state.

The caret SHALL rest in the composer on arrival, once, and only when the runtime can
actually accept a message. Later re-renders MUST NOT re-take focus, so focus is
never pulled out of a dialog or the transcript.

#### Scenario: Enter-to-send preserved

- **GIVEN** a non-empty draft
- **WHEN** the reader presses Enter with no modifier and no composition in progress
- **THEN** the prompt submits and the input clears

#### Scenario: A composing Enter does not send

- **GIVEN** an IME composition in progress
- **WHEN** Enter commits the candidate
- **THEN** nothing is submitted

#### Scenario: Shift+Enter inserts a newline

- **WHEN** the reader presses Shift+Enter
- **THEN** a newline is inserted and nothing is submitted

#### Scenario: A whitespace-only draft is not sent

- **GIVEN** a draft of spaces and newlines
- **THEN** submission does nothing

#### Scenario: Focus is taken once, not repeatedly

- **GIVEN** the composer took focus on arrival
- **WHEN** a dialog opens and the surface re-renders
- **THEN** focus stays in the dialog

### Requirement: The send control doubles as the stop control

While a run is in flight, the composer's circular send control SHALL become a stop
control: a filled square, always enabled, labelled for stopping the assistant's
response, invoking the run-cancel path.

No separate cancel affordance may appear anywhere else on the chat surface.

When no run is in flight the control is the send arrow, enabled only with a
non-empty draft.

#### Scenario: Stop during generation

- **GIVEN** a run is streaming
- **WHEN** the reader clicks the composer's circular control
- **THEN** the active run is cancelled
- **AND** no cancel pill exists above the transcript

#### Scenario: The stop control is enabled with an empty draft

- **GIVEN** a run in flight and an empty draft
- **THEN** the control is a stop control and it is enabled

### Requirement: Pending runs do not masquerade as disconnection

The composer SHALL show its disconnected copy only when the secure runtime is
actually unavailable. While a run is merely in flight, the placeholder is unchanged
and typing remains possible; only submission is gated.

The two states MUST be separate inputs to the component, so a caller cannot
accidentally report one as the other.

#### Scenario: Typing while the assistant responds

- **GIVEN** a run is streaming and the runtime is connected
- **WHEN** the reader focuses the composer
- **THEN** the placeholder is the ordinary one and keystrokes land in the draft

### Requirement: Typing has exactly one subscriber

The component bound to the draft store SHALL be the only subscriber to it, so a
keystroke re-renders the input and nothing above it.

The control itself MUST stay free of store knowledge, taking the draft as a prop, so
it is rendered in more than one position and tested in each without resetting global
state.

#### Scenario: A keystroke re-renders the input alone

- **WHEN** the reader types one character
- **THEN** only the composer re-renders

## Provenance

Consolidated from the deltas of `refine-run-feedback-ux`,
`fix-conversation-open-web-parity`, `polish-chat-visual-system` and
`refine-composer-and-response-boundary`, reconciled against the shipped code and
resolved in that order.

Resolved contradictions:

- The placeholder is "Ask about your data", not "Ask anything". The two earlier
  changes named the latter; `refine-composer-and-response-boundary` changed it and
  the code agrees.
- The autosize cap is 330px and the input's minimum is 27px with an explicit 24px
  line box. Two earlier deltas named ~300px and a ~72px shell; the shell's height
  now comes from the card's own block padding plus the input, and both were raised
  10%. Requirements above state the cap as a fixed value rather than repeating a
  number that has moved twice.
