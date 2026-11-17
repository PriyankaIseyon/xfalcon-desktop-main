## 1. Contract and native local read

- [x] 1.1 Add the typed optional conversation-preview command by reusing the
      generated `ChatConversationView` payload without editing generated code.
- [x] 1.2 Add `chat_conversation_preview` as an authenticated, account-bound,
      read-only projection lookup with stale availability.
- [x] 1.3 Prove the preview cache-hit and cache-miss paths make zero cloud
      repository requests and do not mutate project or active-chat state.

## 2. Parallel workspace arrival

- [x] 2.1 Start workspace status and chat workspace bootstrap independently on
      authenticated shell mount.
- [x] 2.2 Classify speculative bootstrap errors only after workspace status
      settles, with account-generation protection.
- [x] 2.3 Prove both calls start before either deferred result settles and the
      arrival skeleton gates on both ready-path results.
- [x] 2.4 Prove a not-ready workspace reveals the shell and does not surface a
      speculative chat error.
- [x] 2.5 Run the bounded recent-conversation recovery reconciles concurrently,
      while keeping projection mutation and zombie finalization ordered.
- [x] 2.6 Keep a successful chat bootstrap operational when the parallel status
      summary fails, while preserving an explicit not-ready result.

## 3. Immediate and race-safe navigation

- [x] 3.1 Add a complete optimistic conversation-open snapshot, including
      copied arrays and admitted invocation IDs.
- [x] 3.2 Update the active conversation ID and known title synchronously and
      clear every prior conversation-scoped live field.
- [x] 3.3 Start local preview and authoritative open concurrently.
- [x] 3.4 Admit only the latest account and navigation generation, and coalesce
      repeated clicks on the same opening target.
- [x] 3.5 Restore the exact snapshot on a current transient failure and preserve
      the typed not-found self-heal.

## 4. Loading and stale UI

- [x] 4.1 Add a compact transcript skeleton for a current cache miss without
      replacing the real sidebar or titlebar.
- [x] 4.2 Apply a local preview immediately as stale and remove the skeleton.
- [x] 4.3 Preserve reduced-motion behavior and expose `aria-busy` plus an
      accessible loading status.

## 5. Verification

- [x] 5.1 Prove immediate sidebar highlight and titlebar change before native
      promises resolve.
- [x] 5.2 Prove cache-hit, cache-miss, rapid A-to-B, stale emitted A snapshot,
      transient rollback, and authoritative not-found behavior through real
      hook/store consumers.
- [x] 5.3 Prove rollback restores every store field and admitted invocation ID.
- [x] 5.4 Run generated-contract checks, `npm run typecheck`, targeted frontend
      tests, targeted Rust tests, and the relevant full suites.
- [x] 5.5 Validate this OpenSpec change strictly and mark each task complete only
      after its stated evidence passes.
