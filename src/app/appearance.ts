import { useSyncExternalStore } from "react";
import type { AppearanceSettings, ReadingSize, ThemePreference } from "../lib/generated/contracts";
import { appearanceGet, appearanceSet } from "../lib/native";

/** What an install with no stored preference renders (spec:
 * settings-appearance). Kept in step with `AppearanceSettings::default` on
 * the Rust side — the two agree so the first frame matches what the
 * subsequent read returns. */
export const DEFAULT_APPEARANCE: AppearanceSettings = {
  version: 1,
  theme: "light",
  reading_size: "small",
  show_reasoning: false,
  show_follow_ups: true,
};

/**
 * The appearance in effect, and who to tell when it changes.
 *
 * Two preferences are stamped onto the document element and read
 * by the token layer, so they need no subscribers. The chat preferences are read by
 * components (spec: chat-reasoning-panel), which is why this module keeps the
 * value rather than only applying it. Same module-store shape as `chatStore`
 * and `composerStore` — the project's convention is one pattern for all of
 * them, not a second state library for the third store.
 */
let current: AppearanceSettings = DEFAULT_APPEARANCE;
let appearanceLoading = false;
let persistenceTail: Promise<void> = Promise.resolve();
const listeners = new Set<() => void>();

export function getAppearance(): AppearanceSettings {
  return current;
}

export function subscribeAppearance(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function commit(next: AppearanceSettings): AppearanceSettings {
  current = next;
  for (const listener of listeners) {
    listener();
  }
  return next;
}

type AppearanceTarget = Pick<HTMLElement, "dataset">;

/** Stamps the choice onto the document element, where the token layer reads
 * it: `[data-theme]` selects the palette, `[data-font-size]` sets the type
 * multiplier. Both are enum values from the contract, never free text, so
 * there is nothing here to escape. `show_reasoning` is not stamped — no
 * stylesheet reads it; the components that draw reasoning do. */
export function applyAppearance(
  appearance: AppearanceSettings,
  target: AppearanceTarget = document.documentElement,
): void {
  target.dataset.theme = appearance.theme;
  target.dataset.fontSize = appearance.reading_size;
}

/**
 * Applies the stored appearance as early as the renderer can ask for it.
 *
 * The default is applied synchronously first so the app never paints an
 * unstyled or wrong-palette frame while the read is in flight, and the stored
 * value replaces it when it lands. A read that fails leaves the default
 * standing — a preference file we cannot reach is not a reason to refuse to
 * render.
 */
export async function installAppearance(
  target: AppearanceTarget = document.documentElement,
): Promise<AppearanceSettings> {
  appearanceLoading = true;
  applyAppearance(DEFAULT_APPEARANCE, target);
  commit(DEFAULT_APPEARANCE);
  try {
    const stored = await appearanceGet();
    applyAppearance(stored, target);
    appearanceLoading = false;
    return commit(stored);
  } catch {
    appearanceLoading = false;
    return commit(DEFAULT_APPEARANCE);
  }
}

/** Whether this window can play a same-document crossfade right now.
 * Checked fresh on every call rather than cached: `prefers-reduced-motion`
 * can change while the app is open, and the API itself is absent on the
 * WKWebView build this app also ships on. */
function canCrossfadeTheme(): boolean {
  return (
    typeof document !== "undefined" &&
    typeof document.startViewTransition === "function" &&
    typeof window !== "undefined" &&
    typeof window.matchMedia === "function" &&
    !window.matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** Applies a choice immediately, then persists it. The order is deliberate:
 * the control must feel instant, and a write that fails is reported by the
 * caller rather than undoing what the user just saw.
 *
 * A genuine theme flip crossfades the whole window instead of snapping the
 * palette instantly. Gated on `theme` alone — reading size and the reasoning
 * toggle change type and visibility, not a palette, and have nothing to
 * crossfade. Where the API is missing or reduced motion is set, this takes
 * the exact same apply-then-commit path as before. */
export async function chooseAppearance(
  appearance: AppearanceSettings,
  target: AppearanceTarget = document.documentElement,
): Promise<AppearanceSettings> {
  const apply = () => {
    applyAppearance(appearance, target);
    commit(appearance);
  };
  if (appearance.theme !== current.theme && canCrossfadeTheme()) {
    const transition = document.startViewTransition(apply);
    // Fire-and-forget: nothing here awaits the animation, so an unhandled
    // rejection (a transition that cannot start, or a page hidden mid-flight)
    // must not surface as noise. Same no-op-catch idiom as `persistenceTail` below.
    transition.ready.catch(() => undefined);
    transition.finished.catch(() => undefined);
  } else {
    apply();
  }
  const persisted = persistenceTail.then(() => appearanceSet(appearance));
  persistenceTail = persisted.then(
    () => undefined,
    () => undefined,
  );
  return persisted;
}

export function withTheme(appearance: AppearanceSettings, theme: ThemePreference): AppearanceSettings {
  return { ...appearance, theme };
}

export function withReadingSize(appearance: AppearanceSettings, reading_size: ReadingSize): AppearanceSettings {
  return { ...appearance, reading_size };
}

export function withShowReasoning(appearance: AppearanceSettings, show_reasoning: boolean): AppearanceSettings {
  return { ...appearance, show_reasoning };
}

export function withShowFollowUps(appearance: AppearanceSettings, show_follow_ups: boolean): AppearanceSettings {
  return { ...appearance, show_follow_ups };
}

/** The whole preference set, for the settings surface that edits it. */
export function useAppearance(): AppearanceSettings {
  return useSyncExternalStore(subscribeAppearance, getAppearance, getAppearance);
}

/**
 * Whether a run's reasoning is offered to the reader.
 *
 * A primitive, not a slice of the settings object: React compares the selected
 * value by reference, so a component that reads only this one must not wake for
 * a theme change (spec: chat-streaming-performance, "Subscribers Re-render Only
 * For Fields They Read").
 */
function readShowReasoning(): boolean {
  return current.show_reasoning;
}

export function useShowReasoning(): boolean {
  return useSyncExternalStore(subscribeAppearance, readShowReasoning, readShowReasoning);
}

function readShowFollowUps(): boolean {
  // Boot does not block rendering on the native preference read. Do not issue
  // a suggestion request until a stored off choice has had a chance to load.
  return !appearanceLoading && current.show_follow_ups;
}

export function useShowFollowUps(): boolean {
  return useSyncExternalStore(subscribeAppearance, readShowFollowUps, readShowFollowUps);
}

/** Restores the module to a just-booted state. Tests only — the app installs
 * the stored value once, at boot.
 *
 * Notifies rather than clearing the listener set: hooks run their cleanups when
 * the tree unmounts, and dropping their subscriptions from under them would
 * leave a mounted component reading a value nothing will tell it about. */
export function resetAppearanceForTests(): void {
  appearanceLoading = false;
  persistenceTail = Promise.resolve();
  commit(DEFAULT_APPEARANCE);
}
