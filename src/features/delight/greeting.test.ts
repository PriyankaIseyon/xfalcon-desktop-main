import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { claimTodaysGreeting, localDay, pickGreeting } from "./greeting";

const claimMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({ delightGreetingClaim: claimMock }));

describe("greeting lines", () => {
  const morning = new Date(2026, 9, 1, 8, 0);

  it("names the day by the local calendar, zero-padded", () => {
    expect(localDay(new Date(2026, 0, 5, 23, 59))).toBe("2026-01-05");
    expect(localDay(new Date(2026, 9, 11, 0, 0))).toBe("2026-10-11");
  });

  it("offers five different lines", () => {
    const lines = [0, 0.2, 0.4, 0.6, 0.8].map((pick) => pickGreeting(pick, morning, "Jeevan"));
    expect(new Set(lines).size).toBe(5);
    expect(lines[0]).toBe("Good morning, Jeevan");
  });

  it("follows the hour in the first line", () => {
    expect(pickGreeting(0, new Date(2026, 9, 1, 11, 59), null)).toBe("Good morning");
    expect(pickGreeting(0, new Date(2026, 9, 1, 12, 0), null)).toBe("Good afternoon");
    expect(pickGreeting(0, new Date(2026, 9, 1, 16, 59), null)).toBe("Good afternoon");
    expect(pickGreeting(0, new Date(2026, 9, 1, 17, 0), null)).toBe("Good evening");
  });

  it("uses only the first word of the name, and none when there is none", () => {
    expect(pickGreeting(0.4, morning, "  Jeevan T M ")).toBe("Welcome back, Jeevan.");
    expect(pickGreeting(0.4, morning, "   ")).toBe("Welcome back.");
    expect(pickGreeting(0.4, morning, undefined)).toBe("Welcome back.");
  });

  it("stays inside the list at the ends of the range", () => {
    expect(pickGreeting(0, morning, null)).toBe("Good morning");
    expect(pickGreeting(0.999999, morning, null)).toBe("Let's find some answers today.");
    expect(pickGreeting(1, morning, null)).toBe("Let's find some answers today.");
  });
});

describe("claiming the day's greeting", () => {
  beforeEach(() => {
    claimMock.mockReset();
  });
  afterEach(() => vi.restoreAllMocks());

  it("claims the local day and reports the answer", async () => {
    claimMock.mockResolvedValue(true);
    await expect(claimTodaysGreeting(new Date(2026, 9, 1, 9, 0))).resolves.toBe(true);
    expect(claimMock).toHaveBeenCalledWith("2026-10-01");
  });

  it("gives callers that ask together one claim and one answer", async () => {
    claimMock.mockResolvedValue(true);
    const [first, second] = await Promise.all([claimTodaysGreeting(), claimTodaysGreeting()]);
    expect([first, second]).toEqual([true, true]);
    expect(claimMock).toHaveBeenCalledTimes(1);
  });

  it("asks again once the first claim is settled", async () => {
    claimMock.mockResolvedValueOnce(true).mockResolvedValueOnce(false);
    expect(await claimTodaysGreeting()).toBe(true);
    expect(await claimTodaysGreeting()).toBe(false);
    expect(claimMock).toHaveBeenCalledTimes(2);
  });

  it("is a no when the claim cannot be recorded", async () => {
    claimMock.mockRejectedValue({ code: "storage_unavailable" });
    await expect(claimTodaysGreeting()).resolves.toBe(false);
    claimMock.mockImplementation(() => {
      throw new Error("no native shell");
    });
    await expect(claimTodaysGreeting()).resolves.toBe(false);
  });
});
