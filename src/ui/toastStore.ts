// Module store behind `Toaster` — split from the component so the imperative
// `toast` API can be imported from hooks and plain modules without tripping
// react-refresh's components-only rule on the component file.

export type ToasterTone = "message" | "success" | "warning" | "error";

export interface ToastAction {
  label: string;
  onSelect: () => void;
}

export interface ToasterToast {
  id: number;
  text: string;
  tone: ToasterTone;
  /** Milliseconds before auto-dismiss. `null` keeps the toast until closed. */
  duration: number | null;
  action?: ToastAction;
}

const AUTO_DISMISS_MS = 3000;
/** An error is a problem to act on, so it stays five times as long as a status
 * line: long enough to read and act on, short enough that a stack of them does
 * not linger. The stack pauses every timer while the pointer or focus is in it. */
const ERROR_AUTO_DISMISS_MS = 15_000;

type Listener = () => void;

let nextToastId = 0;
let toasts: ToasterToast[] = [];
const listeners = new Set<Listener>();

function notify() {
  for (const listener of listeners) listener();
}

export function subscribeToasts(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getToasts(): ToasterToast[] {
  return toasts;
}

function push(text: string, tone: ToasterTone, duration?: number | null, action?: ToastAction): number {
  // An error gets the long timer, not the short one: a slow reader, or a screen
  // reader queued behind other speech, still has time for it, and hovering or
  // focusing the stack holds it. A caller that needs it kept passes `null`.
  const resolved = duration === undefined ? (tone === "error" ? ERROR_AUTO_DISMISS_MS : AUTO_DISMISS_MS) : duration;
  const id = nextToastId++;
  toasts = [...toasts, { id, text, tone, duration: resolved, action }];
  notify();
  return id;
}

export function dismissToast(id: number) {
  toasts = toasts.filter((item) => item.id !== id);
  notify();
}

/** Imperative notification API, usable from hooks and plain modules alike. */
export const toast = {
  message(text: string, duration?: number | null, action?: ToastAction) {
    return push(text, "message", duration, action);
  },
  success(text: string, duration?: number | null, action?: ToastAction) {
    return push(text, "success", duration, action);
  },
  warning(text: string, duration?: number | null, action?: ToastAction) {
    return push(text, "warning", duration, action);
  },
  error(text: string, duration?: number | null, action?: ToastAction) {
    return push(text, "error", duration, action);
  },
};

export function resetToastsForTests() {
  toasts = [];
  nextToastId = 0;
  notify();
}
