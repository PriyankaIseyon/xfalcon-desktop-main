# Change: Recover from deleted project folders

## Why

Unavailable project status overflows fixed-height sidebar rows. Confirmed OS deletion also leaves stale projects and associated chats. The user requested automatic cleanup of both, while temporary access failures must remain recoverable.

## What Changes

- Allow project status to occupy its own measured row height; report project availability without a false index diagnosis.
- Revalidate on startup, window focus, and visibility restoration.
- Delete bound chats and revoke trust only after definitive deletion evidence from a retained root handle, or a previously persisted confirmation.
- Preserve moved, renamed, disconnected, and otherwise ambiguous roots. A folder missing when first observed cannot be classified as deleted from its path alone.
- Retain unfinished cleanup state on failure and resume without deleting unrelated chats.

## Impact

- Affected specs: conversation-project-binding, local-project-execution.
- Affected code: project trust/platform helpers, chat cleanup, native project listing, AppShell, project sidebar.
- Add an account-scoped conversation-removal event and a durable local deletion fence. Existing manual project removal continues to preserve chats.
- Unix retained handles can prove unlink. Windows pending deletion cannot prove permanence; release the observer handle and retain the unavailable project and chats. Ignore legacy unversioned deletion flags.
