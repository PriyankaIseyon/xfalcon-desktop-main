import { describe, expect, it } from "vitest";
import { formatConversationAge, formatConversationCalendarDate } from "./formatRelativeTime";

describe("formatConversationAge", () => {
  const now = Date.parse("2026-08-20T12:00:00.000Z");

  it("returns Now for very recent updates", () => {
    expect(formatConversationAge("2026-08-20T11:59:30.000Z", now)).toBe("Now");
  });

  it("returns minutes, hours, and days compactly", () => {
    expect(formatConversationAge("2026-08-20T11:00:00.000Z", now)).toBe("1h");
    expect(formatConversationAge("2026-08-19T12:00:00.000Z", now)).toBe("1d");
  });

  it("returns a calendar date beyond a week", () => {
    expect(formatConversationAge("2026-01-05T12:00:00.000Z", now)).toBe(
      formatConversationCalendarDate("2026-01-05T12:00:00.000Z"),
    );
  });
});

describe("formatConversationCalendarDate", () => {
  it("returns a numeric local date, not a relative age", () => {
    expect(formatConversationCalendarDate("2026-08-12T12:00:00.000Z")).toMatch(/\d/);
    expect(formatConversationCalendarDate("not-a-date")).toBe("");
  });
});
