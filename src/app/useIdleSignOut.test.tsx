import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  IDLE_CHECK_INTERVAL_MS,
  IDLE_DIALOG_HOLD_MS,
  IDLE_RETRY_MS,
  IDLE_SIGN_OUT_MS,
  idleSignOutNotice,
  useIdleSignOut,
} from "./useIdleSignOut";

const SECOND = 1_000;
const MINUTE = 60 * SECOND;
const HOUR = 60 * MINUTE;

let clock = 0;
const now = (): number => clock;
let onIdle = vi.fn<() => void | Promise<boolean | void>>();

/** Moves the clock in one-second steps so every timer callback sees `now()` at the same instant. */
function advance(ms: number): void {
  let remaining = ms;
  while (remaining > 0) {
    const step = Math.min(remaining, SECOND);
    clock += step;
    vi.advanceTimersByTime(step);
    remaining -= step;
  }
}

function fire(type: string): void {
  window.dispatchEvent(new Event(type));
}

function fireVisibilityChange(): void {
  document.dispatchEvent(new Event("visibilitychange"));
}

function mount(busy = false, isBusy?: () => boolean) {
  return renderHook((props: { busy: boolean }) => useIdleSignOut({ onIdle, busy: props.busy, isBusy, now }), {
    initialProps: { busy },
  });
}

beforeEach(() => {
  vi.useFakeTimers();
  clock = 0;
  onIdle = vi.fn<() => void | Promise<boolean | void>>();
  Object.defineProperty(document, "visibilityState", { configurable: true, get: () => "visible" });
});

afterEach(() => {
  vi.useRealTimers();
  vi.restoreAllMocks();
  Reflect.deleteProperty(document, "visibilityState");
});

