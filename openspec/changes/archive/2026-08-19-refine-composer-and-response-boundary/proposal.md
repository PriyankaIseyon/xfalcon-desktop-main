# Refine Composer And Response Boundary

## Why

Owner review of the shipped transcript (2026-08-06), three findings:

1. **The composer shows a scrollbar with nothing typed in it.** The
   textarea carries a standing `overflow-y: auto` while the autosize
   sets `height = scrollHeight`. `scrollHeight` is an integer and the
   reading type is a fluid `clamp()` (16.928px at this window), so the
   line box is fractional and the measured height lands a hair under the
   content — one pixel of overflow, a permanent scrollbar on an empty
   composer.
2. **The placeholder is casual.** "Ask anything" reads consumer; this is
   an analyst's tool.
3. **The assistant turn has no response boundary and its accent rail is
   heavier than it should be.** xfalcon-web opens every assistant answer
   with a labelled hairline (`ResponseRibbon`) so the step out of the
   run's activity and into the answer reads the same whether tools ran
   or not; desktop has no such marker.

## What Changes

- **The composer never shows a scrollbar it does not need.** Scrolling
  stays off below the cap and turns on at the exact line the draft
  outgrows it, then off again when the draft shrinks back — the pattern
  every auto-growing composer converges on. The line box is pinned to an
  integer (`leading-6`) so the measured height can no longer fall short
  of the content; the cap itself is unchanged at 300px (twelve lines),
  and the input grows to it before scrolling.
- **Placeholder: "Ask about your data."** Names the task in the
  product's own language.
- **A response ribbon opens every assistant answer** — an uppercase
  `RESPONSE` label and a hairline that carries the brand accent at the
  label and dissolves before the measure ends. Static by construction:
  the in-flight stream row and the durable row that replaces it paint
  the identical ribbon, so the settle has nothing to re-animate.
  `aria-hidden` — the turn already carries its role for assistive tech.
- **The assistant accent rail is halved to `0.5px`.** Chromium paints a
  sub-pixel box at partial coverage, which is the intent: the rail
  registers at the edge of vision and never reads as a border.

## Capabilities

### Modified Capabilities

- `chat-composer`: the placeholder, and the scrollbar/growth contract.
- `chat-thread-layout`: the response boundary above every assistant
  answer, and the weight of the accent rail.

## Impact

- `src/features/chat/Composer.tsx` (autosize + overflow, placeholder,
  line box) + tests.
- `src/features/chat/ResponseRibbon.tsx` (new),
  `MessageRow.tsx` (renders it) + tests.
- `src/index.css` (`.response-ribbon`, rail width).
- No store, wire contract, or Rust change.

## Research

`field-sizing: content` would replace the autosize script outright and
is the direction this converges on, but it is Chromium-only in the
versions we ship against: WebView2 on Windows has it, WKWebView on macOS
does not, and carrying both mechanisms would be two code paths for one
behaviour. The corrected script stays the single mechanism until
WKWebView ships the property.

- https://developer.chrome.com/docs/css-ui/css-field-sizing
- https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Properties/field-sizing
- https://css-tricks.com/the-cleanest-trick-for-autogrowing-textareas/
