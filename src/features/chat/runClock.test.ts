import { afterEach, describe, expect, it, vi } from "vitest";

import { beginRunSubmission, resetChatStore } from "./chatStore";

import { formatTurnElapsed, resetTurnClock, syncTurnClock, turnStartedAt } from "./runClock";

afterEach(() => {
  resetTurnClock();
  vi.useRealTimers();
});

describe("turn clock", () => {
  it("starts when work starts, holds while it continues, and clears when it ends", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000);
    expect(turnStartedAt()).toBeNull();

    syncTurnClock(true);
    expect(turnStartedAt()).toBe(1_000);

    vi.setSystemTime(9_000);
    syncTurnClock(true);
    expect(turnStartedAt()).toBe(1_000);

    syncTurnClock(false);
    expect(turnStartedAt()).toBeNull();
  });

  it("is cleared when the chat store is reset, so a new account never inherits a start", () => {
    resetChatStore();
    beginRunSubmission();
    expect(turnStartedAt()).not.toBeNull();

    resetChatStore();
    expect(turnStartedAt()).toBeNull();
  });

  it("keeps a run's time while another chat is open, and resumes it on return", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000);
    syncTurnClock(true, "run-a");
    expect(turnStartedAt()).toBe(1_000);

    // Chat B opens: the view has no run, which reads as "not working".
    vi.setSystemTime(40_000);
    syncTurnClock(false, null);
    expect(turnStartedAt()).toBeNull();

    // Back in chat A, whose run is still going.
    vi.setSystemTime(80_000);
    syncTurnClock(true, "run-a");
    expect(turnStartedAt()).toBe(1_000);
  });

  it("gives another chat's run its own start, never the one before it", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000);
    syncTurnClock(true, "run-a");
    syncTurnClock(false, null);

    vi.setSystemTime(50_000);
    syncTurnClock(true, "run-b");
    expect(turnStartedAt()).toBe(50_000);

    syncTurnClock(false, null);
    syncTurnClock(true, "run-a");
    expect(turnStartedAt()).toBe(1_000);
  });

  it("forgets a run once it is seen to end, so a new turn starts from zero", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000);
    syncTurnClock(true, "run-a");
    vi.setSystemTime(9_000);
    syncTurnClock(false, "run-a");

    vi.setSystemTime(20_000);
    syncTurnClock(true, "run-c");
    expect(turnStartedAt()).toBe(20_000);
  });

  it("counts the wait before a run has an id as part of the run", () => {
    vi.useFakeTimers();
    vi.setSystemTime(1_000);
    syncTurnClock(true, null);
    vi.setSystemTime(1_400);
    syncTurnClock(true, "run-a");
    expect(turnStartedAt()).toBe(1_000);
  });

  it("formats short times in tenths and long ones in minutes and hours", () => {
    expect(formatTurnElapsed(0)).toBe("0.0s");
    expect(formatTurnElapsed(8_049)).toBe("8.0s");
    expect(formatTurnElapsed(59_990)).toBe("59.9s");
    expect(formatTurnElapsed(128_000)).toBe("2m 8.0s");
    expect(formatTurnElapsed(3_723_000)).toBe("1h 2m 3s");
    expect(formatTurnElapsed(-5)).toBe("0.0s");
  });
});
