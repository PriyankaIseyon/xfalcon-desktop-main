## Context

There are two independent critical paths.

On first arrival, `AppShell` mounts `useWorkspaceConnection`, but `useChat`
receives `false` until `workspace_status` resolves with `chat_ready`. This makes
the status and conversation-list calls serial.

On navigation, `useChat.openConversation` awaits the cloud-first native open and
only then applies the returned view. The sidebar highlight, titlebar, and
transcript all wait for the same round trip. `ChatSyncService::peek_local`
already provides an account-bound read from `EncryptedChatProjection`, and the
normal reconcile already refreshes that projection and emits a snapshot event.

## Goals

- Make first-arrival duration approach the slower of the two startup calls,
  rather than their sum.
- Make the clicked row and title respond in the same JavaScript task.
- Show an existing local transcript after one local IPC and no network wait.
- Keep one authoritative cloud reconcile for each open.
- Make rapid navigation deterministic.
- Preserve complete optimistic rollback state.

## Decisions

### 1. Start chat bootstrap speculatively after authentication

`useChat` will distinguish permission to attempt bootstrap from the settled
`chat_ready` capability. In the authenticated Tauri shell it will start one
bootstrap immediately, in parallel with `workspace_status`.

A successful chat bootstrap is applied immediately. It is an authenticated
request to the chat data plane and is therefore stronger evidence of chat
connectivity than an absent auxiliary status summary. An explicit not-ready
status still disables chat actions. If the speculative call fails before status
settles, the failure is retained until status can classify it. A permanently
not-ready workspace reveals the normal shell instead of showing a chat error.
An actually ready workspace surfaces a real bootstrap failure.

The effect remains account-generation scoped. A late result from a signed-out
account cannot populate the next account.

After the workspace list arrives, native startup recovery reconciles at most
`STARTUP_RECOVERY_CONVERSATION_LIMIT` conversations. Those independent cloud
reads run as one bounded concurrent batch. Zombie finalization and projection
mutation remain ordered after all reads settle, so concurrent network I/O does
not create concurrent local state mutation.

### 2. Use one local-preview IPC instead of a second cache

Add a typed `chat_conversation_preview` command. It reads only the existing
encrypted projection for the authenticated account and returns:

- `Some(ChatConversationView)` with `availability: stale` for a cache hit; or
- `None` for a cache miss.

The preview command makes no data-plane request, changes no active pointer,
changes no project binding, emits no projection event, and performs no purge.
It is a read optimization only.

The existing `chat_conversation_open` remains the authoritative path. It still
performs exactly one cloud bootstrap, applies server truth to the projection,
restores the project binding, emits the reconciled snapshot, and performs the
typed not-found self-heal.

### 3. Navigate optimistically, then race preview and reconcile

On a valid click, the store synchronously snapshots the complete current view,
clears all conversation-scoped live state, sets the target conversation ID and
known sidebar title, and marks the target as opening. This immediately moves the
sidebar highlight and titlebar.

The client then starts local preview and authoritative open without awaiting one
before the other.

- If preview wins with data, apply it as stale and remove the skeleton while
  keeping the opening marker until authoritative open settles.
- If preview returns no data, keep the compact transcript skeleton.
- If authoritative open succeeds, apply it as live and clear the opening marker.
- If authoritative open returns the existing stale fallback, apply it and clear
  the marker.
- If authoritative open fails transiently, restore the exact pre-click snapshot
  and admitted invocation set, then show the error.
- If it fails with typed not-found, do not restore the dead target. Run the
  existing purge/list-refresh flow. Restore the prior view only when it remains
  a valid different conversation.

The optimistic snapshot includes every store field, copied collection fields,
and `admittedInvocations`. Rollback has a snapshot equality test.

### 4. Latest navigation wins

Each open receives a monotonically increasing request generation. Every preview,
open, error, rollback, and follow-up project refresh checks both account
generation and navigation generation before it can change the visible chat.

Clicking A and then B immediately selects B. A late preview, authoritative view,
or error is ignored. Native projection events already apply only when their
conversation ID equals the current store conversation ID, so an emitted A
snapshot cannot teleport the reader away from B.

Repeated clicks on the currently opening target reuse the in-flight navigation
and do not issue another authoritative open.

### 5. Skeleton only represents a true content miss

`ChatPane` owns the transcript loading surface because it is already the only
subscriber to transcript state. When the current target is opening and no
preview has been applied, it renders a compact transcript skeleton with
`aria-busy=true` and a status label. The sidebar and titlebar remain real and
interactive.

The skeleton uses existing visual tokens and the existing reduced-motion rule.
It does not use a minimum display delay. A fast cache hit replaces it immediately
to avoid adding artificial latency or a flash caused by a timer.

When a local preview is visible during reconcile, the normal stale indicator
makes its status explicit. The old conversation never remains under the new
title.

### 6. Performance evidence

Tests use deferred promises around the real hook/store consumers.

- Startup proves both native calls are invoked before either resolves.
- Navigation proves the active row and title change before either promise
  resolves.
- Cache hit proves local content replaces the skeleton before server resolution.
- Cache miss proves the skeleton remains until authoritative content arrives.
- A-to-B proves late A success and failure cannot replace or roll back B.
- Rollback proves complete snapshot and admitted-invocation restoration.

No wall-clock server SLA is asserted in jsdom. A native test proves preview does
not call the cloud repository. Existing performance harness language continues
to distinguish deterministic client budgets from real-device traces.

## Rejected Alternatives

### Return local data directly from `chat_conversation_open`

This would make the command return before it performs its documented access,
project-binding, and not-found work. It would also hide background reconcile
errors from the caller. A separate read-only preview keeps authority explicit.

### Wait for the server and only add a skeleton

This improves perceived response but leaves valid encrypted content behind a
network wait. It does not solve actual open latency.

### Add TanStack Query or an in-memory transcript cache

The encrypted projection already owns persistence, account isolation, staleness,
and reconciliation. A second cache would add invalidation paths and memory cost
without adding capability.

### Prefetch every recent conversation

It spends CPU, I/O, and memory on conversations the reader may never open. The
local projection already retains useful recent data. Read on selection is the
smallest correct path.

## Risks and Mitigations

- **Local content is obsolete:** mark previews stale and replace them with the
  authoritative view.
- **Rapid clicks reorder results:** gate all effects by navigation generation.
- **Optimistic selection fails:** restore the complete prior snapshot only for
  the still-current request.
- **Deleted server row has a local copy:** preserve the existing stale fallback
  contract. A local copy remains readable and marked stale; no deletion is
  inferred from a failure that can be transient.
- **Startup call runs on a not-ready tenant:** classify it after status settles
  and keep chat disabled.
- **Status summary fails after chat connects:** keep the successful bootstrap
  active and let the status surface report its narrower failure.
- **Sensitive cache exposure:** use the existing account-bound encrypted
  projection and existing authenticated command boundary only.

## Migration and Rollback

No data migration is required. The command and state fields are additive within
the current contract version. Rollback removes the preview command and restores
cloud-first rendering; the encrypted projection remains compatible.
