import type { ChatConversationSummary } from "../../lib/generated/contracts";

export type RecencyBucket = "today" | "yesterday" | "thisWeek" | "older";

export const RECENCY_SECTIONS = [
  { key: "today", label: "Today" },
  { key: "yesterday", label: "Yesterday" },
  { key: "thisWeek", label: "This week" },
  { key: "older", label: "Older" },
] as const satisfies ReadonlyArray<{ key: RecencyBucket; label: string }>;

const DAY_MS = 86_400_000;

function startOfLocalDay(atMs: number): number {
  const date = new Date(atMs);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/** Which recency band an unpinned chat belongs in, from the reader's local day. */
export function recencyBucket(iso: string, now = Date.now()): RecencyBucket {
  const timestamp = Date.parse(iso);
  if (Number.isNaN(timestamp)) return "older";
  const today = startOfLocalDay(now);
  if (timestamp >= today) return "today";
  if (timestamp >= today - DAY_MS) return "yesterday";
  if (timestamp >= today - 7 * DAY_MS) return "thisWeek";
  return "older";
}

function byUpdatedAtDesc(left: ChatConversationSummary, right: ChatConversationSummary): number {
  return right.updated_at.localeCompare(left.updated_at);
}

/**
 * Pinned first (most recently pinned on top), then unpinned chats by recency.
 *
 * Shared rather than duplicated: the recents list and the project tree both
 * show the same conversations, and two orderings for one set of rows would
 * read as two different truths about which chat is on top.
 */
export function groupConversations(conversations: readonly ChatConversationSummary[], now = Date.now()) {
  const pinned = conversations
    .filter((conversation) => conversation.pinned_at)
    .sort((a, b) => (b.pinned_at ?? "").localeCompare(a.pinned_at ?? ""));
  const recents = conversations.filter((conversation) => !conversation.pinned_at).sort(byUpdatedAtDesc);
  const today: ChatConversationSummary[] = [];
  const yesterday: ChatConversationSummary[] = [];
  const thisWeek: ChatConversationSummary[] = [];
  const older: ChatConversationSummary[] = [];
  for (const conversation of recents) {
    switch (recencyBucket(conversation.updated_at, now)) {
      case "today":
        today.push(conversation);
        break;
      case "yesterday":
        yesterday.push(conversation);
        break;
      case "thisWeek":
        thisWeek.push(conversation);
        break;
      case "older":
        older.push(conversation);
        break;
    }
  }
  return {
    pinned,
    recents,
    today,
    yesterday,
    thisWeek,
    older,
    ordered: [...pinned, ...recents],
  };
}
