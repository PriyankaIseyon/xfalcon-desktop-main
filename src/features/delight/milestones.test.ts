import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";
import { MILESTONE_NOTE_MS, milestoneNote, noteMessageSent } from "./milestones";

const sentMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({ delightMessageSent: sentMock }));

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
}

describe("message milestones", () => {
  beforeEach(() => {
    sentMock.mockReset();
    resetToastsForTests();
  });
  afterEach(resetToastsForTests);

  it("notes the 100th message and no message either side of it", () => {
    expect(milestoneNote(99)).toBeNull();
    expect(milestoneNote(100)).toContain("100 messages sent");
    expect(milestoneNote(101)).toBeNull();
    expect(milestoneNote(500)).toContain("500");
    expect(milestoneNote(1000)).toContain("1000");
  });

  it("shows the note for twenty seconds when the count reaches a milestone", async () => {
    sentMock.mockResolvedValue(100);
    noteMessageSent();
    await flush();
    expect(getToasts()).toHaveLength(1);
    expect(getToasts()[0]).toMatchObject({ tone: "message", duration: MILESTONE_NOTE_MS });
    expect(MILESTONE_NOTE_MS).toBe(20_000);
    expect(getToasts()[0].text).toContain("great xFalcon user");
  });

  it("shows nothing between milestones", async () => {
    sentMock.mockResolvedValue(57);
    noteMessageSent();
    await flush();
    expect(getToasts()).toHaveLength(0);
  });

  it("never disturbs the send it follows when the count cannot be kept", async () => {
    sentMock.mockRejectedValue({ code: "storage_unavailable" });
    expect(() => noteMessageSent()).not.toThrow();
    await flush();
    sentMock.mockImplementation(() => {
      throw new Error("no native shell");
    });
    expect(() => noteMessageSent()).not.toThrow();
    expect(getToasts()).toHaveLength(0);
  });
});
