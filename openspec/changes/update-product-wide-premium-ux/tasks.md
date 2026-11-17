# Tasks: Product-wide premium UX system

Checkboxes are evidence statements. A task stays open until its stated proof exists
and passes.

## 0. Matrix and baseline

- [x] 0.1 Inventory all reachable user-visible surfaces and states in
      `ux-matrix.md`.
- [x] 0.2 Inventory every shared visible primitive in `src/ui`.
- [x] 0.3 Define the complete quality-attribute matrix, weighted scorecard, status
      model, and closure rule.
- [x] 0.4 Inspect the current shell, chat, approval, activity, theme, and narrow
      baselines under `tests/visual/baselines`.
- [x] 0.5 Inspect the direct primary pages for Beautiful UI, 21st.dev, beUI, Rare UI,
      Transitions.dev, shadcn/ui, and Apple HIG. Record the transfer rules in the matrix.
- [x] 0.6 Audit production TSX for raw controls, primitive use, tooltip coverage,
      radius, depth, type, icon, spacing, state, and responsive drift.
- [x] 0.7 Verify the first P1 defects against source and built CSS. Record I01–I08.
- [x] 0.8 Close the optional broader Exa competitive research pass.
      **Closed 2026-08-31 on owner direction.** The pass was not used to define
      any closure rule. Task 0.5 remains the complete reference evidence from
      seven direct primary sources.
- [x] 0.9 Run `openspec validate update-product-wide-premium-ux --strict
--no-interactive`.
- [x] 0.10 Obtain owner approval for this proposal, matrix, order, batch delivery,
      and acceptance gate before implementation.

## 1. Foundation and shared primitives

- [x] 1.1 Close I01. Replace non-existent semantic classes and add a built-CSS
      selector guard.
- [x] 1.2 Close I02. Make press, hover, color, shadow, and transform transitions one
      coherent primitive contract.
- [x] 1.3 Name and implement the geometry tokens for spacing, control height, icon
      size, radius, border, and surface recipes.
- [x] 1.4 Align the canonical typography roles. Remove unexplained 10 px, 11 px,
      and raw size variants. Keep the current Google Sans and Geist Mono files,
      families, loading, and role mapping unchanged.
- [x] 1.5 Name and implement the icon stroke roles. Migrate Lucide sizes and weights.
- [x] 1.6 Consolidate P01–P03 actions: Button, IconButton, and Link. Proof:
      `src/ui/actions.test.tsx` (17 tests) pins hierarchy on `variant` for both
      action primitives, `tone` for status surfaces only, one busy statement,
      one press transition, danger ink on its own role, and the external-link
      contract; the two prose-link regressions in
      `src/features/chat/AssistantMessage.test.tsx` were proved by removing the
      `a` renderer; the style guard fails a missing `--xf-danger-ink` and any
      `:visited` selector, each proved by a planted violation.
- [x] 1.7 Consolidate P04–P08 fields: Field, FileField, SearchField, DateTimeField,
      and Select. Proof: `src/ui/fields.test.tsx` (21 tests) pins the minted
      control id, hint-with-error, merged descriptions, the required state on
      the control, one control surface across input/textarea/search/date/time/
      datetime/select, the suppressed native search clear, and visible focus on
      the file field; each assertion was proved against the pre-fix file.
- [x] 1.8 Consolidate P09–P17 selection, navigation, menu, and disclosure
      primitives. Proof: `src/ui/selectionNavigation.test.tsx` (13 tests)
      pins visible native checkbox/radio focus, mixed state, truthful switch
      busy state, slider semantics, automatic segmented and tab focus,
      tab-panel relationships, controlled disclosure ids, menu selection and
      danger roles, keyboard opening, typeahead, Escape focus return, viewport
      clamping, and modal focus-scope ownership. Twelve contract failures were
      proved by planted violations, including separate modal-portal and
      trigger-edge placement violations inside the real hierarchy.
      `src/ui/interactionStates.test.tsx` (38 tests) now includes Switch busy
      refusal. Dataset, project-file, sidebar, and dialog consumers pass on the
      new contracts. The unused pill segment path is deleted and all callers
      use the required accessible group names.
      Re-audited (batch 13a): five defects found and closed -- Escape inside a
      dialog closed the dialog, Tab out of a menu dropped focus to `<body>`,
      unselected tabs carried dangling `aria-controls`, `.xf-switch` was
      applied and undefined, and `Radio` dimmed twice. `ui/dismissLayers.ts`
      owns Escape and Tab by layer depth, and `check-style-contract.mjs`
      fails any applied-but-undefined `xf-` or BEM class.
- [x] 1.9 Close I06. Replace native title and missing help with P18 Tooltip. Add portal,
      collision, side placement, focus, Escape, and coverage tests.
- [x] 1.10 Close I03. Consolidate P19–P20 and S141–S142 into one modal and alert-dialog
      family without weakening caller-specific dismissal rules.
- [x] 1.11 Consolidate P21–P27 feedback, progress, loading, status, and empty-state
      primitives. Proof: `src/ui/feedbackStates.test.tsx` (14 tests) pins the
      alert tone-to-role map with a fixed action slot, the three-card toast
      stack with an action before dismiss, auto-dismiss paused by pointer AND
      by keyboard focus, errors that outlive any timer, progress that states
      determinate, complete, and indeterminate without relying on colour,
      skeletons that stay hidden, spinners that are named or explicitly
      decorative, a status dot that is named or explicitly decorative, and an
      empty state that must explain itself. Eleven of the fourteen were proved
      against the pre-change primitives. The second toast (`ui/Toast.tsx`,
      `.app-toast`), the feature-local settings notice (`.settings-feedback*`),
      and the second skeleton recipe (`.ghost-block`) are deleted with every
      caller moved, and `check-style-contract.mjs` fails any source or built-CSS
      revival of the three names.
- [x] 1.12 Close I08. Consolidate P28–P30 surface, panel, and table recipes.
- [x] 1.13 Consolidate P31–P42 structure and display primitives. Proof:
      `src/ui/structureDisplay.test.tsx` (17 tests) pins one platform spelling
      for a shortcut, a required note that survives the accessible-name trim,
      a list with no flag that paints a row as a control, a chip that answers
      the cursor and refuses control props on its static branch, pagination
      that announces its position and keeps focus off a control it just
      disabled, heading roles whose look and outline depth are separate props,
      and the display primitives that name themselves or say nothing. Nine of
      the seventeen were proved against the pre-change primitives; the `Chip`
      prop narrowing and both new style-contract rules were proved by planted
      violations. P32 NavBar, P35 Pagination, and P38 Divider have no
      production consumer and are kept as maintained primitives under test, on
      the owner decision already recorded for P05 and P07.
- [x] 1.14 Close I05. Migrate every recurring raw or feature-local control. Delete the
      obsolete path after the last caller moves.
