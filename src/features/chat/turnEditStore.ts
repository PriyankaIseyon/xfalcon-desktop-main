import { useSyncExternalStore } from "react";

export type TurnEditState = {
  activeMessageId: string | null;
  draft: string;
};

let state: TurnEditState = {
  activeMessageId: null,
  draft: "",
};

const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) {
    listener();
  }
}

export function getTurnEditState(): TurnEditState {
  return state;
}

export const getTurnEditSnapshot = getTurnEditState;

export function startTurnEdit(messageId: string, initialDraft: string): void {
  state = {
    activeMessageId: messageId,
    draft: initialDraft,
  };
  notify();
}

export function updateTurnEditDraft(nextDraft: string): void {
  if (state.draft === nextDraft) return;
  state = {
    ...state,
    draft: nextDraft,
  };
  notify();
}

export function clearTurnEdit(): void {
  if (state.activeMessageId === null && state.draft === "") return;
  state = {
    activeMessageId: null,
    draft: "",
  };
  notify();
}

export function subscribeTurnEdit(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useTurnEdit(): TurnEditState {
  return useSyncExternalStore(subscribeTurnEdit, getTurnEditState, getTurnEditState);
}

/**
 * The message being edited, or null. A primitive snapshot: a draft keystroke
 * leaves it unchanged, so a component that reads only this does not re-render
 * while the reader types.
 */
export function useTurnEditActiveMessageId(): string | null {
  return useSyncExternalStore(
    subscribeTurnEdit,
    () => getTurnEditState().activeMessageId,
    () => getTurnEditState().activeMessageId,
  );
}
