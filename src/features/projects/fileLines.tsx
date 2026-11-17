import type { ReactNode } from "react";

/** A line past this many characters is drawn cut, with how much more it has.
 * The same rule editors use: a minified megabyte on one line would otherwise
 * be one DOM text node the browser has to lay out on every scroll. */
export const DISPLAY_LINE_CHARS = 10_000;

/** Columns one tab takes. Mirrors `tab-size` on `.project-file-lines__row` in
 * `index.css`, so the measured width and the drawn width agree. */
export const TAB_COLUMNS = 4;

/** Lines as the backend numbers them: a trailing newline ends the last line
 * rather than starting an empty one. A CRLF file's `\r` is the line ending's,
 * not the line's, so it is not part of what is drawn or measured. */
export function splitLines(text: string): string[] {
  const lines = text.split("\n");
  if (lines.length > 1 && lines[lines.length - 1] === "") lines.pop();
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index]!;
    if (line.endsWith("\r")) lines[index] = line.slice(0, -1);
  }
  return lines;
}

/** A plain line for display: cut past [`DISPLAY_LINE_CHARS`], saying how much
 * was left out. */
export function displayLine(line: string): ReactNode {
  if (line.length <= DISPLAY_LINE_CHARS) return line;
  return (
    <>
      {line.slice(0, DISPLAY_LINE_CHARS)}
      <span className="text-faint"> … {(line.length - DISPLAY_LINE_CHARS).toLocaleString()} more characters</span>
    </>
  );
}

/** The columns `text` takes on one row: its length, with each tab counted at
 * its full width. An upper bound — a tab after text stops short of it — which
 * is the safe side for a scroll width. */
function textColumns(text: string): number {
  let tabs = 0;
  for (let at = text.indexOf("\t"); at !== -1; at = text.indexOf("\t", at + 1)) tabs += 1;
  return text.length + tabs * (TAB_COLUMNS - 1);
}

/** Room the "… N more characters" note takes after a cut line. */
const CUT_NOTE_COLUMNS = 32;

/** The columns [`displayLine`] draws `line` in: all of it, or its cut and the
 * note after it. */
export function lineColumns(line: string): number {
  if (line.length <= DISPLAY_LINE_CHARS) return textColumns(line);
  return textColumns(line.slice(0, DISPLAY_LINE_CHARS)) + CUT_NOTE_COLUMNS;
}

/** The widest drawn line's columns, for [`VirtualLinesProps.maxLineColumns`]. */
export function longestLine(lines: readonly string[]): number {
  let longest = 0;
  for (const line of lines) {
    // Tabs only ever widen a line, so one that cannot win even as all tabs is
    // not scanned.
    if (Math.min(line.length, DISPLAY_LINE_CHARS) * TAB_COLUMNS + CUT_NOTE_COLUMNS <= longest) continue;
    const columns = lineColumns(line);
    if (columns > longest) longest = columns;
  }
  return longest;
}
