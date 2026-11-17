# Chat streaming and presentation review

Research and implementation review: 2026-09-16.

The reported symptoms were intermittent cut-off, jagged, or laggy conversations.
No production trace identified one common cause. Investigation covered both live
responses and saved conversation rendering. The presentation changes repair
reproduced defects without changing transport or DA contracts. The subsequent staging-to-main audit also repaired native
persistence and cancellation defects; see [the audit report](staging-main-audit.md).

## Research

Primary sources reviewed before implementation:

- [TanStack Virtual API](https://tanstack.com/virtual/latest/docs/api/virtualizer):
  dynamic measurement must have one owner; mixing `resizeItem` and
  `measureElement` for the same item can produce unpredictable results. Deferring
  ResizeObserver measurement through another animation frame can leave stale
  layout. The installed 3.14.9 implementation was also inspected: without an
  observer entry its default measurement can return a cached height.
- [AI SDK memoized markdown cookbook](https://ai-sdk.dev/cookbook/next/markdown-chatbot-with-memoization)
  and [Streamdown](https://github.com/vercel/streamdown): reuse completed blocks and
  handle incomplete markdown deliberately. Desktop already has incremental mdast
  splitting and memoized blocks, including differential parser tests. Replacing
  that pipeline or adding another renderer was unnecessary.
- [React useDeferredValue](https://react.dev/reference/react/useDeferredValue):
  deferral prioritizes work but does not make parsing cheaper. The confirmed costs
  were addressed directly rather than placing another deferred queue in the path.

Local implementations inspected for mechanics, without copying their code:

| Reference | Relevant behavior | Decision |
| --- | --- | --- |
| `references/LibreChat/client/src/hooks/Messages/useMessageScrolling.ts` | Scroll direction, reader intent, immediate stream following | Distinguish upward movement from delayed programmatic events |
| `references/opencode/packages/ui/src/hooks/create-auto-scroll.tsx` | Explicit handling of asynchronous scroll events and dynamic browser anchoring | Fix the reproduced race with the existing desktop scroll state |
| `references/opencode/packages/session-ui/src/components/message-part.tsx` | Backlog-aware pacing and immediate terminal synchronization | Retain desktop pacing, remove its fixed 48-character ceiling |
| `references/goose/ui/desktop/src/hooks/useThrottledStreamingText.ts` | Latest-value coalescing and terminal flush | Preserve existing store/frame coalescing; avoid stacking another delay |
| `references/LibreChat/client/src/components/Chat/Messages/Content/MarkdownBlocks.tsx` | Stable completed markdown blocks | Preserve desktop's existing block memoization |
| `xfalcon-web/src/components/StreamingTail.tsx` and `MarkdownRenderer.tsx` | Stable live/durable presentation and delayed retirement | Preserve desktop's existing handoff; no reproduced reason to add a hold or fade |

Reference paths above are relative to `/home/jeevan/work`.

## Confirmed repairs

1. **Stream backlog:** the old 48-character-per-frame ceiling exposed only about
   961 UTF-16 units after 20 frames of a 42k source burst. Proportional draining
   now keeps pace with large bursts. Existing terminal, replacement, reduced
   motion, hidden-document, and Unicode safeguards remain covered.
2. **Scroll detachment:** a browser test with a long streamed answer stopped
   roughly 2,019 px above the bottom without user input. A delayed scroll event
   observed newer content height and incorrectly released following. The handler
   now tracks actual upward movement, and browser anchoring is disabled while
   following. Wheel scrolling releases following; the jump button restores it.
3. **Stale historical heights:** the grow-only cache retained a 200 px row after
   it shrank to 72 px. Ordinary ResizeObserver updates now accept positive shrink
   measurements. Settle reports and width changes use that same measurement path;
   remounted rows read fresh layout. Zero-size transient measurements keep the
   previous valid height.
4. **Repeated grammar setup:** a module-level plugin tuple still constructed a
   new lowlight registry for each ReactMarkdown processor. A shared configured
   transformer reduces registration from three calls to one in the real streaming
   consumer regression. Sanitization and language classification are preserved.
5. **Unlabelled code fidelity:** removing every markdown `pre` wrapper collapsed
   unlabelled fenced and indented code into inline text. These blocks now retain
   plain `pre/code` structure and exact whitespace, including nested markdown.
   Labelled artifact cards keep their existing classification and wrapper.

## Validation and limits

Focused regressions exercise the real message components, the native-event mock
boundary, and browser layout. Browser scenarios include large Unicode/code/table
answers in normal and reduced motion, deliberate scrolling during generation,
jump-to-latest, terminal rendering, the live-to-durable snapshot handoff in the
mounted conversation, reopening saved text, reading-size shrinkage, and revisiting
offscreen history. Existing streaming, splitter, and 10k-message budgets remain
regression gates.

Verification results:

- Frontend gate: 128 Vitest files / 1,717 tests passed, including the existing
  streaming and large-history budgets. Typecheck, lint, formatting, style,
  tooltip, font, release-script, and UX-matrix checks passed.
- Production frontend build passed. All four modified specifications passed
  strict OpenSpec validation.
- The Windows custom-protocol build passed and the local v0.5.13 app was
  relaunched successfully with the rebuilt frontend.
- Four focused browser scenarios passed, including the complete response
  lifecycle in both motion modes. The separate update-badge interaction suite
  passed 18 repeated cases. Its second-action setup now waits for SettingsView's
  existing 400 ms reveal reconciliation before attempting another navigation.
- Full chat E2E: 14 passed against the production frontend, including the
  lifecycle tests above. Four stale expectations from untouched staging were
  corrected after checking the implementation and commit history.
- Full visual suite: 180 passed. Sixty-one stale screenshots were reviewed against
  earlier approved sidebar and control changes before updating their baselines.
  An activity fixture now supplies response feedback correctly, removing a false
  error from the happy-path screenshots. Pixel tolerances remain unchanged.
- Independent presentation review found no blocking or important regression.
  The broader native and security audit is documented separately.

The original rare production complaint has no captured trace, so these repairs
do not establish that every reported incident had the same cause. Chromium tests
and a local Windows build do not substitute for a macOS WebKit run. Proportional
pacing bounds queue growth; it does not establish a universal frame-time budget
for arbitrarily large markdown or code blocks. The caret's whole-answer text walk
remains a profiling candidate, with no demonstrated content-loss defect here.