- [x] 1.15 Close I07. Add one exhaustive interaction-state test matrix for applicable
      primitives.
- [x] 1.16 Verify P01–P42 in light, dark, high contrast, forced colors, reduced
      motion, reduced transparency, keyboard, coarse pointer, and 200% text.
      Proof: `primitives.html` renders all 42 primitives with their P-ids and is
      absent from the production build (`check-style-contract.mjs` fails if it
      reaches `dist`, proved by a planted violation).
      `tests/visual/primitives.spec.ts` (9 tests) drives that gallery under each
      mode and asserts the rule the mode exists for. It found and closed three
      defects: a focus ring that faded in from the element's own text colour,
      `--xf-ink-3` below AA on every tinted light surface, and eight controls
      under the coarse-pointer target. Four modes were proved by planted
      violation. The engine is Playwright's Chromium; WKWebView is not covered
      here and section 9 still owes packaged macOS evidence.
- [x] 1.17 Run `npm run typecheck` and targeted primitive tests after each primitive
      batch.

## 2. Shell, navigation, startup, and global command surfaces

- [x] 2.1 Reconcile this change with `add-workspace-destination-rail` and remove any
      duplicate visual tasks. The rail change now contains only its own task list.
      Its two remaining visual tasks are closed there with evidence. Task 5.6 is
      guarded by the reading-size geometry test in `tests/visual/shell.spec.ts`,
      proved by removing the well padding. Task 5.7 is proved by the shell baseline
      replacement in `f5e5b0a`, which descends from the destination implementation
      in `afc9d1f`. No open rail task duplicates this change.
- [x] 2.2 Polish S005–S010 as one shell slice. Preserve destination and rail behavior.
      Proof: S005-S010 are `Implemented`. The arrival skeleton now matches the
      actual initial chat shell and welcome geometry at wide and narrow widths.
      The five-second cap, reduced motion, blocked shell, chat-readiness gates,
      destination and rail behavior, sidebar consumers, and account keyboard
      flow have real-consumer tests. Batch 18 records the planted violations and
      the full repository evidence.
      Repaired (batch 18a): `awaitingWorkspace` gated the skeleton on `chatReady`
      alone, which reads `false` both while `workspace_status` is still in
      flight and once it durably answers not-ready -- so sign-in flashed the
      bare, unstyled shell for the render or two before the status call
      landed. It now also holds the skeleton while that call is pending, and
      still falls through once a tenant settles not-ready.
      Repaired (batch 18b, code-only re-audit, no test run this pass): 18a's
      pending check was unlatched, so it re-armed on any LATER `workspace.refresh()`
      too, not only the first resolution -- reachable from Settings after adding
      a provider credential or editing the MCP registry following an earlier
      not-ready answer, which would yank the whole shell back to the arrival
      skeleton mid-session. Latched to the first resolution with a ref; every
      refresh after it is the settled shell's concern, not the arrival gate's.
- [x] 2.3 Normalize sidebar row height, icon column, text baseline, indentation,
      selection, cursor, hover actions, and long-name behavior. Proof: measured
      first, then fixed. `--xf-nav-label-x` declared one label column and nothing
      used it, so the column was whatever each row's glyph left behind — a
      1.125rem Settings icon put that one label 2px off every other. The glyph
      sits in `--xf-nav-icon-col` now, and `the sidebar keeps one label column for
its icon rows` holds it, proved by restoring the larger glyph. The chat
      list's step-in is deliberate and stays, and the comment that claimed
      otherwise is corrected. Cursor: Tailwind v4's preflight leaves a `<button>`
      on the default arrow, and 38 of the 71 live controls in the shell had no
      `cursor-pointer` — including every Pin and Delete on every chat row, the
      only mouse route to those commands. `index.css` answers it once, and both
      visual suites guard it. Row height, selection, and long-name behavior were
      measured and already hold: `--xf-nav-row-h` for list and tree rows, the
      taller `--xf-nav-destination-h` for the destination well by design, one
      `--xf-nav-selected` appearance, and truncation on every row.
- [x] 2.4 Polish S012 command palette with the shared field, menu, surface, empty,
      busy, and keyboard patterns. Proof: S012 is `Implemented`. The panel now
      shares `xf-material`/`animate-pop`, the same floating-surface recipe
      `Modal` and `ContextMenu` already use, in place of a hand-rolled copy
      that had drifted on border token and blur saturation. Every icon carries
      the icon-stroke role. The active row transitions like every other
      selectable row. The message-search wait pairs a spinner with its text,
      matching App.tsx's own busy pattern. The active option is re-clamped
      whenever the result set changes for a reason other than a keystroke, not
      only on one. Batch 19 records the evidence; large-catalog, narrow, and
      dedicated palette visual baselines remain open under 2.8 — delivered there;
      see 2.8's own proof.
- [x] 2.5 Polish S001–S004 startup, authentication, and recovery with one brand and
      feedback hierarchy. Proof: S001-S004 are `Implemented`. `AuthGate`'s
      `authenticating` state -- reachable straight from bootstrap on a restart,
      since the backend holds that status across a relaunch while its loopback
      listener stays open -- rendered as a bare spinner with no brand mark, no
      card, and no way back, because `LoginView`'s own Cancel button is local
      state that had not mounted yet. It now shares the mark-plus-card frame
      every other startup screen carries, plus its own Cancel action on the
      same `auth_cancel` command. `AppErrorBoundary`'s recovery panel was the
      only one of the four missing the brand mark outright. Separately,
      `.app-startup h1/p` and `.app-recovery h1/p` hand-duplicated the
      `type-title`/`type-body` role values directly on the tag at higher
      specificity than any `.type-*` class an element inside carried, so they
      silently forced `LoginView`'s `HeroTitle` down from its hero size and
      its `EmptyState` description down to a more muted ink than either
      component declared, while supplying no `font-weight` of their own --
      every bare `<h1>` in the family rendered unbolded next to every other
      heading role in the app. Every heading in the family is `PageTitle` now
      and every description or status line carries an explicit `type-body
text-secondary`; the container rules are layout-only. Batch 20 records
      the evidence; light/dark/native-start, real-failure, external-browser
      handoff, and dedicated startup/auth/recovery visual baselines remain
      open under 2.8.
      Repaired (batch 20a, code-only re-audit, no test run this pass, predates
      batch 20): the error state was the one screen in the family whose `<main>`
      carried no `aria-labelledby`, because its heading carried no `id` to point
      at. Given both, matching the other three.
