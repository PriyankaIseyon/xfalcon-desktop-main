import { useSyncExternalStore } from "react";

/**
 * The composer's draft, in its own store.
 *
 * It used to live in `chatStore` beside the transcript, which made every
 * keystroke a transcript change: the snapshot was replaced, every subscriber was
 * notified, and the whole application re-rendered to move a caret one character
 * (spec: chat-streaming-performance, "Composer Typing Is Independent Of The
 * Transcript").
 *
 * Draft text is not transcript state and never was. It is not persisted, it is
 * not projected, and nothing outside the composer reads it — so it gets its own
 * listener set, and `ComposerDock` is the only subscriber.
 *
 * Same module-store shape as `chatStore` (`useSyncExternalStore` over plain
 * module state), because the project's convention is one pattern for all of
 * them, not a second state library for the second store.
 */

// One draft per conversation. A single shared string carried whatever was typed
// in one chat into whichever chat was opened next, where one keystroke more
// would send it to the wrong place. The new-chat screen has no conversation, so
// it is the entry with no id.
const NO_CONVERSATION = "";
const drafts = new Map<string, string>();
const listeners = new Set<() => void>();

type DraftOwner = string | null | undefined;

const keyOf = (owner: DraftOwner): string => owner ?? NO_CONVERSATION;

export function getComposerDraft(owner: DraftOwner = null): string {
  return drafts.get(keyOf(owner)) ?? "";
}

export function setComposerDraft(next: string, owner: DraftOwner = null): void {
  // An identical write is not a change. Without this, a controlled input that
  // re-emits the same value would still wake every subscriber.
  const key = keyOf(owner);
  if (next === (drafts.get(key) ?? "")) return;
  if (next === "") {
    drafts.delete(key);
  } else {
    drafts.set(key, next);
  }
  for (const listener of listeners) {
    listener();
  }
}

export function subscribeComposerDraft(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Clears every draft.
 *
 * Called from `resetChatStore` so signing out still leaves nothing of the
 * previous account's typing behind — the guarantee the draft had while it was a
 * field of the chat snapshot.
 */
export function resetComposerDraft(): void {
  if (drafts.size === 0) return;
  drafts.clear();
  for (const listener of listeners) {
    listener();
  }
}

export function useComposerDraft(owner: DraftOwner = null): string {
  return useSyncExternalStore(
    subscribeComposerDraft,
    () => getComposerDraft(owner),
    () => getComposerDraft(owner),
  );
}
