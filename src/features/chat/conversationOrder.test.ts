import { describe, expect, it } from "vitest";
import type { ChatConversationSummary } from "../../lib/generated/contracts";
import { groupConversations, recencyBucket } from "./conversationOrder";

function conversation(id: string, updatedAt: string, pinnedAt?: string): ChatConversationSummary {
  return {
    version: 1,
    conversation_id: id,
    title: id,
    updated_at: updatedAt,
    pinned_at: pinnedAt,
  };
}

function localIso(year: number, month: number, day: number, hour = 15): string {
  return new Date(year, month - 1, day, hour).toISOString();
}

const NOW = new Date(2026, 7, 21, 16).getTime();

describe("recencyBucket", () => {
  it("places chats on the reader's local calendar, not a rolling 24 hours", () => {
    expect(recencyBucket(localIso(2026, 8, 21, 1), NOW)).toBe("today");
    expect(recencyBucket(localIso(2026, 8, 20, 23), NOW)).toBe("yesterday");
    expect(recencyBucket(localIso(2026, 8, 16, 12), NOW)).toBe("thisWeek");
    expect(recencyBucket(localIso(2026, 8, 1, 12), NOW)).toBe("older");
  });

  it("treats an unreadable timestamp as older rather than inventing a band", () => {
    expect(recencyBucket("not-a-date", NOW)).toBe("older");
  });
});

describe("groupConversations", () => {
  it("keeps pinned chats out of the recency bands and newest-pinned first", () => {
    const groups = groupConversations(
      [
        conversation("recent", localIso(2026, 8, 21), "2026-08-01T00:00:00Z"),
        conversation("older-pin", localIso(2026, 8, 1), "2026-08-10T00:00:00Z"),
        conversation("today", localIso(2026, 8, 21)),
      ],
      NOW,
    );

    expect(groups.pinned.map((row) => row.conversation_id)).toEqual(["older-pin", "recent"]);
    expect(groups.today.map((row) => row.conversation_id)).toEqual(["today"]);
    expect(groups.recents.map((row) => row.conversation_id)).toEqual(["today"]);
  });

  it("fills Today, Yesterday, This week, and Older from unpinned chats", () => {
    const groups = groupConversations(
      [
        conversation("old", localIso(2026, 8, 1)),
        conversation("week", localIso(2026, 8, 16)),
        conversation("yday", localIso(2026, 8, 20)),
        conversation("now", localIso(2026, 8, 21)),
      ],
      NOW,
    );

    expect(groups.today.map((row) => row.conversation_id)).toEqual(["now"]);
    expect(groups.yesterday.map((row) => row.conversation_id)).toEqual(["yday"]);
    expect(groups.thisWeek.map((row) => row.conversation_id)).toEqual(["week"]);
    expect(groups.older.map((row) => row.conversation_id)).toEqual(["old"]);
    expect(groups.ordered.map((row) => row.conversation_id)).toEqual(["now", "yday", "week", "old"]);
  });
});
