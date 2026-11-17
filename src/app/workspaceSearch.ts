import { scoreSearch } from "../lib/searchText";

/** How many rows one group may offer. A longer list than the sidebar is the
 * failure this surface exists to avoid. */
export const MAX_RESULTS_PER_GROUP = 12;

/** Rendered in this order, so the group a reader is most likely to want leads. */
export const PALETTE_GROUPS = ["Chats", "Projects", "Reports", "Settings", "Commands"] as const;

export type PaletteGroup = (typeof PALETTE_GROUPS)[number];

export interface WorkspaceSearchRecord {
  id: string;
  group: PaletteGroup;
  /** Primary label shown in the row and matched first. */
  label: string;
  /** Extra words that should match but are not the title (paths, keywords). */
  keywords?: string;
  /** Sidebar / store order among siblings, used when scores tie. */
  order: number;
  /** Pin boost for chats and reports that the source list already pinned. */
  pinned?: boolean;
}

export interface RankedSearchHit extends WorkspaceSearchRecord {
  score: number;
}

/**
 * Filter and bound a catalog. Chats keep the order the sidebar already
 * decided (pinned first). Every other group ranks by match quality, then
 * that source order.
 */
export function rankWorkspaceSearch(
  records: readonly WorkspaceSearchRecord[],
  query: string,
  extraMatchedIds: ReadonlySet<string> = new Set(),
): RankedSearchHit[] {
  const scored: RankedSearchHit[] = [];
  for (const record of records) {
    const haystack = record.keywords ? `${record.label}\n${record.keywords}` : record.label;
    const score = scoreSearch(haystack, query);
    if (score === null && !extraMatchedIds.has(record.id)) continue;
    scored.push({
      ...record,
      score: score === null ? 16 : record.pinned ? score + 24 : score,
    });
  }

  const taken: RankedSearchHit[] = [];
  for (const group of PALETTE_GROUPS) {
    const inGroup = scored.filter((hit) => hit.group === group);
    inGroup.sort((left, right) => {
      if (group === "Chats") return left.order - right.order;
      if (right.score !== left.score) return right.score - left.score;
      return left.order - right.order;
    });
    taken.push(...inGroup.slice(0, MAX_RESULTS_PER_GROUP));
  }
  return taken;
}