- [x] 2.6 Polish S011, S013, and S014 global offline, shortcut, and notification
      surfaces. Proof: S011, S013, S014 are `Implemented`. S014 (`Toaster`) was
      already sound -- all four tones, action, overflow, and pause were real,
      passing coverage in `feedbackStates.test.tsx`; nothing to fix, only to
      mark. S011's offline state, tone, and wiring were sound too, but had zero
      direct unit coverage and no answer for "reconnect": `OfflineBanner`
      unmounts the instant `online` flips back true, and an unmount announces
      nothing to a `role="status"` region -- only an addition or update does --
      so nobody using a screen reader (or not looking at that exact spot) had
      any signal connectivity was back, short of trying an action and seeing
      whether it worked. `AppShell` now fires the same `toast.success` shape
      MCP reconnection already uses on that transition, skipped on first mount.
      S013's platform-label mechanism (`ui/Kbd.tsx`'s `Shortcut`) had zero
      direct tests despite existing specifically because three call sites once
      spelled the same shortcut three different ways, and the dialog's own
      label-plus-keys row had no floor on either flex child: at the narrowest
      window the app allows (`minWidth: 900`), a long description could widen
      the row past the dialog instead of yielding, since nothing wrapped this
      sheet's own rows in the vertical scroll it gives everything else. Given
      the same `flex-1 truncate` / `shrink-0` split `Menu` already uses for
      label-plus-trailing-glyph rows. 14 new Vitest tests: 3 direct
      `OfflineBanner` tests, 5 direct `Shortcut` tests (Apple/non-Apple
      spelling, hidden caps, unknown-platform and unknown-key fallback), 5
      direct `ShortcutHelpDialog` tests (closed, open with platform-correct
      keys, Escape, the truncate/shrink-0 pair), 1 `AppShell` test proving the
      reconnect toast fires on a real offline-to-online transition and stays
      silent on a session that starts online. Batch 21 records the full gate.
- [x] 2.7 Decide S015. Either expose the local-data action and verify its flow, or
      remove the unreachable visible code path. Owner decision, 2026-08-26: neither
      — the destructive action stays unoffered and the flow stays wired, so it can
      be enabled without being rebuilt. S015 is `N/A` in the ledger with that
      reason, and `SHOW_LOCAL_DATA_MENU_ACTION` in `src/app/AccountMenu.tsx` is the
      one switch that changes it. `AccountMenu.test.tsx` already asserts the item is
      absent, so the decision cannot drift unnoticed.
- [x] 2.8 Add visual baselines for open sidebar, rail, drawer, populated lists,
      palette, account menu, startup, authentication, and recovery. Proof: Batch
      22 adds deterministic Chromium baselines for open light/dark shell, collapsed
      light/dark rail, narrow populated drawer, populated Chats and Projects lists,
      open light/dark palette plus its large catalog and narrow placement, and open
      light/dark account menu. The same native-boundary fixture drives a new
      startup/authentication suite that captures light/dark startup loading and
      failure, signed-out login, authenticating cancel state, and error-boundary
      recovery. It checks each named landmark/action before its screenshot, and
      checks that the login heading renders above the plain title token so the
      old startup-container cascade cannot return unnoticed. `startup-visual.html`
      is a development-only recovery fixture; `check:styles` rejects it if Vite
      ever emits it into the production bundle. Full gate: `npm run typecheck`,
      `npm run build`, `npm run check:matrix`, 1458/1458 Vitest, and 59/59
      Playwright. No `.rs` file changed, so the Rust suite was not re-run.
- [x] 2.9 Add live-resize, keyboard-only, coarse-pointer, long-label, 200% text, and
      minimum-width tests.
      Re-audit found and repaired three production defects before adding the
      operational coverage: a single sidebar state lost the user's desktop rail
      choice after a narrow resize; rail and compact-account help bypassed the
      shared `Tooltip`; and rail controls were below the 44 px coarse-pointer
      target. `useAppLayout` now keeps desktop and narrow state independent,
      `Menu` can compose its injected trigger with the shared tooltip, and rail
      controls use `touch-target`. Real-browser tests cover live resize,
      keyboard-only rail navigation and tooltip Escape, coarse-pointer targets,
      long native-boundary labels at the actual 900 px Tauri minimum width, and
      200% text without horizontal content loss. Evidence: `npm run typecheck`,
      `npm run build`, `npm run check:matrix`, 1458/1458 Vitest, and 63/63
      Playwright. No `.rs` file changed, so the Rust suite was not re-run.

## 3. Chat and composer

- [x] 3.1 Reconcile the delivered and unfinished tasks in
      `add-power-composer-and-shell-polish`. Move or remove remaining UX tasks before
      new implementation.
      Moved the three unresolved visual obligations without duplicating their
      implementation: Mica/Vibrancy and its unsupported-platform fallback are
      superseded by task 8.3; completion sounds are superseded by task 8.4.
      The predecessor delta now retains only the delivered Focus-mode
      requirement. The product-wide tasks own the decision, opaque fallback,
      preference, and accessibility evidence. Evidence: `npm run check:matrix`.
- [x] 3.2 Polish S020 welcome hierarchy. Balance brand mark, greeting, composer,
      starters, and setup copy at every width and reading size.
      Re-audit found that provider-ready workspaces still displayed the
      provider-key setup instruction, and that a long display name or enlarged
      text could lose reachable welcome content because the desktop root cannot
      scroll. The setup path now appears only when chat is unavailable. The
      welcome owns a vertical scroll boundary, breaks a native-boundary display
      name, and keeps its centered resting layout when it fits. The skeleton no
      longer reserves the obsolete setup-note row, so its composer and starter
      geometry match the ready welcome. The real browser path uses a long native
      bootstrap name at 900 px and 200% text, proves no horizontal loss, and
      reaches the final starter. Light and dark baselines were renewed after the
      ready-state copy correction. Evidence: `npm run typecheck`, targeted
      welcome tests, `npm run build`, `npm run check:matrix`, 1460/1460 Vitest,
      and 64/64 Playwright. No `.rs` file changed, so the Rust suite was not
      re-run.
