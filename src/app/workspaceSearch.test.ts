import { describe, expect, it } from "vitest";
import { MAX_RESULTS_PER_GROUP, rankWorkspaceSearch, type WorkspaceSearchRecord } from "./workspaceSearch";

function record(
  partial: Pick<WorkspaceSearchRecord, "id" | "group" | "label"> & Partial<WorkspaceSearchRecord>,
): WorkspaceSearchRecord {
  return { order: 0, ...partial };
}

describe("rankWorkspaceSearch", () => {
  it("keeps pinned chats in the order the sidebar already decided", () => {
    const hits = rankWorkspaceSearch(
      [
        record({ id: "chat-a", group: "Chats", label: "Revenue by region", order: 1 }),
        record({ id: "chat-b", group: "Chats", label: "Revenue by product", order: 0, pinned: true }),
      ],
      "revenue",
    );

    expect(hits.map((hit) => hit.id)).toEqual(["chat-b", "chat-a"]);
  });

  it("caps each group so a broad query cannot out-scroll the sidebar", () => {
    const records = Array.from({ length: MAX_RESULTS_PER_GROUP + 8 }, (_, index) =>
      record({ id: `chat-${index}`, group: "Chats", label: `Revenue ${index}`, order: index }),
    );

    expect(rankWorkspaceSearch(records, "revenue")).toHaveLength(MAX_RESULTS_PER_GROUP);
  });

  it("includes a record whose body matched even when the title does not", () => {
    const hits = rankWorkspaceSearch(
      [record({ id: "chat-1", group: "Chats", label: "Untitled conversation" })],
      "gross margin",
      new Set(["chat-1"]),
    );

    expect(hits.map((hit) => hit.id)).toEqual(["chat-1"]);
  });

  it("ranks a settings heading above a weaker keyword-only hit", () => {
    const hits = rankWorkspaceSearch(
      [
        record({
          id: "settings-diagnostics",
          group: "Settings",
          label: "Diagnostics",
          keywords: "logs",
          order: 1,
        }),
        record({
          id: "settings-appearance",
          group: "Settings",
          label: "Appearance",
          keywords: "theme dark light",
          order: 0,
        }),
      ],
      "appearance",
    );

    expect(hits[0]?.id).toBe("settings-appearance");
  });
});
