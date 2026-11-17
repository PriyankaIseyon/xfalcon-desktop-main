import { act, cleanup, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GREETING_MS } from "./greeting";
import { useDailyGreeting } from "./useDailyGreeting";

const claimMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({ delightGreetingClaim: claimMock }));

async function settle() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

describe("useDailyGreeting", () => {
  beforeEach(() => {
    claimMock.mockReset();
    vi.useFakeTimers();
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("greets when the day is unclaimed, then leaves on its own", async () => {
    claimMock.mockResolvedValue(true);
    const { result } = renderHook(() => useDailyGreeting(true, "Jeevan"));
    expect(result.current).toBeNull();
    await settle();
    expect(result.current).toEqual(expect.any(String));
    act(() => void vi.advanceTimersByTime(GREETING_MS - 1));
    expect(result.current).not.toBeNull();
    act(() => void vi.advanceTimersByTime(1));
    expect(result.current).toBeNull();
  });

  it("says nothing when the day is already claimed", async () => {
    claimMock.mockResolvedValue(false);
    const { result } = renderHook(() => useDailyGreeting(true, "Jeevan"));
    await settle();
    expect(result.current).toBeNull();
  });

  it("does not claim the day on a screen that is not the new-chat screen", async () => {
    claimMock.mockResolvedValue(true);
    const { result } = renderHook(() => useDailyGreeting(false, "Jeevan"));
    await settle();
    expect(result.current).toBeNull();
    expect(claimMock).not.toHaveBeenCalled();
  });

  it("keeps the greeting through a remount that lands while the claim is pending", async () => {
    claimMock.mockResolvedValue(true);
    const first = renderHook(() => useDailyGreeting(true, null));
    first.unmount();
    const second = renderHook(() => useDailyGreeting(true, null));
    await settle();
    expect(claimMock).toHaveBeenCalledTimes(1);
    expect(second.result.current).not.toBeNull();
  });

  it("does not claim again when the name arrives after the screen", async () => {
    claimMock.mockResolvedValue(true);
    const { result, rerender } = renderHook(({ name }) => useDailyGreeting(true, name), {
      initialProps: { name: null as string | null },
    });
    rerender({ name: "Jeevan" });
    await settle();
    expect(claimMock).toHaveBeenCalledTimes(1);
    expect(result.current).not.toBeNull();
  });
});
