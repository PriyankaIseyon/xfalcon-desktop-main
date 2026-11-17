## 1. Harness

- [ ] 1.1 Add `tauri-driver` and a WebDriver client as dev dependencies. Do not
      reach for Playwright — it cannot drive WebView2 as the app's webview.
- [ ] 1.2 `tests/smoke/` with a launcher that starts `tauri-driver` and the
      built binary, and tears both down on failure as well as success.
- [ ] 1.3 Fail with the build command in the message when the binary is absent.
      Never fall back to Vite or to mocks.
- [ ] 1.4 Point the app at a scratch app-data directory per run, so a smoke run
      cannot read or destroy a real profile.
- [ ] 1.5 `npm run test:smoke`, and state the wall-clock budget in
      `tests/smoke/README.md`.

## 2. Stub provider

- [ ] 2.1 A local HTTP stub that answers the provider's streaming shape.
- [ ] 2.2 Configure it as the device provider through the app's own settings
      path, not by patching a command.
- [ ] 2.3 One deterministic answer that contains a table, a fenced HTML
      document, and reasoning, so one turn exercises several renderers.

## 3. Seam cases

- [ ] 3.1 Launch, reach the workspace, idle past the acknowledgement timeout,
      still running.
- [ ] 3.2 Invoke every registered command with minimal valid arguments; none
      answers unknown-command. Read the list from the same source
      `security/tests.rs` reads, so a new command cannot be missed here.
- [ ] 3.3 A forbidden filesystem write command is refused by the running shell.
- [ ] 3.4 A registered preview document loads, and its origin is the preview
      scheme rather than the app origin.
- [ ] 3.5 Send one turn against the stub; the answer streams and lands in
      storage.
- [ ] 3.6 Export a document through the save path and assert the bytes on disk.
- [ ] 3.7 Create a conversation, restart, and confirm it is present before any
      network call completes.
- [ ] 3.8 Quit clean: no orphaned `tauri-driver`, no leftover scratch profile.

## 4. CI

- [ ] 4.1 A `windows-latest` job that builds the release bundle and runs the
      smoke suite. Triggered by a pull request touching `src-tauri/`, and by
      every tag.
- [ ] 4.2 No retries. Upload the app log and a screenshot per failed case.
- [ ] 4.3 Fail the job when the suite passes its budget, naming the running
      case.

## 5. Visual baselines

- [ ] 5.1 Generate the missing `tests/visual/baselines/chat/` and
      `tests/visual/baselines/approval-activity/` images on a Windows host.
- [ ] 5.2 Make a missing baseline a failure, so an empty set cannot look green
      again.

## 6. Retire the manual tasks

- [ ] 6.1 Walk the 33 changes carrying a "live on Windows" task. For each,
      either name the smoke case that now covers it, or record why it must stay
      manual.
- [ ] 6.2 List the changes that became archivable. Archiving them is a separate
      pass — do not archive here.

## 7. Verify

- [ ] 7.1 The suite passes on a Windows host and inside CI.
- [ ] 7.2 Break one thing on purpose — unregister a command — and confirm the
      suite fails and names it.
- [ ] 7.3 Confirm the suite fails when run with no built binary.
