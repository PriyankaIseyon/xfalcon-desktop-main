import { useSyncExternalStore } from "react";
import type { Platform } from "./generated/contracts";

/**
 * The host platform, published once at bootstrap.
 *
 * `App` already writes it to `document.documentElement.dataset.platform` for
 * the stylesheet. A component cannot read that during render without reaching
 * into the DOM, and the value arrives asynchronously, so it is also held here
 * and subscribed to — the same shape as the other small stores in this app.
 *
 * `unknown` until bootstrap answers. Every consumer has to render something
 * before then, and the non-Apple spelling is the safe thing to render: it is
 * words rather than glyphs, so it is legible on any platform if the guess is
 * ever wrong for the half-second before the real value lands.
 */
let platform: Platform = "unknown";
const listeners = new Set<() => void>();

export function setPlatform(next: Platform): void {
  if (next === platform) return;
  platform = next;
  for (const listener of listeners) listener();
}

export function getPlatform(): Platform {
  return platform;
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function usePlatform(): Platform {
  return useSyncExternalStore(subscribe, getPlatform, getPlatform);
}

/** True where shortcut labels use Apple's glyphs and run together unseparated. */
export function isApplePlatform(value: Platform): boolean {
  return value === "macos";
}

export function resetPlatformForTests(): void {
  platform = "unknown";
  for (const listener of listeners) listener();
}
