/**
 * Lowercased and stripped of diacritics, for matching what a person typed
 * against what they once wrote.
 *
 * Conversation titles are generated from the user's own opening message, so
 * they carry whatever was typed then — accents included — while the search that
 * has to find them again is typed in a hurry. `NFD` splits an accented
 * character into its base and its combining mark so the mark can be dropped;
 * without it "Café" and "cafe" are different strings all the way down.
 */
export function normalizeForSearch(value: string): string {
  return value
    .normalize("NFD")
    .replace(/\p{Diacritic}/gu, "")
    .toLocaleLowerCase();
}

/**
 * Whether `haystack` contains `needle`, ignoring case and diacritics.
 *
 * An empty query matches everything: a palette that has just opened should show
 * what it can do, not nothing.
 */
export function matchesSearch(haystack: string, needle: string): boolean {
  return scoreSearch(haystack, needle) !== null;
}

/**
 * Rank of `needle` against `haystack`, or `null` when it is not a match.
 *
 * Empty query: `0`, so a just-opened palette can list destinations without
 * inventing relevance. Typed query: every token must match (AND). A token
 * scores highest as an exact / prefix / substring hit; a one-edit typo or a
 * subsequence is accepted at a lower weight so "mrgin" still finds "margin"
 * without a "m" matching every word in the catalog.
 */
export function scoreSearch(haystack: string, needle: string): number | null {
  const query = normalizeForSearch(needle.trim());
  if (query.length === 0) return 0;

  const text = normalizeForSearch(haystack);
  const tokens = query.split(/\s+/).filter(Boolean);
  let total = 0;
  for (const token of tokens) {
    const part = scoreToken(text, token);
    if (part === null) return null;
    total += part;
  }
  return total;
}

function scoreToken(haystack: string, token: string): number | null {
  const index = haystack.indexOf(token);
  if (index >= 0) {
    let score = 120 - Math.min(index, 50);
    if (index === 0 || isWordBoundary(haystack, index)) score += 30;
    if (haystack === token) score += 40;
    return score;
  }

  const words = haystack.split(/[^a-z0-9]+/).filter(Boolean);
  for (const word of words) {
    if (word.startsWith(token)) return 90;
  }

  if (token.length >= 4) {
    for (const word of words) {
      if (isEditDistanceAtMostOne(word.slice(0, token.length + 1), token)) return 28;
    }
  }

  if (token.length >= 2) {
    const subsequence = subsequenceScore(haystack, token);
    if (subsequence !== null) return subsequence;
  }

  return null;
}

function isWordBoundary(haystack: string, index: number): boolean {
  if (index === 0) return true;
  return /[^a-z0-9]/i.test(haystack[index - 1] ?? "");
}

function subsequenceScore(haystack: string, token: string): number | null {
  let cursor = 0;
  let consecutive = 0;
  let score = 8;
  for (const character of token) {
    const found = haystack.indexOf(character, cursor);
    if (found < 0) return null;
    consecutive = found === cursor ? consecutive + 1 : 0;
    score += consecutive > 0 ? 3 : 1;
    cursor = found + 1;
  }
  return score;
}

function isEditDistanceAtMostOne(left: string, right: string): boolean {
  if (Math.abs(left.length - right.length) > 1) return false;
  if (left === right) return true;

  let i = 0;
  let j = 0;
  let edits = 0;
  while (i < left.length && j < right.length) {
    if (left[i] === right[j]) {
      i += 1;
      j += 1;
      continue;
    }
    edits += 1;
    if (edits > 1) return false;
    if (left.length > right.length) i += 1;
    else if (right.length > left.length) j += 1;
    else {
      i += 1;
      j += 1;
    }
  }
  return edits + (left.length - i) + (right.length - j) <= 1;
}
