import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { TAP_GAP_MS, useTaps } from "./useTaps";

function tap(handler: () => void, times: number, gap = 100) {
  for (let i = 0; i < times; i += 1) {
    act(() => {
      handler();
      vi.advanceTimersByTime(gap);
    });
  }
}

describe("useTaps", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("fires on the seventh tap in a row and not before", () => {
    const reached = vi.fn();
    const { result } = renderHook(() => useTaps(7, reached));
    tap(result.current, 6);
    expect(reached).not.toHaveBeenCalled();
    tap(result.current, 1);
    expect(reached).toHaveBeenCalledTimes(1);
  });

  it("starts again after reaching the count", () => {
    const reached = vi.fn();
    const { result } = renderHook(() => useTaps(7, reached));
    tap(result.current, 7);
    tap(result.current, 6);
    expect(reached).toHaveBeenCalledTimes(1);
    tap(result.current, 1);
    expect(reached).toHaveBeenCalledTimes(2);
  });

  it("starts the run over after a pause", () => {
    const reached = vi.fn();
    const { result } = renderHook(() => useTaps(7, reached));
    tap(result.current, 4);
    act(() => void vi.advanceTimersByTime(TAP_GAP_MS + 1));
    tap(result.current, 6);
    expect(reached).not.toHaveBeenCalled();
    tap(result.current, 1);
    expect(reached).toHaveBeenCalledTimes(1);
  });

  it("calls the latest callback, not the first", () => {
    const first = vi.fn();
    const second = vi.fn();
    const { result, rerender } = renderHook(({ onReached }) => useTaps(2, onReached), {
      initialProps: { onReached: first },
    });
    rerender({ onReached: second });
    tap(result.current, 2);
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });
});
