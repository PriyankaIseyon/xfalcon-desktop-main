import { useSyncExternalStore } from "react";

/**
 * Why the person was signed out, shown on the sign-in screen until the next
 * sign-in. It lives outside the shell because the shell unmounts on sign-out.
 *
 * Kept free of `authStore` imports: `authStore` clears this when a new session
 * is authenticated, so importing it back would make a cycle.
 */
type Listener = () => void;

let notice: string | null = null;
const listeners = new Set<Listener>();

export function getSignedOutNotice(): string | null {
  return notice;
}

export function setSignedOutNotice(text: string): void {
  notice = text;
  emit();
}

export function clearSignedOutNotice(): void {
  if (notice === null) return;
  notice = null;
  emit();
}

function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function emit(): void {
  for (const listener of listeners) {
    listener();
  }
}

export function useSignedOutNotice(): string | null {
  return useSyncExternalStore(subscribe, getSignedOutNotice, getSignedOutNotice);
}
