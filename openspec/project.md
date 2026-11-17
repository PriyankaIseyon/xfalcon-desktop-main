# Project Context

## Purpose

xFalcon Desktop — a Tauri v2 desktop client for the xFalcon analytics chat
platform. It is the ONLY run executor in the product: the desktop streams
model runs locally with provider credentials resolved from DA per call and mirrors every
delta to the DA data plane. xfalcon-web is the working reference
implementation of the chat UX — desktop mirrors web's contracts unless a
desktop-specific constraint (offline copies, device credentials, OS shell)
forces a divergence, and every divergence is documented where it happens.

## Tech Stack

- Frontend: React 19 + TypeScript + Vite, Tailwind utilities over a token
  sheet (`src/index.css`, `--xf-*` variables), vitest + Testing Library
- Shell: Tauri v2, Rust (tokio, reqwest, rusqlite, thiserror)
- Contracts: `src/lib/generated/contracts.ts` generated from
  `xfalcon-contracts` (Rust crate); DA OpenAPI pinned by SHA to deployed
  staging — new DA fields require DA merge+deploy first
- Backend (data plane): DA server `/api/v1/chat/*` (data-analyst repo);
  auth via Casdoor OAuth

## Project Conventions

### Code Style

- Comments explain why, never what; keep the file's prevailing idioms
- Frontend state: module stores (`chatStore.ts` pattern:
  `useSyncExternalStore` + plain module state), not Redux/zustand
- Rust errors: `thiserror` enums; user-facing strings come from `Display`;
  machine-matched tokens (e.g. `conversation_not_found`) must stay stable

### Architecture Patterns

- Event scoping: run/inference/tool events are admitted only for the active
  conversation's run (`chatStore.ts` `adoptActiveRun` + admitted-invocation
  set); conversation-scoped `run_state_changed` is the adoption seam
- Local-first reads: `EncryptedChatProjection` (sqlite, sealed rows) gives
  stale opens offline; cloud reconcile is the freshness pass
- Lazy conversation create: a row is minted on first send, never on
  "New chat" (web `ensureConversation` parity)
- New Tauri commands require: `APP_COMMANDS` in `security/capabilities.rs`,
  then `lib.rs`, `build.rs`, and the window capability. Tests parse those
  files against `APP_COMMANDS`; they do not own a second list.

### Testing Strategy

- Targeted tests per batch, full suite before push (not per edit)
- Frontend: vitest, colocated `*.test.tsx`; Rust: `src-tauri/src/**/tests/`
- npm builds run in WSL; cargo builds/tests run on Windows
  (`cmd.exe /c "cd /d D:\\work\\xfalcon\\xfalcon-desktop\\src-tauri && cargo …"`)

### Git Workflow

- Feature branches, no direct main commits; commit+push per verified batch
- Every change goes through OpenSpec: proposal → (review) → apply → archive

## Domain Context

- A conversation view (`ChatConversationView`) is the projection the UI
  renders; snapshots are per-account and encrypted at rest
- Runs execute ONLY on this device; an in-flight DA run not owned by this
  process is a zombie and is finalized at bootstrap
- Tool calls are auto-approved by default
  (`XFALCON_REQUIRE_TOOL_APPROVALS=1` restores prompting)

## Important Constraints

- DA OpenAPI pin: desktop consumes the deployed staging spec by SHA
- Windows/WSL split for builds (see Testing Strategy)
- Offline copies must never be silently destroyed; purge only on
  authoritative server answers or explicit user delete

## External Dependencies

- DA server (data-analyst repo) — chat data plane + auth brokering
- Casdoor (auth.mcp-staging.iseyon.com) — OAuth
- OpenRouter/Anthropic — model providers, keys encrypted by DA and resolved per call
- xfalcon-web repo — reference implementation for chat UX contracts