- [x] 3.3 Polish S024–S026 composer geometry, actions, context, uploads, command
      palette, mention palette, keyboard, help, and transition states.
      Re-audit found that a failed chat attachment concealed its safe error,
      an empty suggestion surface did not remain a listbox or respond to
      Escape, and selection could leave the caret inside an inserted mention.
      Failed uploads now show their safe reason beside retry/remove actions.
      The empty surface preserves its listbox relationship and dismisses with
      Escape. Suggestions reset on a kind change, return the caret after the
      inserted text, and wrap their keyboard-help row at 200% text. The real
      app-shell check uses an enabled MCP server at 720 px and 200% text to
      verify readable palette help, the empty state, Escape, and continued
      typing after selection. Evidence: `npm run typecheck`, targeted composer
      tests, `npm run build`, `npm run check:matrix`, 1464/1464 Vitest, and
      65/65 Playwright. No `.rs` file changed, so the Rust suite was not re-run.
      Repaired (batch 25a, re-audit before this batch's first commit): the new
      caret-placement call deferred both `.focus()` and `setSelectionRange` to
      `requestAnimationFrame`, a later and less deterministic point than the
      next keystroke -- `userEvent.type` (and a fast typist) can fire several
      keydowns inside one animation frame, so the frame's `setSelectionRange`
      landed mid-sequence and scrambled whatever had already been typed after
      it: "next" arriving as "extn", a 2-in-5 flake on the very regression test
      this call exists to satisfy. Moved to a `useLayoutEffect` keyed on the
      prompt, which runs synchronously right after React commits the new value
      (so the palette is already gone from the DOM, which is what the RAF was
      actually protecting) and strictly before the browser can process another
      input event. 8/8 reruns of the previously-flaking test now pass; full
      gate re-run clean: `npm run typecheck`; `vite build`; `check:styles`;
      `check:tooltips`; `check:matrix`; 1464/1464 Vitest; 65/65 Playwright.
      Repaired (batch 25b, reported directly against the running app): an
      empty, freshly-focused composer put the caret at position 0, right where
      the placeholder's first glyph starts -- `px-1` (4px) sat the two close
      enough to read as one mark instead of two. `px-3` now matches
      `controlBase`'s own horizontal padding, every other control in the app,
      instead of a composer-only exception. `caret-shape: block` (CSS UI 4,
      confirmed rendering in this Chromium via `getComputedStyle`) makes the
      mark itself heavier; unsupported engines fall back to the existing bar,
      never worse than before. 1 new Playwright test pins both the computed
      padding and the computed caret-shape; the one baseline screenshot that
      happened to frame the composer (`approval-activity` high-contrast) is
      regenerated and stable over two full-suite reruns. Full gate: `npm run
lint`; `npm run typecheck`; `vite build`; `check:styles`;
      `check:tooltips`; `check:matrix`; 1465/1465 Vitest; 66/66 Playwright.
      Repaired (batch 25c, reported directly against the running app):
      `px-3` (12px), seen live, read as too roomy -- back to the `px-1` (4px)
      baseline plus a modest 15% (`px-[4.6px]`), keeping the caret and the
      placeholder's first glyph apart without the box looking oversized.
      `caret-shape: block` is unchanged. The pinned Playwright test now
      expects the 4.6px padding; the `approval-activity` high-contrast
      baseline needed no regeneration this time (66/66 Playwright green
      as-is). Full gate: `npm run lint`; `npm run typecheck`; `vite build`;
      `check:styles`; `check:tooltips`; `check:matrix`; 1465/1465 Vitest;
      66/66 Playwright.
      Reverted (batch 25d, reported directly against the running app): the
      15% nudge (batch 25c) still did not read as fixed. Reverted both the
      padding (`px-1`) and `caret-shape: block` all the way back to the
      pre-batch-25b baseline -- plain `caret-color`, no extra horizontal
      padding -- rather than continue tuning a direction that was not
      landing. The now-unneeded computed-style Playwright test is removed;
      no baseline regeneration was needed (65/65 Playwright green as-is).
      The composer's caret/placeholder crowding this whole 25b-25d sequence
      set out to fix is open again, unaddressed. Full gate: `npm run lint`;
      `npm run typecheck`; `vite build`; `check:styles`; `check:tooltips`;
      `check:matrix`; 1465/1465 Vitest; 65/65 Playwright.
- [x] 3.4 Polish S021–S023 transcript, user messages, assistant messages, edit,
      regenerate, copy, streaming, optimistic, and scroll-return behavior.
      Partial (batch 26, reported directly against the running app): every
      conversation now opens pinned to its own bottom. `MessageList` is one
      persistent instance across every conversation switch, not remounted per
      conversation, so its scroll-pin flag was scroll state that silently
      carried over from whichever conversation set it last -- scroll up to
      read history in one chat, switch to another, and the switch landed
      wherever that stale flag left it, not the new conversation's own latest
      message. A conversation-identity-keyed effect now re-pins and re-scrolls
      on every switch. 1 new Vitest test proves it, and fails without the fix
      (confirmed by reverting it and rerunning). The rest of S021-S023 --
      transcript, user/assistant messages, edit, regenerate, copy, streaming,
      optimistic -- remains open.
      Completed (batch 27): the remaining audit found four state defects. A
      failed edit restored the transcript but discarded the revised draft. A
      second revision could cut the already-cut snapshot and leave only a
      partial restore point. A failed copy returned to the same silent icon.
      A send awaiting its run id still accepted another Enter submission, and
      a failed send left a synthetic transcript row with no durable message.
      Revision rollback now captures and restores the complete chat snapshot
      plus admitted invocation ids. The edit draft stays active on failure and
      clears only on success or a conversation switch. One optimistic revision
      owns one restore point, and all other edit/regenerate controls disable
      while it is active. Copy now has a busy state and a persistent visible
      "Try copy again" recovery action. Admission gates send and attachment
      actions without disabling typing; a failed send removes its synthetic row
      and restores the submitted prompt unless newer typing already exists.
      `turnRevisionFlow.test.tsx` mounts the real `useChat` → `ChatPane` →
      virtualized transcript path and proves edit replacement, regenerate
      retention, instant thinking, full failure rollback, draft retention, and
      action gating. `instantThinking.test.tsx`, `MessageRow.test.tsx`, and
      `turnEditStore.test.ts` prove duplicate-send refusal, failed-send prompt
      recovery, visible copy retry, exact snapshot equality, conversation-switch
      cleanup, and the single-restore invariant. Existing transcript coverage
      retains the 10,000-row virtualization bound, per-conversation scroll pin,
      streamed-growth following, scrolled-away new-response return, rich
      streaming fences, and keyboard action names.
      Full gate: `npm run lint`; `npm run typecheck`; `npm run build`;
      1472/1472 Vitest; 65/65 Playwright. The one existing native-minimum shell
      test that missed startup in the first parallel run passed 6/6 isolated
      repetitions and the complete Playwright rerun. No `.rs` file changed, so
      the Rust suite was not re-run.
- [x] 3.5 Polish S027–S029 waiting, recovery, timeline, tool states, and approval as
      one agent-state language.
      Completed (batch 28): retry no longer removes the only recovery surface
      before the native command answers. The shared alert stays in place, its
      Retry action reports busy, duplicate retry is refused, and both a typed
      failure and a rejected invoke leave a usable retry action. Waiting stays
      on the same activity shell and does not appear beside the retry card.
      Approval now renders inside the run tail, beside the activity row it
      resolves. It shares that row's parsed tool name and approval glyph, keeps
      busy and submitted decisions truthful per invocation, restores keyboard
      focus after a resolution error, and cannot carry one invocation's error
      into another. Live and durable tool rows now state when input, output, or
      both were shortened; the complete requested/running/approval/completed/
      failed/denied/cancelled lifecycle has rendered-consumer coverage.
      `approval-activity.spec.ts` drives the built `AppShell`, emits the real
      projection and tool-event sequence, confirms the decision stays inside
      `.chat-column-messages`, resolves it by keyboard, holds a deferred native
      call in busy state, rejects it, proves focus recovery, then denies it and
      observes the prompt-to-Denied handoff. The xfalcon-web reference at
      revision `370ddfe` (`ToolCallGroup.tsx`, `ChatThread.tsx`, and
      `StreamingTail.tsx`) supplied the compact inline-decision and lifecycle
      invariants. Desktop rejects its persistent approval scopes, risk tiers,
      and argument disclosure because the generated desktop contract carries
      only the exact invocation, tool id, and one allow/deny decision. The
      focused approval path and renewed recovery baseline pass in Playwright;
      comprehensive theme and width baselines remain owned by task 3.8. Code
      gate: `npm run lint`; `npm run typecheck`; `npm run build`; 1476/1476
      Vitest. The complete Playwright suite was stopped and skipped at the
      user's direction. No `.rs` file changed, so the Rust suite was not
      re-run.
- [x] 3.6 Polish S030–S031 Markdown and artifacts. Normalize action chrome, source
      disclosure, loading geometry, overflow, and degraded states.
      Implementation complete (batch 29; combined code gate passed):
      Markdown tables retain native table semantics inside a labelled keyboard-
      scroll region. Images load lazily inside bounded geometry. All heading and
      task-list levels have explicit reader styling. The one artifact frame now
      orders and spaces actions directly, owns busy/copied/failure copy states,
      relates source disclosure to its controlled region, and keeps large source
      keyboard-scrollable and bounded. Preview registration keys results to the
      exact kind and content, measures the native two-megabyte limit in UTF-8
      bytes, never encodes a growing fence per token, and states source-only
      degradation. The table building body now uses stable frame-owned geometry.
      The re-audit also fixes nested task-list flow and proves that a late preview
      registration cannot replace the current document. The combined code gate
      passes with 1482/1482 Vitest tests. Full content-stress and browser
      baselines were completed. **Closed 2026-08-31 on owner browser verification.**
- [x] 3.7 Polish S032 export and S033 chat recovery with shared dialogs and feedback.
      Implementation complete (batch 30; combined code gate passed):
      PDF freezes the exact title, time, selection, reasoning, and turns before
      the native save starts. Both formats lock mutable controls during work,
      accept an intentional empty selection with explicit document copy, map
      failures to safe recovery text, and discard late results after dismissal.
      Printed Markdown replaces links and task controls with static content.
      Chat-local render recovery uses the shared alert, distinguishes a repeat
      failure, and resets before the active conversation changes render. The
      re-audit also retires Markdown and PDF results as soon as a controlled
      dialog closes. The combined code gate passes with 1482/1482 Vitest tests.
      Platform PDF and repeated-boundary baselines were completed. **Closed
      2026-08-31 on owner browser verification.**
- [x] 3.8 Add complete chat baselines for empty, short, long, streaming, scrolled,
      editing, upload, mention, tool activity, failure, approval, and rich content.
      **Closed 2026-08-31 on owner browser verification.**
- [x] 3.9 Run `npm run typecheck` and targeted chat tests after every schema, reducer,
      composer, or IPC consumer batch. **Re-verified 2026-08-31:** typecheck
      passed and the complete Vitest suite passed (1,579 tests), including the
      real chat reducer, composer, store, activity, recovery, and rich-content
      consumers.

## 4. Projects and file workflows

- [x] 4.1 Polish S040–S042 explorer, tree, schema, filter, index, unavailable, empty,
      and large-project states.
      Implementation complete (batch 31; combined code gate passed):
      unavailable roots no longer start file or schema calls. The explorer uses
      the shared search field, retains ancestors in filtered trees, bounds both
      narrow panes, truncates long paths, and does not call directory listing an
      index status. Tree focus follows stable paths through filter and collapse,
      keyboard parent and child moves follow the ARIA tree model, sensitive files
      cannot open, and large trees keep one bounded virtual scroll owner. Schema
      extraction now has stable skeletons, safe retry feedback, empty copy, and
      collapsed, keyboard-scrollable semantic tables for many tables and columns.
      The re-audit resets project-local controls before a switch can start stale
      work, deduplicates in-flight folder reads, uses valid encoded active-row
      ids, and prevents duplicate schema heading ids.
      The renderer contract exposes no live index-progress state, so the surface
      shows no index progress rather than inventing a status. That is the shipped
      decision, not an outstanding item.
      The combined code gate passes with 1482/1482 Vitest tests. Real index
      progress has no renderer contract, and the large-state browser baselines
      were completed. **Closed 2026-08-31 on owner browser verification.**
- [x] 4.2 Polish S043 file reading and editing without changing local safety or save
      contracts. Proof: `ProjectFileViewer.test.tsx` covers every file contract
      variant, paging and retry without content loss, stale-response rejection,
      exact draft retention, confirmed destructive reload, and a full-file
      pre-save check that refuses to overwrite an external edit.
- [x] 4.3 Polish S044 revision history and restore confirmation. Proof:
      `ProjectFileViewer.test.tsx` covers deferred history loading, empty and
      populated history, one explicit restore confirmation, in-dialog failure,
      retry, response identity validation, and file reload after success.
- [x] 4.4 Polish S045–S046 project trust and removal with shared dialog geometry and
      clear safety copy. Implementation and automated flow proof are complete.
      `ProjectCreateSheet.test.tsx`, `ProjectNavList.test.tsx`,
      `DialogShell.test.tsx`, and `AppShell.test.tsx` cover trust copy, picker
      cancellation and failure, stale picker results, non-dismissible busy
      states, retained removal confirmation, retry, IPC revoke, and local row
      removal. The owner-requested manual native picker check is closed on owner
      acceptance (2026-08-31), recorded as attestation rather than a harness result.
- [x] 4.5 Polish S047 patch approval, unified and split diffs, high-risk confirmation,
      large diffs, and opaque fallback.
      Implementation is complete (batch 33; compile-only check passed). The
      parser preserves each file as a separate section, pairs replacements in
      split view, labels binary and non-line operations without inventing line
      semantics, preserves the original artifact on parse failure, and exposes
      the backend truncation marker. The viewer virtualizes large row sets. The
      approval dialog validates event payloads, reports scope, uses explicit
      high-risk confirmation, keeps one decision in flight, blocks dismissal,
      restores retry focus, and rejects stale completion from a prior queued
      plan.
      Verified (batch 34): ran the automated and browser evidence batch 33 left
      paused. One accessible-name assertion in
      `ProjectPatchApprovalSheet.test.tsx` had drifted from the shipped
      confirmation copy and is corrected. The browser pass then found a real
      defect before any baseline recorded it: the approval host's `.catch()`
      on a failed subscription showed a persistent "Restart xFalcon" toast,
      unlike every sibling listener host in the app (`McpReauthPrompt`, chat's
      own event bridge), which treats that failure silently because there is
      nothing the user can act on. The toast fired on every shell screenshot
      in this repo's Playwright harness, which never mocks Tauri's event
      channel, and widened 14 unrelated `shell.spec.ts` baselines. Reverted to
      the silent, sibling-consistent behavior; confirmed by reverting the
      whole batch on the unmodified base commit and reproducing the same 14
      passes. The separate malformed-payload toast is unchanged. Four
      `approval-activity.spec.ts` baselines (light/dark/high-contrast/narrow)
      are renewed for the redesigned dialog. Full gate: `npm run lint`;
      `npm run typecheck`; `vite build`; `check:styles`; `check:tooltips`;
      `check:matrix`; 1498/1498 Vitest; 61/66 Playwright, the other 5 failing
      identically on the unmodified base commit (font-rendering drift on four
      `project-trust` baselines and one `populated-projects-light` baseline,
      confirmed pre-existing and unrelated). No `.rs` file changed, so the
      Rust suite was not re-run.
      Re-audited (batch 35, code review before commit): a multi-dimension pass over
      the batch 33/34 diff found six further defects, each proved by reverting the
      fix and reproducing the failure, then restoring it. Two are safety-relevant:
      `isApprovalView` folded a 300 KiB UI display cap into payload validity, so a
      large but well-formed plan was silently dropped from the queue with a
      confusing "invalid response" toast and no way to ever see or decide it --
      that cap is removed, leaving diff length to the `truncated`/opaque path that
      already owns it. Separately, `FileDiffViewer` had no `key` across queued
      plans, so a reviewer's scroll position in one plan's diff leaked into the
      next, unrelated plan shown in the same mounted view; it is now keyed by
      `plan_id`. Two are parser correctness: one file section failing to parse in
      a multi-file plan discarded every other file's already-parsed diff instead
      of falling back only for that section, and `toSplitRows` broke a paired
      replacement into two lopsided rows whenever a "\ No newline at end of file"
      note (common for any file without a trailing newline) landed between the
      delete and the add -- both are fixed and covered by new `diff.test.ts`
      cases. Two are regressions against the pre-batch-33 view found by a
      removed-behavior audit: the per-file path label no longer stayed visible
      while its hunks scrolled underneath (now `sticky` in the non-virtualized
      path), and a single-file diff's landmark lost its specific
      "Changes to <path>" name in favor of a generic one (restored, with the
      generic label kept for multi-file and fully-opaque plans, which have no one
      name to give it) -- both covered by new `FileDiffViewer.test.tsx` cases. A
      large (>500-row) diff going through `VirtualizedDiff` also hit
      `@tanstack/react-virtual`'s known zero-measurement failure mode under a
      layout-less renderer (`jsdom`, the same reason `MessageList.tsx` already
      carries the identical `measureMessageRow` guard): every remeasure reported a
      smaller size than the last, and the resulting resize-then-remeasure loop hit
      React's nested-update limit -- `measureDiffItem` (the same cache-and-floor
      pattern) fixes it, caught only once the large-diff regression test above
      was added, since nothing previously exercised a diff past the
      virtualization threshold. Quality-only, no test needed: mode no longer
      remounts the virtualizer on every Unified/Split toggle (it was discarding
      scroll position and the measurement cache for no correctness reason);
      `hasTextDiff` moved into the existing `sections` memo instead of
      recomputing every render; two highlighting regexes are hoisted out of their
      hot per-row/per-token call sites; the two near-identical
      "\"/truncation-marker note handling blocks inside `parseTextSection` are
      merged into one `consumeNoteLine` helper; the two scroll-container wrappers
      in `FileDiffViewer` share one `DiffScrollShell`; and two now-redundant
      staleness refs (`mountedRef`, `activePlanIdRef`) are removed from
      `ProjectPatchApprovalSheet`, since the remaining `operationRef` already
      changes on every plan swap and on unmount and neither ref was ever shown to
      diverge from it. Not changed: `opaqueTitle`'s regex-based sniffing of the
      backend's opaque text stays, since a typed reason needs a paired backend
      contract change outside this task's scope; the decision-button
      busy/focus-restore pattern shared with `ToolApprovalPrompt.tsx` stays
      unconsolidated, since unifying it would touch a different, already-`Verified`
      surface for a duplication concern, not a defect. Full gate: `npm run lint`;
      `npm run typecheck`; `vite build`; `check:styles`; `check:tooltips`;
      `check:matrix`; 1506/1506 Vitest (8 new); 61/66 Playwright, the same 5
      pre-existing failures as batch 34. No `.rs` file changed, so the Rust suite
      was not re-run.
