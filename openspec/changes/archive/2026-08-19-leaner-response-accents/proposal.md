# Leaner Response Accents

## Why

Two marks frame every assistant answer: a vertical rail down its left edge
and the response ribbon's horizontal rule. Both still read as chrome.

The rail already distinguishes the latest answer (accent) from history
(grey). The ribbon does not — every rule in the transcript carries the
brand accent, so a scroll through twenty answers is twenty accent strokes
competing for the eye, and "this is the answer you just got" stops meaning
anything.

## What Changes

- **Both marks lose half their ink.** The rail and the rule drop from a
  50% to a 25% mix of their colour, and the rule's gradient reaches half
  as far across the measure before dissolving. The stroke stays one device
  pixel wide: a 0.5px box does not paint at all on this engine — measured,
  at device scale 1 and 2 — so presence, not width, is what halves.
- **Only the latest answer is accented.** The ribbon's rule takes the same
  latest/history split the rail already has, driven by the same
  `isLatestAssistant` the row already knows. History reads grey on both
  marks; the newest answer reads accent on both.

## Capabilities

### Modified Capabilities

- `chat-thread-layout`: the answer's accent marks, and which answer wears
  them.

## Impact

- `src/index.css`: `.assistant-accent*::before`, `.response-ribbon__rule`.
- `src/features/chat/ResponseRibbon.tsx`: a `tone` prop.
- `src/features/chat/MessageRow.tsx`: passes it.
