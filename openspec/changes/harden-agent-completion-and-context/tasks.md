## Implementation

- [x] 1. Report stop reasons from every adapter; discard unfinished tool calls.
- [x] 2. Recover from an output-limit cut-off with a notice and a bounded retry.
- [x] 3. Fail a refusal or a full context window with a clear message.
- [x] 4. Add `project.update_plan` and the completion prompt; nudge a reply that ends with open items.
- [x] 5. Report every bad operation at once; drop a modification that already matches.
- [x] 6. Name applied and unapplied operations in a partial failure; mark it retryable.
- [x] 7. Refuse writes to credentials, `.git` and `.xfalcon`; allow env templates.
- [x] 8. Canonical path spelling and case-folded duplicate detection.
- [x] 9. Whole-file replace only for a file one read returns in full.
- [x] 10. Keep a CRLF file CRLF on a model rewrite, an anchored edit, and a viewer save.
- [x] 11. Estimate at three bytes per token, charge images their ceiling, anchor on the provider's count.
- [x] 12. Bootstrap compaction sends the run's tools and the smallest legal trigger.
- [x] 13. Recognise "prompt too long" on every adapter; compact and retry at most twice.
- [x] 14. Stub old tool results, then summarize the oldest history, for models without native compaction.
- [x] 15. Replay a non-replayable compaction as text.
- [x] 16. Keep thinking and attachments across the snapshot reload.
- [x] 17. Release a claim left by a crash when its plan is proposed again.
- [x] 18. Stream the base-hash check instead of buffering the file.
- [x] 19. Keep unsaved edits across file and tab switches; replace an unedited open file that changed on disk.

## Validation

- [x] 20. Adapter tests prove a cut-off call is reported and never emitted, and a corrupt call is still a failure.
- [x] 21. Driver tests prove retry, exhaustion, refusal, continuation, and completion nudges are bounded.
- [x] 22. Dispatch tests prove the protected paths, partial-failure text, unchanged reporting, and line endings.
- [x] 23. Driver tests prove pruning, summarizing, cache reuse, cache invalidation, text replay, and thinking survival.
- [x] 24. Viewer tests prove CRLF save, draft restore, stale-draft drop, and quiet refresh.
- [x] 24a. A context-compaction event brackets every summary and bootstrap request; the label follows it for the active run only.
- [x] 24b. Tolerant anchors, anchor-failure hints, the placeholder guard and search notices are built and covered by tests.
- [x] 24c. The project prompt states the editing, search, plan, output-limit and compaction behavior the desktop now has.
- [ ] 25. A signed-in person asks the assistant to change several HTML files in a project and every file changes.
- [ ] 26. A long chat on a model without native compaction keeps answering after its history is summarized.
- [x] 27. The catalog's `max_output_tokens` for each shipped model is checked against the provider's published limit. Grok 450k, Sonnet 128k, Luna and Sol 128k match OpenRouter's public model list, 2026-09-30.
