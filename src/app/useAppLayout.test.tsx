import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { APP_NARROW_QUERY, useAppLayout } from "./useAppLayout";

interface MatchMediaController {
  matchMedia: typeof window.matchMedia;
  setMatches: (matches: boolean) => void;
  addEventListener: ReturnType<typeof vi.fn>;
  removeEventListener: ReturnType<typeof vi.fn>;
}

function createMatchMedia(initialMatches: boolean): MatchMediaController {
  let matches = initialMatches;
  const listeners = new Set<(event: MediaQueryListEvent) => void>();
  const addEventListener = vi.fn((_type: string, listener: (event: MediaQueryListEvent) => void) => {
    listeners.add(listener);
  });
  const removeEventListener = vi.fn((_type: string, listener: (event: MediaQueryListEvent) => void) => {
    listeners.delete(listener);
  });

  return {
    matchMedia: vi.fn((media: string) => ({
      matches,
      media,
      onchange: null,
      addEventListener,
      removeEventListener,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })) as unknown as MatchMediaController["matchMedia"],
    setMatches(nextMatches: boolean) {
      matches = nextMatches;
      const event = { matches, media: APP_NARROW_QUERY } as MediaQueryListEvent;
      listeners.forEach((listener) => listener(event));
    },
    addEventListener,
    removeEventListener,
  };
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe("useAppLayout", () => {
  it("derives its initial sidebar state from the narrow media query", () => {
    const media = createMatchMedia(true);
    vi.stubGlobal("matchMedia", media.matchMedia);

    const { result } = renderHook(() => useAppLayout());

    expect(media.matchMedia).toHaveBeenCalledWith(APP_NARROW_QUERY);
    expect(result.current.isNarrow).toBe(true);
    expect(result.current.sidebarOpen).toBe(false);
  });

  it("keeps the desktop rail choice separate from the narrow drawer", () => {
    const media = createMatchMedia(false);
    vi.stubGlobal("matchMedia", media.matchMedia);
    const { result } = renderHook(() => useAppLayout());

    expect(result.current.sidebarOpen).toBe(true);
    act(() => result.current.toggleSidebar());
    expect(result.current.sidebarOpen).toBe(false);
    act(() => result.current.setSidebarOpen(true));
    expect(result.current.sidebarOpen).toBe(true);

    act(() => media.setMatches(true));

    expect(result.current.isNarrow).toBe(true);
    expect(result.current.sidebarOpen).toBe(false);
    act(() => result.current.setSidebarOpen(true));
    expect(result.current.sidebarOpen).toBe(true);

    act(() => media.setMatches(false));

    expect(result.current.isNarrow).toBe(false);
    expect(result.current.sidebarOpen).toBe(true);

    act(() => result.current.toggleSidebar());
    expect(result.current.sidebarOpen).toBe(false);
    act(() => media.setMatches(true));
    act(() => media.setMatches(false));
    expect(result.current.sidebarOpen).toBe(false);
  });

  it("listens only to screen width, so a print page cannot close the sidebar", () => {
    const media = createMatchMedia(false);
    vi.stubGlobal("matchMedia", media.matchMedia);
    const { result } = renderHook(() => useAppLayout());

    expect(APP_NARROW_QUERY).toBe("screen and (max-width: 900px)");
    expect(media.matchMedia).toHaveBeenCalledWith("screen and (max-width: 900px)");
    expect(result.current.sidebarOpen).toBe(true);
  });

  it("removes its media-query listener when unmounted", () => {
    const media = createMatchMedia(false);
    vi.stubGlobal("matchMedia", media.matchMedia);
    const { unmount } = renderHook(() => useAppLayout());

    expect(media.addEventListener).toHaveBeenCalledOnce();
    unmount();
    expect(media.removeEventListener).toHaveBeenCalledOnce();
    expect(media.removeEventListener.mock.calls[0]?.[1]).toBe(media.addEventListener.mock.calls[0]?.[1]);
  });
});
