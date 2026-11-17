# Change: Desktop response actions and contextual follow-ups

## Why

Desktop should offer the response feedback and next-prompt flow already available in xfalcon-web. Copy and regenerate should be visible without hover.

## What Changes

- Keep copy and regenerate visible in the reserved action slot.
- Add icon-only helpful/unhelpful actions beside regenerate. Hydrate, switch, and clear the current user's votes through the same DA message-feedback endpoints as web.
- Generate up to three contextual follow-up prompts through the desktop's existing native provider transport. Send a selected prompt through the normal chat submission path.
- Match web's local suggestion fallback when optional generation fails. Suppress suggestions during active turns and discard stale results after navigation.

## Impact

- Affected spec: chat-thread-layout.
- Affected code: native chat commands/contracts, DA feedback adapter, provider utility call, chat surface and message actions.
- No DA schema change. Native transport replaces web's cookie-authenticated Next.js route because desktop executes provider calls locally.
- Implementation authorized by the user's explicit three-part request in this task.
