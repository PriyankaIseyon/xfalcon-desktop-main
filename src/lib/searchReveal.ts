import { createContext, useContext, useEffect, useState } from "react";
import { normalizeForSearch } from "./searchText";

/**
 * The non-component half of search reveal: the two contexts, the hooks that
 * read them, and the pure match-splitting the marks are drawn from.
 *
 * Split out of `highlightedText.tsx` rather than living beside the components
 * because a module that exports both components and non-components defeats
 * Fast Refresh for the whole file (`react-refresh/only-export-components`) —
 * editing the provider would blow away the state of every consumer while a
 * search is open, which is exactly when it is being worked on.
 *
 * The contexts are exported for `highlightedText.tsx`'s providers. Nothing
 * else should read them directly; use the hooks.
 */

export type SearchReveal = {
  query: string;
  targetId: string;
  token?: number;
};

/** Accent ring and match marks stay this long, then drop. */
export const SEARCH_REVEAL_MS = 5000;

export const RevealContext = createContext<SearchReveal | null>(null);
export const HighlightScopeContext = createContext(false);

export function useSearchReveal(): SearchReveal | null {
  return useContext(RevealContext);
}

export function useIsSearchRevealTarget(id: string | undefined): boolean {
  const reveal = useContext(RevealContext);
  const matched = Boolean(reveal && id && reveal.targetId === id);
  const [visible, setVisible] = useState(matched);

  useEffect(() => {
    if (!matched) {
      setVisible(false);
      return;
    }
    setVisible(true);
    const timer = window.setTimeout(() => setVisible(false), SEARCH_REVEAL_MS);
    return () => window.clearTimeout(timer);
  }, [matched, reveal?.targetId, reveal?.query, reveal?.token]);

  return visible;
}

export function splitHighlighted(text: string, query: string): { value: string; hit: boolean }[] {
  const tokens = query
    .trim()
    .split(/\s+/)
    .filter((token) => token.length >= 2)
    .sort((left, right) => right.length - left.length);
  if (tokens.length === 0) return [{ value: text, hit: false }];

  const haystack = normalizeForSearch(text);
  const ranges: { start: number; end: number }[] = [];
  for (const token of tokens) {
    const needle = normalizeForSearch(token);
    if (!needle) continue;
    let from = 0;
    while (from <= haystack.length - needle.length) {
      const at = haystack.indexOf(needle, from);
      if (at < 0) break;
      ranges.push({ start: at, end: at + needle.length });
      from = at + needle.length;
    }
  }
  if (ranges.length === 0) return [{ value: text, hit: false }];

  ranges.sort((left, right) => left.start - right.start);
  const merged: { start: number; end: number }[] = [];
  for (const range of ranges) {
    const last = merged[merged.length - 1];
    if (last && range.start <= last.end) {
      last.end = Math.max(last.end, range.end);
    } else {
      merged.push({ ...range });
    }
  }

  const parts: { value: string; hit: boolean }[] = [];
  const indexMap = originalIndexMap(text);
  let cursor = 0;
  for (const range of merged) {
    const start = indexMap[range.start] ?? text.length;
    const end = indexMap[range.end] ?? text.length;
    if (start > cursor) parts.push({ value: text.slice(cursor, start), hit: false });
    if (end > start) parts.push({ value: text.slice(start, end), hit: true });
    cursor = end;
  }
  if (cursor < text.length) parts.push({ value: text.slice(cursor), hit: false });
  return parts;
}

function originalIndexMap(text: string): number[] {
  const map: number[] = [];
  let normalizedAt = 0;
  for (let index = 0; index < text.length; index += 1) {
    const piece = normalizeForSearch(text[index] ?? "");
    if (piece.length === 0) continue;
    for (let offset = 0; offset < piece.length; offset += 1) {
      map[normalizedAt] = index;
      normalizedAt += 1;
    }
  }
  map[normalizedAt] = text.length;
  return map;
}
