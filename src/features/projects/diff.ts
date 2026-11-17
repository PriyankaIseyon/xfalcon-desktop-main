export type DiffLineKind = "hunk" | "context" | "add" | "delete" | "note";

export interface DiffRow {
  kind: DiffLineKind;
  marker: " " | "+" | "-" | "@" | "\\";
  oldLine: number | null;
  newLine: number | null;
  text: string;
}

export interface TextDiffSection {
  kind: "text";
  path: string | null;
  change: "created" | "deleted" | "changed";
  rows: DiffRow[];
}

export interface OpaqueDiffSection {
  kind: "opaque";
  content: string;
}

export type DiffSection = TextDiffSection | OpaqueDiffSection;

export type ParsedDiff =
  { kind: "sections"; sections: DiffSection[]; truncated: boolean } | { kind: "opaque"; source: string };

export type SplitDiffRow =
  { kind: "meta"; row: DiffRow } | { kind: "pair"; left: DiffRow | null; right: DiffRow | null };

const HUNK = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?:.*)?$/;
const TRUNCATION_MARKER = "... (diff truncated)";

interface ParsedTextSection {
  section: TextDiffSection;
  nextIndex: number;
  truncated: boolean;
}

function headerPairAt(lines: readonly string[], index: number): boolean {
  return lines[index]?.startsWith("--- ") === true && lines[index + 1]?.startsWith("+++ ") === true;
}

