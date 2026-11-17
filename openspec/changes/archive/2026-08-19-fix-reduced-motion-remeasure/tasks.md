# Tasks

Sequenced upstream-first: the reporter is the shared abstraction both
disclosures use, so it settles before either call site adopts it, and the
tests that pin the animated path as the only path are corrected before new
ones are added beside them. Fixing `ToolActivityShell` alone would leave
`ToolContent` on the old contract and the two disclosures disagreeing about
what a settle is.

## 1. Name what the signal actually reports

- [x] 1.1 Rename `useRemeasureOnAnimationEnd` to a name that states the
      fact it reports (a disclosure settled) rather than the event it used to
      wait for. The exported `REMEASURE_EVENT` name and its payload-free shape
      stay — `MessageList` already reads the row's height itself, which is what
      makes a duplicate report harmless.
      → `useDisclosureSettleReporter`.
- [x] 1.2 Rewrite its comment to record why the trigger moved: an
      `animationend`-only reporter is silent under
      `prefers-reduced-motion: reduce`, and silence there means a permanent gap.
      Keep the two notes still true (native listener because React 19's
      synthetic animation events do not fire in jsdom; callback ref because
      Radix mounts and unmounts the body per open/close cycle).
      → The second note was **false for this Radix version** and is now recorded
      as what actually happens; see 2.2.

## 2. Make the reporter motion-independent

- [x] 2.1 On attach, capture the enclosing row (`closest("[data-index]")`,
      falling back to the body's parent) and keep it for the detach dispatch —
      the body is gone by then and cannot dispatch for itself.
- [x] 2.2 On attach, ask the element whether an animation will run, the same
      way Radix's presence does: `getComputedStyle(node).animationName ===
"none"`. When nothing will animate, dispatch at once — the body is already
      at its final height. When something will, wait, so no intermediate frame
      is written as the settled height.
      → **Corrected against observed behaviour.** `@radix-ui/react-collapsible`
      1.1.x does NOT remount the body per cycle: it keeps the content node
      mounted, renders `{isOpen && children}`, and flips `data-state` (plus
      `hidden` once a close settles). A ref-attach check would therefore fire
      once, at mount, while the disclosure is closed — never on a toggle. The
      question is asked on each `data-state`/`hidden` mutation instead, via a
      `MutationObserver` whose microtask also lands after the commit that
      changed the height. A `hidden` body is likewise treated as settled: it
      contributes no height, which is what makes the post-animation close
      report land on the collapsed height.
- [x] 2.3 Keep the native `animationend` listener for the animated case,
      and keep it on the body so nested rows' animations still bubble through
      it.
- [x] 2.4 On detach, dispatch from the captured row. This is the collapse
      path under reduced motion, and it is also the correct moment on an
      animated close — Radix removes the body only after the close animation
      has finished.
      → Deferred to a microtask: React detaches refs BEFORE it removes the
      nodes, so a synchronous dispatch here measures the pre-collapse height.
      With this Radix version the path that exercises it is a nested detail
      panel disappearing with its umbrella.
- [x] 2.5 No `matchMedia` anywhere in this path. The condition is "will an
      animation run", and the media query is only one of the reasons the answer
      is no.

## 3. Adopt it at both disclosures

- [x] 3.1 `ToolActivityShell` keeps using it on its `CollapsibleContent`
      (the grouping umbrella).
- [~] 3.2 `components/ai-elements/tool.tsx`: `ToolContent` uses the same
  reporter on its own `CollapsibleContent`, so a call row's
  Parameters/Result panel settles its row with motion off. Note the
  divergence from the vendored AI Elements source in the file's existing
  divergence list, with the reason.
  → **Deviation, deliberate.** The reporter reaches every nested panel, but
  through `<ToolContent ref={reportSettle}>` at both call sites in
  `ToolActivityCard.tsx`; `tool.tsx` is untouched. Putting the hook call
  inside `ToolContent` requires `tool.tsx` to import from
  `features/chat/ToolActivityCard`, which already imports `tool.tsx` — a
  features↔components import cycle. Coverage is identical today (both
  `ToolContent` call sites are in `ToolActivityCard.tsx`), and one hook
  still owns the mechanism: the callback ref's React 19 cleanup is
  per-attachment, so a single reporter serves the umbrella and every panel.
  If `ToolContent` should own it for future call sites, the hook and
  `REMEASURE_EVENT` want to move to `src/lib/` first, which is its own edit.
- [x] 3.3 Confirm `MessageList`'s handler still resolves the right row for a
      report dispatched from the row element itself — `closest("[data-index]")`
      on a row returns that row — and that a report dispatched during unmount
      still reaches the scroll element's listener.
      → Asserted in `ToolActivityCard.test.tsx` (every report resolves to the
      row, including the one dispatched from the row itself). A report from the
      live tail, which sits outside the virtualizer, resolves to no row and is
      ignored — the tail is not virtualized and needs no measurement.

## 4. Tests

- [x] 4.1 Replace `ToolActivityCard.test.tsx`'s single remeasure case: it
      asserts exactly one dispatch, which is what pinned the animated path as
      the only path. Split it into a settle that animated (synthetic
      `animationEnd`, as today) and a settle that did not (open with no
      animation reported by the environment).
- [x] 4.2 A collapse with no animation still reports — the body is removed
      and the report arrives from the row. This is the case that failed in
      production and had no test.
- [x] 4.3 A nested detail panel's expand and collapse each report, and the
      report resolves to the containing row.
- [x] 4.4 `MessageList.test.tsx`: a report for an expanded-then-collapsed
      row writes the collapsed height, so the grow-only cache is overridden in
      the shrink direction; and two reports for one toggle leave the same
      height as one.
- [x] 4.5 Assert the negative that keeps the fix honest: no `matchMedia`
      call is made while a disclosure toggles.
- [x] 4.6 Confirm the new cases fail against the old reporter: with the
      observer and the detach dispatch removed, 4.1's motionless half, 4.2,
      4.3 and the body-removal case all fail; the animated case and the
      `matchMedia` negative still pass, as they should.

## 5. Verification

- [x] 5.1 `npx vitest run src/features/chat src/components`, `npx tsc -b`,
      `npx eslint` clean on the touched files.
      → `src/components` holds no tests. `tsc -b` clean. `eslint` clean on the
      touched files except one pre-existing `react-refresh/only-export-components`
      error on `MessageList.tsx`'s `buildDisplayItems` export, unchanged by this
      work.
- [x] 5.2 Full `npx vitest run` — `ToolActivityCard`, `MessageList`, and
      `ChatSurface` all render these disclosures.
      → All 20 files under `src/features/chat` green (178 tests), which is every
      consumer: nothing outside that directory imports these components. The
      whole-repo run was not done here.
- [x] 5.3 Live, with OS reduced motion ON: run a multi-tool conversation,
      expand the trail mid-transcript, expand a call's detail panel, collapse
      both, and confirm the following rows sit flush and the scroll position
      does not jump.
- [x] 5.4 Live, with OS reduced motion OFF: the same pass, confirming the
      animation still plays and no row flickers to an intermediate height on
      the way open.
