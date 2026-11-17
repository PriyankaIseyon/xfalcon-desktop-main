import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { AppearanceSettings } from "../lib/generated/contracts";

const appearanceGetMock = vi.hoisted(() => vi.fn());
const appearanceSetMock = vi.hoisted(() => vi.fn());

vi.mock("../lib/native", () => ({
  appearanceGet: appearanceGetMock,
  appearanceSet: appearanceSetMock,
}));

import { chooseAppearance, resetAppearanceForTests, withReadingSize, withTheme } from "./appearance";

const LIGHT: AppearanceSettings = {
  version: 1,
  theme: "light",
  reading_size: "small",
  show_reasoning: false,
  show_follow_ups: true,
};

const matchMediaMock = vi.fn().mockReturnValue({ matches: false } as MediaQueryList);

beforeAll(() => {
  // jsdom does not implement `matchMedia` at all; several other reduced-motion
  // checks in this app (`thinkingPhrase.ts`, `MessageList.tsx`) already guard
  // with `typeof window.matchMedia === "function"` for exactly this reason.
  Object.defineProperty(window, "matchMedia", { configurable: true, writable: true, value: matchMediaMock });
});

afterEach(() => {
  resetAppearanceForTests();
  appearanceGetMock.mockReset();
  appearanceSetMock.mockReset();
  appearanceSetMock.mockResolvedValue(undefined);
  matchMediaMock.mockReset().mockReturnValue({ matches: false } as MediaQueryList);
  document.documentElement.removeAttribute("data-theme");
  document.documentElement.removeAttribute("data-font-size");
  delete (document as { startViewTransition?: unknown }).startViewTransition;
});

/** jsdom has no `startViewTransition` either — every test here stands in for
 * a Chromium build new enough to have shipped it (WebView2, current Chrome),
 * exercising the branch a WKWebView build (no API) or `installAppearance`'s
 * first paint (theme unchanged) never take. */
function stubViewTransition() {
  const startViewTransition = vi.fn((callback: () => void) => {
    callback();
    return { ready: Promise.resolve(), finished: Promise.resolve() };
  });
  Object.defineProperty(document, "startViewTransition", {
    configurable: true,
    writable: true,
    value: startViewTransition,
  });
  return startViewTransition;
}

describe("chooseAppearance theme crossfade", () => {
  it("wraps a genuine theme change in a view transition when the API is available", async () => {
    const startViewTransition = stubViewTransition();
    await chooseAppearance(withTheme(LIGHT, "dark"));
    expect(startViewTransition).toHaveBeenCalledOnce();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("does not start a transition when the theme is unchanged", async () => {
    const startViewTransition = stubViewTransition();
    // `current` is already the light-theme default; only reading size changes.
    await chooseAppearance(withReadingSize(LIGHT, "large"));
    expect(startViewTransition).not.toHaveBeenCalled();
    expect(document.documentElement.dataset.fontSize).toBe("large");
  });

  it("does not start a transition under reduced motion, but still applies the theme", async () => {
    const startViewTransition = stubViewTransition();
    matchMediaMock.mockReturnValue({ matches: true } as MediaQueryList);
    await chooseAppearance(withTheme(LIGHT, "dark"));
    expect(startViewTransition).not.toHaveBeenCalled();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("applies and persists exactly as before when the browser has no view-transition API", async () => {
    // No stub installed: this is the real jsdom (and WKWebView) shape.
    const next = withTheme(LIGHT, "dark");
    await chooseAppearance(next);
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(appearanceSetMock).toHaveBeenCalledWith(next);
  });
});
