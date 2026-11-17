# Refine run feedback UX — stop-in-send, in-transcript activity, settings cleanup

## Why

Three pieces of run feedback fight the reading surface today (user report,
2026-08-05, with screenshot):

1. Cancelling a run is a dedicated "Cancel run" pill floating at the top of
   the transcript — far from where the user's attention is (the composer),
   and rendered even when the answer is streaming right under it. xfalcon-web
   integrates stop into the composer's send control (`Composer.tsx:1402`,
   `Square` icon, `aria-label="Stop the assistant's response"`).
2. Tool activity renders as a bare "Thinking…" pill floating at the bottom,
   plus a separate "Activity" accordion BELOW the composer (outside the
   transcript entirely). Web interleaves an agent-steps card into the message
   flow. The user asked for: the card just below the user's message,
   auto-expanded with all tool calls while the run generates, auto-collapsed
   once the response is ready, manually expandable afterwards.
3. The Settings page shows a "Provided by your workspace" tenant-server card
   whose copy ("Every call to this server pauses for your approval") became
   false when auto-approve landed. The user asked for the card's removal.

Also from the screenshot: while a run is pending the composer placeholder
reads "Chat is not connected yet" — a false statement (the runtime is
connected; a run is just in flight).

## What Changes

- Composer's circular send control becomes a stop control while a run is in
  flight (Square icon, always enabled, calls the cancel path). The top
  "Cancel run" pill row is removed.
- New `AgentActivityCard` in the transcript flow (after the last message):
  auto-open while the run is active, auto-collapse on run end, manual toggle
  any time. Replaces both the floating ThinkingPill (when tool calls exist)
  and the below-composer `ActivityDisclosure`.
- The composer keeps the "Ask anything" placeholder while a run is pending;
  the disabled-runtime copy appears only when the runtime is actually not
  connected.
- Settings: the bundled tenant-server card is removed from the MCP servers
  section.

## Impact

- Affected specs: `chat-composer` (modified), `chat-run-activity` (new),
  `settings-mcp-servers` (new)
- Affected code: `src/features/chat/Composer.tsx`, `ChatSurface.tsx`,
  `ThinkingPill.tsx` usage, `src/components/ActivityDisclosure.tsx` (reused
  by the new card), `src/app/AppShell.tsx`,
  `src/features/settings/McpServersSection.tsx`
- No backend changes.
