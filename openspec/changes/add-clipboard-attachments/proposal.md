# Change: Paste chat attachments on Windows and macOS

## Why

The composer currently inserts text but does not attach clipboard files or screenshots. Users need the same attachment flow from paste and from the picker.

## What Changes

- Accept clipboard image/file bytes and native Explorer/Finder file references on paste.
- Reuse chat uploads, progress, cancellation, retry and message binding.
- Enforce 10 MiB per file for all chat attachment types, reject empty/unsupported or inconsistent content, and bound paste batches.
- Keep plain/rich text paste behavior and do not interpret clipboard text as a local path or fetch remote image URLs.

## Impact

- Affected specs: composer-uploads, chat-composer.
- Affected code: composer, chat actions, native upload command/service, native clipboard adapters and capabilities.
- Existing DA attachment contracts remain unchanged. User requested implementation directly on staging.

This change extends the picker-only path-authority statement in `add-composer-uploads`: file references may also come directly from the OS clipboard during paste. Renderer-supplied local paths remain prohibited.
