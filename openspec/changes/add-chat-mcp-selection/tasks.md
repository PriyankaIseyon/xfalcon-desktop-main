# Verification

- [x] Typed contracts and generated TypeScript agree.
- [x] Conversation metadata bindings round-trip and malformed bindings fail closed.
- [x] Selected runtimes declare and execute only the selected server's tools.
- [x] Renames cannot reassign another server's historical tool namespace.
- [x] Missing, denied, unavailable or repointed selections never fall back to All.
- [x] Initial context, reloads, compactions and cached summaries preserve scope.
- [x] Account changes cannot mix conversation context and runtime credentials.
- [x] Native send, retry, edit and regenerate preserve the immutable chat binding.
- [x] Attachment reservations preserve the new chat's selection.
- [x] Changing selection preserves the old chat and clears inherited context.
- [x] Reopen and account switching restore only the correct chat's selection.
- [x] Composer selector supports keyboard use and active-run gating.
- [x] Relevant frontend checks and Windows native regression tests pass on the verified build snapshot.
- [x] macOS validation limits are recorded in delivery notes.

Evidence and the Windows v0.5.50 relaunch are recorded in `validation.md`.
The passing native result uses a verified build snapshot that excludes unrelated
untracked permission artifacts. Those artifacts remain in the canonical checkout.