describe("useIdleSignOut", () => {
  it("does not sign out before 12 hours of silence", () => {
    mount();

    advance(IDLE_SIGN_OUT_MS - SECOND);

    expect(onIdle).not.toHaveBeenCalled();
  });

  it("signs out once at 12 hours and never again", () => {
    mount();

    advance(IDLE_SIGN_OUT_MS);
    expect(onIdle).toHaveBeenCalledTimes(1);

    advance(5 * HOUR);
    fireVisibilityChange();
    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("resets the clock on keydown and pointermove", () => {
    mount();

    advance(11 * HOUR);
    fire("keydown");
    advance(11 * HOUR);
    fire("pointermove");

    advance(IDLE_SIGN_OUT_MS - SECOND);
    expect(onIdle).not.toHaveBeenCalled();

    advance(SECOND);
    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("counts two events 100 ms apart as one activity", () => {
    mount();

    advance(11 * HOUR);
    fire("keydown");
    advance(100);
    fire("pointermove");

    // The first event at 11 h is the recorded activity, so the limit is reached at 23 h.
    advance(IDLE_SIGN_OUT_MS - 100);
    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("signs out on the first check after a sleep that froze the timers", () => {
    mount();

    // No ticks fire during sleep; only the wall clock moves.
    clock += 13 * HOUR;
    fireVisibilityChange();

    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("does not sign out while busy and restarts the full period when the work ends", () => {
    const { rerender } = mount(true);

    advance(IDLE_SIGN_OUT_MS);
    expect(onIdle).not.toHaveBeenCalled();

    advance(6 * HOUR);
    expect(onIdle).not.toHaveBeenCalled();

    rerender({ busy: false });

    advance(IDLE_SIGN_OUT_MS - MINUTE);
    expect(onIdle).not.toHaveBeenCalled();

    advance(MINUTE);
    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("restarts the clock when the time moves backwards", () => {
    mount();

    advance(6 * HOUR);
    fire("keydown");

    clock = HOUR;
    advance(MINUTE);

    // The period now counts from the backwards jump at 1 h, not from the stale 6 h activity.
    advance(IDLE_SIGN_OUT_MS - MINUTE);
    expect(onIdle).not.toHaveBeenCalled();

    advance(MINUTE);
    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("removes every listener and the interval on unmount", () => {
    const windowAddSpy = vi.spyOn(window, "addEventListener");
    const windowRemoveSpy = vi.spyOn(window, "removeEventListener");
    const documentAddSpy = vi.spyOn(document, "addEventListener");
    const documentRemoveSpy = vi.spyOn(document, "removeEventListener");
    const { unmount } = mount();
    expect(vi.getTimerCount()).toBe(1);

    unmount();

    expect(vi.getTimerCount()).toBe(0);
    const hookWindowTypes = new Set<string>([
      "pointerdown",
      "pointermove",
      "keydown",
      "wheel",
      "touchstart",
      "focus",
      "online",
    ]);
    const windowListeners = windowAddSpy.mock.calls.filter(([type]) => hookWindowTypes.has(type));
    expect(windowListeners).toHaveLength(hookWindowTypes.size);
    for (const [type, listener] of windowListeners) {
      expect(windowRemoveSpy).toHaveBeenCalledWith(type, listener);
    }
    const visibilityListeners = documentAddSpy.mock.calls.filter(([type]) => type === "visibilitychange");
    expect(visibilityListeners).toHaveLength(1);
    for (const [type, listener] of visibilityListeners) {
      expect(documentRemoveSpy).toHaveBeenCalledWith(type, listener);
    }
  });

  it("stays fired and never retries while the sign-out resolves to true", async () => {
    onIdle = vi.fn<() => Promise<boolean | void>>().mockResolvedValue(true);
    mount();

    advance(IDLE_SIGN_OUT_MS);
    await act(async () => {});
    advance(5 * HOUR);
    fireVisibilityChange();
    await act(async () => {});

    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("re-arms after a false result and tries again IDLE_RETRY_MS later", async () => {
    onIdle = vi.fn<() => Promise<boolean | void>>().mockResolvedValueOnce(false).mockResolvedValue(true);
    mount();

    advance(IDLE_SIGN_OUT_MS);
    await act(async () => {});
    expect(onIdle).toHaveBeenCalledTimes(1);

    advance(IDLE_RETRY_MS - MINUTE);
    expect(onIdle).toHaveBeenCalledTimes(1);

    advance(MINUTE);
    expect(onIdle).toHaveBeenCalledTimes(2);
  });

  it("re-arms after a rejected sign-out and tries again IDLE_RETRY_MS later", async () => {
    onIdle = vi
      .fn<() => Promise<boolean | void>>()
      .mockRejectedValueOnce(new Error("logout failed"))
      .mockResolvedValue(true);
    mount();

    advance(IDLE_SIGN_OUT_MS);
    await act(async () => {});
    expect(onIdle).toHaveBeenCalledTimes(1);

    advance(IDLE_RETRY_MS - MINUTE);
    expect(onIdle).toHaveBeenCalledTimes(1);

    advance(MINUTE);
    expect(onIdle).toHaveBeenCalledTimes(2);
  });

  it("postpones the sign-out while isBusy is true and restarts the clock from the last busy check", () => {
    let dialogOpen = false;
    mount(false, () => dialogOpen);

    advance(11 * HOUR);
    dialogOpen = true;
    advance(2 * HOUR);
    dialogOpen = false;

    advance(IDLE_SIGN_OUT_MS - MINUTE);
    expect(onIdle).not.toHaveBeenCalled();

    advance(MINUTE);
    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("signs out after a day without input even if a dialog stays open", () => {
    mount(false, () => true);

    advance(IDLE_DIALOG_HOLD_MS - MINUTE);
    expect(onIdle).not.toHaveBeenCalled();

    advance(MINUTE);
    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("restarts the day when the person types into the open dialog", () => {
    mount(false, () => true);

    advance(IDLE_DIALOG_HOLD_MS - MINUTE);
    fire("keydown");

    advance(IDLE_DIALOG_HOLD_MS - MINUTE);
    expect(onIdle).not.toHaveBeenCalled();

    advance(MINUTE);
    expect(onIdle).toHaveBeenCalledTimes(1);
  });

  it("never signs out under running work, however long the dialog or the silence", () => {
    mount(true, () => true);

    advance(IDLE_DIALOG_HOLD_MS * 2);

    expect(onIdle).not.toHaveBeenCalled();
  });

  it("keeps listeners and the interval installed after firing and removes them on unmount", async () => {
    onIdle = vi.fn<() => Promise<boolean | void>>().mockResolvedValue(true);
    const windowRemoveSpy = vi.spyOn(window, "removeEventListener");
    const documentRemoveSpy = vi.spyOn(document, "removeEventListener");
    const { unmount } = mount();

    advance(IDLE_SIGN_OUT_MS);
    await act(async () => {});
    expect(onIdle).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(1);
    expect(windowRemoveSpy).not.toHaveBeenCalled();
    expect(documentRemoveSpy).not.toHaveBeenCalled();

    unmount();

    expect(vi.getTimerCount()).toBe(0);
    expect(windowRemoveSpy).toHaveBeenCalledWith("keydown", expect.any(Function));
    expect(documentRemoveSpy).toHaveBeenCalledWith("visibilitychange", expect.any(Function));
  });

  it("uses a 12 hour limit and an exact notice", () => {
    expect(IDLE_SIGN_OUT_MS).toBe(12 * 60 * 60 * 1000);
    expect(IDLE_CHECK_INTERVAL_MS).toBe(MINUTE);
    expect(idleSignOutNotice()).toBe("Signed out after 12 hours of inactivity. Sign in to continue.");
  });
});
