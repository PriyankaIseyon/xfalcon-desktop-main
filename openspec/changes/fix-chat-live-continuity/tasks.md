## Verification

- [x] 1. A blocked history sink does not block answer paint; finalization waits
      for all ordered records, and sustained backlog pauses provider reads.
- [x] 2. Returning to a known active conversation restores answer, reasoning,
      activity, and sequence watermarks without affecting the other conversation.
- [x] 3. Terminal snapshots, stale events, account reset, deletion, and optimistic
      rollback preserve existing isolation and retirement rules.
- [x] 4. MCP reconnect and capability changes refresh validators; concurrent
      readers cannot restore a stale catalog.
- [x] 5. Per-call, resumed-call, and round timeouts cancel requests and clean up
      progress state without cancelling unrelated calls.
- [x] 6. Caret tests cover painted endpoints, surrogate pairs, and constant work
      over settled paragraphs; chat browser checks preserve scroll and rendering.
- [ ] 7. Frontend and native regression checks, formatting, lint, and final diff
      review pass with their execution boundaries recorded.
- [x] 8. Failed reopens preserve destination updates and approvals; repeated
      source snapshots preserve failed and interrupted retry state on rollback.
- [x] 9. Session-scoped MCP invalidation preserves a newer connection and its
      catalog while current-session invalidation still reconnects. Refusal cleanup
      preserves late current-generation sign-in and cannot race a replacement.
- [x] 10. Stale tool-call refusals remain retryable after replacement, including
      after that replacement is cleared, and a subsequent real call succeeds.
- [x] 11. A blocked tool listing does not block session invalidation or replacement
      authentication; both invalidated and closed sessions refresh schemas on reconnect.

Evidence and execution limits are recorded in [verification.md](verification.md).
Task 7 remains open for the default parallel native gate. The identical baseline
diagnostics failure is preserved and reported. The full serial Windows suite and
the final whole-workspace Rust format check pass.