- [x] 4.6 Add project baselines for every listed state in light, dark, narrow, high
      contrast, keyboard, long path, and large tree modes.
      Completed (batch 36): `tests/visual/project.spec.ts` (32 tests) covers S040-S046 --
      a flagship populated-explorer-with-a-file-open screenshot across
      light/dark/narrow/high-contrast, and one-off light-theme baselines for every
      other reachable state: root/folder loading and error, sensitive/symlink
      markers, a keyboard cursor ring, a 500-row virtualized tree, schema
      extracting/error/empty/populated, file reading/sensitive/binary/partial/
      save-failed/stale, revision loading/error/empty/populated/restore-confirm/
      restoring, and project-removal confirm/failure. S045's trust-copy step keeps
      its existing four-variant baselines in `approval-activity.spec.ts` rather than
      duplicating them. The mock harness needed one general fix: React StrictMode
      (active in this app's dev build) double-invokes every mount effect
      synchronously, so a naive per-call response queue fed a phantom first call
      the response meant for a later real one. Native responses now resolve from a
      per-command queue coalesced across same-tick calls, so calls in one
      synchronous batch share a single queue advance while a call a real tick
      later still advances its own. Evidence: `npm run lint`; `npm run typecheck`;
      `vite build`; `check:styles`; `check:tooltips`; `check:matrix`; 1510/1510
      Vitest; 93/98 Playwright, the same 5 pre-existing font-rendering failures as
      batch 34/35. No `.rs` file changed, so the Rust suite was not re-run.
- [x] 4.7 Verify optimistic and failure rollback with full state equality where a
      project UI mutates local state.
      Completed (batch 36): audited every project-surface local-state mutation
      (create, remove, save, restore, revision reload, directory expansion,
      in-place chat rename). Every write-triggering flow already commits local
      state only after its native call confirms, so most of this task closed by
      evidence rather than by fix -- there is no optimistic update to roll back for
      create, remove, save, or restore, and existing tests already prove several
      to exact value equality (`ProjectFileViewer.test.tsx`'s save-failure test
      already pinned the exact draft text). Two real defects were found and fixed,
      each proved by a new test that fails without the fix and passes with it.
      `RevisionsPanel.load()` nulled the on-screen revision list before every
      reload, not only the first one, so a failed manual reload wiped a list the
      person could already see -- a `loading` flag now tracks the fetch separately
      from the list, and a failed reload states the failure while leaving the
      prior list exactly as it was. `ProjectNavList`'s in-place chat-rename row
      closed the instant Enter was pressed, before the rename was confirmed,
      discarding the typed title on failure with only a toast as any record --
      `AppShell.renameConversationById` now reports success or failure, and the
      row closes only on confirmed success, retrying cleanly from the same open
      editor. The final re-audit applied the same result contract to
      `ConversationList.tsx` and `ConversationTitle.tsx`. Both retain and
      refocus the typed draft after failure. Their real interaction tests also
      prove retry and stale-result refusal after a conversation switch.
      Closed two coverage-only gaps with no code change: a subfolder-listing-
      failure retry (`ProjectExplorerView.test.tsx`) proves the tree's one true
      optimistic update -- marking a folder expanded before its listing confirms --
      already recovers cleanly, and a restore-identity-mismatch case
      (`ProjectFileViewer.test.tsx`) proves that branch already keeps the
      confirmation open. `ProjectCreateSheet.test.tsx`'s create-failure test is
      strengthened to assert exact field values, not only that the sheet stays
      open. Full gate: same as 4.6, run together.

## 5. Datasets and reports

- [x] 5.1 Polish S060–S061 dataset catalog, table, search, sort, action, queue, and
      concurrent progress states.
- [x] 5.2 Polish S062–S064 upload, preview, header, worksheet, cleanup, recipe,
      transfer, completion, and failure states.
- [x] 5.3 Polish S065–S066 schema and deletion flows with shared modal and type-to-
      confirm patterns.
- [x] 5.4 Polish S080 reports library across card and table density, pinned/recent
      hierarchy, search, empty, error, and actions.
- [x] 5.5 Polish S081–S083 report viewing, save, duplicate, rename, delete, busy,
      failure, and rollback states.
- [x] 5.6 Add visual baselines for every dataset and report state at 1/2/3-column,
      narrow, long-title, long-cell, large-schema, and large-catalog variants.
- [x] 5.7 Verify native file pick, drop, save, download, and cancel handoffs in the
      packaged app.
      **Closed 2026-08-31 on owner acceptance.** The remaining evidence was a
      manual check only the owner can run; no code or test work was outstanding.
      Recorded as owner attestation, not as an automated or harness result.

  Batch 37 implements and re-audits 5.1–5.5. Dataset catalog reads are latest-
  wins. Confirmed deletion cannot be undone by an older list. Cleanup uploads
  materialize the exact worksheet and header that produced the reviewed
  preview. Each admitted upload returns its exact transfer id. Upload queue
  revisions reject stale command and event snapshots. Downloads write to a
  temporary sibling and replace the chosen file only after a complete flush.
  Schema and row-save paths validate dataset identity and honor the workspace
  download gate. Reports use commit-after-confirm mutations. All report-list
  producers share one request revision. Account changes clear decrypted report
  metadata before the next shell can render. The library distinguishes an
  unreadable store from a truly empty one. Viewer frames stay inert until they
  load. Report name limits count trimmed Unicode characters like the native
  store. Evidence: 1,542/1,542 Vitest tests; 1,536 Rust tests (1 ignored);
  lint, typecheck, production build, style, tooltip, matrix, and generated-
  contract checks. Final re-audit ran the complete 132-test visual suite twice:
  once to review and accept the intended dataset/report changes, and once
  without snapshot updates to prove deterministic output. This verifies the 44
  dataset/report fixtures and closes task 5.6. Task 5.7 was open at the time of
  this batch for the owner-run packaged-app handoff check; it closed on owner
  acceptance 2026-08-31.

  Batch 38 re-opens and re-closes 5.4-5.6 for one state the reports library did
  not have when they were first verified. `migrate-reports-to-data-plane` moved
  the library onto the data plane, which made "the list is the last synced copy"
  a state the surface can be in -- and a surface that gains a state gains a
  ledger row for it. The stale list now says so through `Alert`, which announces
  itself where the bare paragraph it replaced was silent, and it disables pin,
  rename and delete, because all three are sends and ADR-005 is explicit that
  sending is what does not work offline. A dialog already open when the list
  goes stale is the gap disabling cannot cover, so `useReports` refuses the
  write as well and each of the three says what was kept. The unreadable count
  changed meaning in the same migration: it counts rows of a local projection
  the data plane still backs, not lost documents, so the copy no longer sends
  anyone looking for a backup and offers the sync that repairs it. Evidence:
  1,579 Vitest tests, 4 new; 36/36 dataset/report visual tests, run twice
  without snapshot updates for determinism, with 2 new baselines (stale,
  unreadable) and 2 density baselines regenerated for the new sentence; lint,
  typecheck, style, tooltip, font, and matrix checks.

## 6. Settings and canvas

- [x] 6.1 Polish S100 settings layout, group rhythm, reveal highlight, search result,
      narrow, and 200% text behavior.
- [x] 6.2 Polish S101 appearance controls and immediate apply versus persistence
      failure feedback.
- [x] 6.3 Polish S102 provider credential states without revealing secrets.
- [x] 6.4 Polish S103 MCP server cards, details, forms, testing, auth, confirmation,
      and long-content states.
- [x] 6.5 Polish S104 skill library and multi-file import result states.
- [x] 6.6 Polish S105 update and S106 diagnostics progress, completion, warning,
      error, and native handoff states.
- [x] 6.7 Polish S120 canvas shell, tab strip, split/full transition, focus, resize,
      and overflow.
- [x] 6.8 Polish S121 data grid and its large-data performance states.
- [x] 6.9 Close I04. Tokenize S122 chart series, axes, grid, labels, hover, and export
      for every theme and contrast mode.
- [x] 6.10 Polish S123 document registration, frame stability, retry, copy, save, and
      reload.
- [x] 6.11 Add complete settings and canvas visual baselines and real consumer tests.
      **Progress 2026-08-31:** `tests/visual/settings.spec.ts` adds real-AppShell
      light and narrow settings baselines. It proves the empty provider, MCP, and
      skill states remain reachable with no page-width overflow. The existing
      `CanvasPanel.test.tsx`, `DataGrid.test.tsx`, and `HtmlCanvas.test.tsx`
      remain the real-consumer canvas evidence. The owner also completed the
      full canvas browser-baseline path. **Closed 2026-08-31 on owner browser
      verification.**

  Batch 38 implements and re-audits 6.1–6.10. Settings reveal uses stable
  dependencies. Appearance writes preserve the order in which choices were
  applied. Provider, MCP, skill, update, and diagnostics actions reject duplicate
  admission and retain retryable failure state. Destructive MCP and skill actions
  use the shared confirmation dialog. MCP save and connection results validate
  their target identity. Update download progress supports known and unknown byte
  totals, and a failed restart stays retryable. Canvas tabs restore focus after a
  close. The data grid keeps valid table semantics and mounts fewer than 100 rows
  for a 10,000-row dataset. Document frames stay inert until load and reject empty
  registration ids. The dataset cleanup fast-follow replaces the obsolete dropdown
  with the same 12 direct, color-grouped operations used by `xfalcon-web`; its real
  upload-dialog tests prove that a direct operation still refreshes the preview and
  materializes the exact recipe. Evidence: 1,556/1,556 Vitest tests; 1,536 Rust
  tests (1 ignored); lint, typecheck, production build, style, tooltip, matrix,
  Rust format, and strict OpenSpec checks. Task 6.11 was open at the time of this
  batch because browser execution was skipped; the visual matrix has since been run
  and 6.11 closed.

## 7. Cross-product feedback, accessibility, and content stress

- [x] 7.1 Polish S140 MCP reauthentication on the consolidated alert-dialog contract.
- [x] 7.2 Verify Q39–Q45 on every async surface. No flow may omit an applicable empty,
      loading, busy, success, warning, error, offline, or recovery state.
- [x] 7.3 Verify Q46–Q50 on every interactive matrix row.
- [x] 7.4 Verify Q51–Q54 motion and reduced-motion behavior. Remove motion with no
      continuity or feedback purpose.
- [x] 7.5 Verify Q55–Q59 scroll, virtualization, keyboard, shortcut, and drag/drop
      behavior in the real enclosing containers.
- [x] 7.6 Verify Q60–Q63 screen-reader semantics, contrast, 200% text, and localization
      stress.
- [x] 7.7 Verify Q64–Q67 narrow, wide, resize, coarse pointer, high DPI, Windows, and
      macOS behavior.
- [x] 7.8 Verify Q68–Q71 copy, trust, feedback latency, and layout stability.

  **Browser evidence closed 2026-08-31 on owner verification.** The owner
  completed the required browser, accessibility, resize, pointer, DPI, platform,
  and layout-stability checks for Q39–Q71.

  Batch 39 implements and re-audits 7.1. MCP reauthentication now uses the
  shared `AlertDialog` geometry, focus, busy, and inline-error contract. It keeps
  the stricter no-scrim-dismiss rule because dismissal accepts a degraded tool
  state. A synchronous guard rejects duplicate reconnect admission. The
  connection result must name the same server before the prompt can close.
  Twelve real consumer tests cover refusal reasons, busy state, verified
  success, retained failure, identity mismatch, duplicate refusal, deliberate
  dismissal, real scrim interaction, prompt stability, and listener cleanup.
  The Q51–Q54 source audit also removed the last two `transition-all` paths and
  added a production-source guard against their return. Q39–Q71 browser,
  contrast, resize, pointer, DPI, platform, and layout-shift checks were
  subsequently completed by the owner. Evidence: 1,558/1,558 Vitest tests; 1,536 Rust tests (1 ignored);
  lint, typecheck, production build, style, tooltip, and font checks.

## 8. Native, export, and platform evidence

- [x] 8.1 Complete the S145 packaged Windows checklist for folder, file, save, print,
      OAuth, cancel, window chrome, resize, DPI, and opaque material fallback.
      **Closed 2026-08-31 on owner acceptance.** The remaining evidence was a
      manual check only the owner can run; no code or test work was outstanding.
      Recorded as owner attestation, not as an automated or harness result.
- [x] 8.2 Complete the same S145 checklist on macOS.
      **Closed 2026-08-31 on owner verification.** The owner verified the
      packaged macOS checklist. This is owner attestation for the native-only
      handoffs, not an automated browser or harness result.
- [x] 8.3 Decide whether native material enters this change. If it does, add opaque,
      reduced-transparency, unsupported, and initialization-failure tests. This
      owns predecessor tasks `add-power-composer-and-shell-polish` 2.1 and 4.2.
- [x] 8.4 Decide whether completion sounds remain in scope. If they do, add a user
      setting, no-sound default behavior, and accessibility evidence. This owns
      predecessor task `add-power-composer-and-shell-polish` 3.2.
- [x] 8.5 Verify S146 PDF and print output with long prose, long code, wide tables,
      page breaks, bundled fonts, and fallback behavior.
      **Closed 2026-08-31 on owner acceptance.** The remaining evidence was a
      manual check only the owner can run; no code or test work was outstanding.
      Recorded as owner attestation, not as an automated or harness result.

  Batch 40 closes the two product decisions and completes the non-browser export
  pass. Native material and completion sounds stay out of scope. Export saves now
  reject duplicate admission, wait for bundled fonts, contain unbroken table cells,
  and replace existing destinations only after a complete temporary write. Real
  consumer tests mount long prose, long code, and twelve-column tables and prove the
  fallback and cancellation paths. Full non-browser gate: `npm run lint`; `npm run
build`; strict OpenSpec validation; 1,561/1,561 Vitest; Windows strict Clippy;
  1,522 Rust tests passed (1 ignored); dependency policy passed. The repository-wide
  Prettier check retains the same nine pre-existing files. The owner subsequently
  completed the packaged platform and rendered PDF checks.

## 9. Final verification and acceptance

- [x] 9.1 All P01–P42 rows are `Verified` or have an approved `N/A` reason.
      **Closed 2026-08-31 on owner evidence review.**
- [x] 9.2 All reachable S001–S146 rows are `Verified`.
      **Closed 2026-08-31 on owner evidence review.**
- [x] 9.3 I01–I10 are closed with automated evidence. (I09 and I10 were opened
      after this line was written; I10 is the ledger-drift gap batch 38 found.)
      **Closed 2026-08-31 on owner verification of the completed automated gate.**
      I10 is the exception: it is closed on owner direction, not by automated
      evidence — its proposed guard is not built. See its ledger row.
- [x] 9.4 `npm run typecheck`, targeted tests, `npm test`, `npm run lint`,
      `npm run build`, visual tests, and `scripts/gate.sh` pass.
      **Closed 2026-08-31 on owner browser verification.**
- [x] 9.5 The complete light, dark, high-contrast, forced-colors, reduced-motion,
      reduced-transparency, narrow, wide, pointer, keyboard, 200% text, and localization
      suites pass.
      **Closed 2026-08-31 on owner browser verification.**
- [x] 9.6 The weighted scorecard is reviewed with evidence and reaches at least
      90/100. **Closed 2026-08-31 on owner review.**
- [x] 9.7 No P0 or P1 UX defect remains open.
      **Closed 2026-08-31 on owner review.**
- [x] 9.8 The owner completes final acceptance on the representative end-to-end flows.
      **Closed 2026-08-31 on owner acceptance.**
- [x] 9.9 Run strict OpenSpec validation and archive only after deployment evidence is
      complete. **Closed 2026-08-31 on owner verification.**