function headerPath(value: string): string | null {
  const path = value.split("\t", 1)[0];
  if (!path || path === "/dev/null") return null;
  return path.replace(/^[ab]\//, "");
}

function safeLineNumber(value: string): number | null {
  const parsed = Number(value);
  return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : null;
}

function opaqueSource(source: string): string {
  return source.trim().length > 0 ? source : "No displayable changes.";
}

/**
 * A `\ No newline at end of file` marker or the backend's own truncation
 * marker can appear both between hunks and inside one, so both loops in
 * `parseTextSection` consume it the same way instead of repeating the check.
 */
function consumeNoteLine(line: string): { row: DiffRow; truncated: boolean } | null {
  if (line === TRUNCATION_MARKER) {
    return { row: { kind: "note", marker: "\\", oldLine: null, newLine: null, text: line }, truncated: true };
  }
  if (line.startsWith("\\")) {
    return {
      row: { kind: "note", marker: "\\", oldLine: null, newLine: null, text: line.slice(1).trim() },
      truncated: false,
    };
  }
  return null;
}

function parseTextSection(lines: readonly string[], startIndex: number): ParsedTextSection | null {
  const oldPath = headerPath(lines[startIndex].slice(4));
  const newPath = headerPath(lines[startIndex + 1].slice(4));
  if (oldPath === null && newPath === null) return null;
  let index = startIndex + 2;
  let foundHunk = false;
  let truncated = false;
  const rows: DiffRow[] = [];

  while (index < lines.length) {
    if (headerPairAt(lines, index)) break;
    const line = lines[index];
    const note = consumeNoteLine(line);
    if (note) {
      rows.push(note.row);
      index += 1;
      if (note.truncated) {
        truncated = true;
        break;
      }
      continue;
    }

    const hunk = HUNK.exec(line);
    if (!hunk) break;
    const oldStart = safeLineNumber(hunk[1]);
    const oldCount = safeLineNumber(hunk[2] ?? "1");
    const newStart = safeLineNumber(hunk[3]);
    const newCount = safeLineNumber(hunk[4] ?? "1");
    if (oldStart === null || oldCount === null || newStart === null || newCount === null) return null;

    foundHunk = true;
    rows.push({ kind: "hunk", marker: "@", oldLine: null, newLine: null, text: line });
    index += 1;
    let oldLine = oldStart;
    let newLine = newStart;
    let oldRemaining = oldCount;
    let newRemaining = newCount;

    while (oldRemaining > 0 || newRemaining > 0) {
      const body = lines[index];
      if (body === undefined) return null;
      const note = consumeNoteLine(body);
      if (note) {
        rows.push(note.row);
        index += 1;
        if (note.truncated) {
          truncated = true;
          break;
        }
        continue;
      }
      if (body.startsWith(" ") && oldRemaining > 0 && newRemaining > 0) {
        rows.push({ kind: "context", marker: " ", oldLine, newLine, text: body.slice(1) });
        oldLine += 1;
        newLine += 1;
        oldRemaining -= 1;
        newRemaining -= 1;
        index += 1;
        continue;
      }
      if (body.startsWith("-") && oldRemaining > 0) {
        rows.push({ kind: "delete", marker: "-", oldLine, newLine: null, text: body.slice(1) });
        oldLine += 1;
        oldRemaining -= 1;
        index += 1;
        continue;
      }
      if (body.startsWith("+") && newRemaining > 0) {
        rows.push({ kind: "add", marker: "+", oldLine: null, newLine, text: body.slice(1) });
        newLine += 1;
        newRemaining -= 1;
        index += 1;
        continue;
      }
      return null;
    }

    if (truncated) break;
  }

  if (!foundHunk) return null;
  return {
    section: {
      kind: "text",
      path: newPath ?? oldPath,
      change: oldPath === null ? "created" : newPath === null ? "deleted" : "changed",
      rows,
    },
    nextIndex: index,
    truncated,
  };
}

/**
 * Parses the backend's bounded display artifact. It does not reconstruct a
 * patch. A malformed line diff falls back to raw source so review never
 * presents guessed line numbers or silently drops content -- but a failure
 * is scoped to the one file section it happened in. A multi-file plan puts
 * every file's `--- `/`+++ ` pair through its own `parseTextSection` call, so
 * one file with an unrecognized line (an unusual "\" note, a corrupt hunk
 * header) falls back to its own raw text instead of discarding every other
 * file's already-parsed diff in the same plan.
 */
export function parseUnifiedDiff(source: string): ParsedDiff {
  const normalized = source.replace(/\r\n/g, "\n");
  const lines = normalized.split("\n");
  if (lines.at(-1) === "") lines.pop();
  if (lines.length === 0) return { kind: "opaque", source: opaqueSource(source) };

  const sections: DiffSection[] = [];
  let truncated = false;
  let index = 0;
  while (index < lines.length) {
    if (headerPairAt(lines, index)) {
      const sectionStart = index;
      const parsed = parseTextSection(lines, sectionStart);
      if (parsed !== null) {
        sections.push(parsed.section);
        truncated ||= parsed.truncated;
        index = parsed.nextIndex;
        continue;
      }
      // This one file's section did not parse. Resync at the next file
      // boundary (never at `sectionStart` itself, which is the header pair
      // that just failed) and keep this section's own raw text, rather than
      // losing every other section already parsed in this plan.
      let scan = sectionStart + 1;
      while (scan < lines.length && !headerPairAt(lines, scan)) {
        if (lines[scan] === TRUNCATION_MARKER) truncated = true;
        scan += 1;
      }
      const content = lines.slice(sectionStart, scan).join("\n");
      if (content.trim().length > 0) sections.push({ kind: "opaque", content });
      index = scan;
      continue;
    }

    const start = index;
    while (index < lines.length && !headerPairAt(lines, index)) {
      if (lines[index] === TRUNCATION_MARKER) truncated = true;
      index += 1;
    }
    const content = lines.slice(start, index).join("\n");
    if (content.trim().length > 0) sections.push({ kind: "opaque", content });
  }

  return sections.length > 0
    ? { kind: "sections", sections, truncated }
    : { kind: "opaque", source: opaqueSource(source) };
}

/** Pairs adjacent delete/add blocks for a real two-column comparison. */
export function toSplitRows(rows: readonly DiffRow[]): SplitDiffRow[] {
  const split: SplitDiffRow[] = [];
  let index = 0;
  while (index < rows.length) {
    const row = rows[index];
    if (row.kind === "hunk" || row.kind === "note") {
      split.push({ kind: "meta", row });
      index += 1;
      continue;
    }
    if (row.kind === "context") {
      split.push({ kind: "pair", left: row, right: row });
      index += 1;
      continue;
    }

    const deleted: DiffRow[] = [];
    const added: DiffRow[] = [];
    const notes: DiffRow[] = [];
    // A "\ No newline at end of file" note (common for any file that does
    // not end in a trailing newline) can land between the delete and add
    // half of one logical replacement. Keep gathering through it instead of
    // ending the run early, or the pairing below splits a single
    // replacement into two lopsided rows with an empty opposite column.
    while (
      index < rows.length &&
      (rows[index].kind === "delete" || rows[index].kind === "add" || rows[index].kind === "note")
    ) {
      const change = rows[index];
      if (change.kind === "delete") deleted.push(change);
      else if (change.kind === "add") added.push(change);
      else notes.push(change);
      index += 1;
    }
    const pairCount = Math.max(deleted.length, added.length);
    for (let pairIndex = 0; pairIndex < pairCount; pairIndex += 1) {
      split.push({ kind: "pair", left: deleted[pairIndex] ?? null, right: added[pairIndex] ?? null });
    }
    for (const note of notes) {
      split.push({ kind: "meta", row: note });
    }
  }
  return split;
}
