# Settle a row's height whether or not the animation played

## Why

Collapse a tool disclosure in the middle of a transcript with
`prefers-reduced-motion: reduce` set, and the space it occupied stays
occupied. The gap sits under the turn for the rest of the session, and it
appears only for the users who asked for less motion.

The chain is short. `useRemeasureOnAnimationEnd`
(`src/features/chat/ToolActivityCard.tsx`) binds exactly one listener,
`animationend`, and the event it carries — `REMEASURE_EVENT` — is the only
thing that ever tells `MessageList` a row got _shorter_. The global
reduced-motion kill switch (`src/index.css`) sets `animation: none
!important` on every element. No animation, no `animationend`, so
`virtualizer.resizeItem` is never called, and `measureMessageRow`'s cache —
which deliberately only ever grows, so a streaming row cannot jitter —
keeps the expanded height forever. `chat-tool-trail`'s "Expand then
collapse leaves no gap" is a requirement this build honours only while
motion is on.

Three properties of the defect decide the shape of the fix, and each one
rules out an easier-looking repair:

1. **The report cannot come from the disclosure body.** Radix decides
   whether to wait for a close animation by reading
   `getComputedStyle(node).animationName` and unmounting immediately when
   it is `none` — which is exactly the reduced-motion case. So on the
   collapse, the direction that leaves the gap, the node the listener is
   bound to is detached before anything could be dispatched from it.
   Swapping the event name on the same node cannot fix the failing
   direction.

2. **The trigger cannot be the shell's `open` prop.** A call row's
   Parameters/Result panel is a second, _uncontrolled_ Radix collapsible
   living inside `Tool`; the shell never sees its state, and today only a
   bubbling `animationend` covers it. A fallback keyed to the umbrella's
   `open` would settle the umbrella and leave every nested panel broken —
   a partial fix that reads as a fix.

3. **The condition is not the media query.** Reading
   `prefers-reduced-motion` from JS answers a different question than the
   one that matters — _will an animation run?_ — and misses every other
   reason one does not: keyframes absent from a build, a platform that
   drops them, and the jsdom test environment. That last one is why this
   shipped: the single remeasure test fires a synthetic `animationEnd`, so
   it exercises the one path that already worked and cannot fail on the one
   that does not.

## What Changes

- **The settle is reported by the disclosure, not by the animation.** A
  disclosure body reports when it has finished opening and again when it
  goes away. `animationend` stops being the trigger and becomes one of the
  moments a settle is recognised — the animated one.
- **The report is dispatched from a node that outlives the body.** The
  reporter captures the enclosing row when the body attaches and dispatches
  from that on detach, so the collapse can still speak after Radix has
  removed the content it used to speak from.
- **Whether to wait is asked of the DOM, not of the user's preference** —
  the same question Radix asks, of the same element. A body that attaches
  with no animation is already at its final height and reports at once; a
  body that attaches with one waits for it, so an intermediate frame's
  height is never written as the settled height.
- **Nested detail panels report too.** `ToolContent` uses the same
  reporter as the umbrella, so expanding or collapsing one call's
  Parameters/Result re-measures its row with motion on or off.
- **Repeated reports are harmless.** A settle report carries no height; the
  listener reads the row's real height and writes that, so one toggle
  reported twice converges on the same measurement rather than fighting
  itself.
- **The kill switch is not touched.** Exempting `.ai-collapsible-content`
  from `animation: none !important` would "fix" the measurement by
  restoring the motion the user asked not to see — trading a layout bug for
  a consent bug.

## Capabilities

### Added Capabilities

None. This change adds no capability; it repairs one that shipped with a
rule whose obligation was carried entirely by an animation.

### Modified Capabilities

- `chat-motion-system`: suppressing motion stays presentational — a row's
  measured height settles after a disclosure toggles whether or not
  anything animated.

## Impact

- Changed: `src/features/chat/ToolActivityCard.tsx` (the hook, renamed for
  what it now reports), `src/components/ai-elements/tool.tsx`
  (`ToolContent` reports through the same hook),
  `src/features/chat/ToolActivityCard.test.tsx`,
  `src/features/chat/MessageList.test.tsx`.
- Unchanged deliberately: `src/index.css` — the kill switch is correct and
  is not the defect; `MessageList`'s grow-only measure — the jitter it
  prevents is real and unrelated; every contract, every Rust file, every
  Tauri command. This change is entirely inside the renderer's measurement
  path.
- One existing test must change rather than be added to:
  `ToolActivityCard.test.tsx`'s remeasure case asserts exactly one
  dispatch, which pins the animated path as the only path. It becomes two
  cases — a settle that animated and a settle that did not.

## Not in this change

**Letting the measurement cache shrink on its own.** Dropping the grow-only
rule in `measureMessageRow` would make every disclosure settle itself, and
would also let a streamed row shrink on any transient re-layout of its
markdown — the jitter that rule was written to stop. The explicit signal
stays; only its trigger is wrong.

**A motion preference inside the app.** `settings-appearance` chooses theme
and reading size; motion stays the OS's answer, read only by CSS. Nothing
here reads or stores a motion preference, which is the point of asking the
DOM instead.

**Auditing the rest of the surface for the same class of bug.** This change
fixes the one place where an animation event carries a non-visual
obligation, and states the rule that forbids new ones. A sweep for other
behaviour that only runs when motion is on is worth doing and is its own
change.
