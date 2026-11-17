# Change: Make workspace arrival and chat navigation local-first

## Why

The authenticated shell starts `workspace_status` first and starts
`chat_workspace_bootstrap` only after that result says chat is ready. The
arrival skeleton therefore hides two network-dependent operations in series.

Opening a conversation has a second avoidable wait. The client does not update
the active row or title until `chat_conversation_open` completes, and that
command waits for a cloud reconcile before it returns. The desktop already
maintains an account-bound encrypted projection, but the open path uses it only
as a fallback after a server failure.

The result is correct but slow. A click can appear ignored for several seconds,
and a valid local transcript waits behind the network.

## What Changes

- Start workspace status and chat workspace bootstrap concurrently after an
  authenticated desktop shell mounts.
- Reconcile the bounded startup-recovery set concurrently instead of paying up
  to eight independent cloud round trips in series.
- Keep the first-arrival gate until both independent startup results settle, or
  the existing hard cap expires.
- Select the clicked conversation and its known title synchronously.
- Read the existing encrypted projection immediately while the single
  authoritative server reconcile runs in parallel.
- Render the local projection as stale when it exists. Render a compact
  transcript skeleton only on a local cache miss.
- Admit only results that belong to the latest navigation request.
- Preserve the existing server authority, typed not-found self-heal, project
  binding, and one-cloud-request-per-open invariants.
- Add deterministic tests and latency budgets for startup concurrency, immediate
  selection, cache-hit display, cache-miss skeletons, rollback, and rapid
  A-to-B navigation.

## SOTA Basis

This change applies the common local-first read and server-authoritative sync
model. Linear publicly describes a local database with delta sync so navigation
does not need a network round trip. Gmail documents caching message bodies and
using partial sync after the initial full sync. Discord recommends cached client
state updated by events. Slack documents why an unbounded history cache is a
mistake. The desktop already has the correct bounded, encrypted projection, so
this change reuses it instead of adding another cache or dependency.

## Impact

- Affected specs: `workspace-arrival`, `chat-conversation-open`
- Affected frontend: `src/app/AppShell.tsx`, `src/features/chat/useChat.ts`,
  `src/features/chat/chatStore.ts`, `src/features/chat/ChatPane.tsx`, and tests
- Affected native code: chat command registration and the chat service local
  projection read
- Affected contracts: one typed local-preview command that reuses optional
  `ChatConversationView`; generated contract payloads remain unchanged
- Persistence: no schema change and no second cache
- Data Analyst repository: no change. The desktop uses existing APIs and its
  existing encrypted local projection.

## Non-Goals

- Making cached content authoritative.
- Adding a general query-cache library.
- Prefetching every transcript or keeping the whole workspace in memory.
- Changing conversation mutation, run execution, or data-plane endpoints.
- Hiding stale state or treating a network failure as deletion.
