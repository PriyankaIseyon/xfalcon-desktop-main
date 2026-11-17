export type CanvasCell = string | number | boolean | null;

/**
 * Serializing a grid to CSV (spec: dynamic-canvas-artifacts).
 *
 * Pure, and separate from the grid, because the correctness that matters here
 * is not visual: these rows are model output on their way into a spreadsheet,
 * which is a program.
 */

/**
 * Characters that make a spreadsheet treat a cell as a formula rather than
 * text.
 *
 * `docs/threat-model.md` TM-009 names formula-like content as a way
 * model output reaches a user's machine, and quoting does NOT stop it: Excel
 * and Sheets both evaluate `"=1+1"` after stripping the quotes. `+`/`-` and `@`
 * begin formulas too, and a leading tab or carriage return can slip a payload
 * past a naive check on the first character.
 */
const FORMULA_LEAD = /^[=+\-@\t\r]/;

/**
 * Neutralizes one cell for a spreadsheet.
 *
 * A leading `'` is the documented way to force a cell to text in Excel, Sheets,
 * and LibreOffice, and it survives a round trip: the value the reader sees is
 * the value the model produced. Deliberately not stripping the character —
 * deleting content to make it safe would silently change the data, and a
 * negative number is the common case here (`-4.2` leads with `-`).
 */
export function neutralizeSpreadsheetFormula(text: string): string {
  return FORMULA_LEAD.test(text) ? `'${text}` : text;
}

function csvField(cell: CanvasCell): string {
  // A quoted empty field, not a bare one: the promise below is that EVERY field
  // is quoted, and a reader scanning the file for an unquoted value should find
  // none.
  if (cell === null || cell === undefined) return '""';
  const text = neutralizeSpreadsheetFormula(String(cell));
  // Every field is quoted, so a comma, a quote, or a newline inside a value
  // cannot end it. Doubling is CSV's own escape for a literal quote.
  return `"${text.replace(/"/g, '""')}"`;
}

/**
 * RFC 4180 CSV for `columns` and `rows`.
 *
 * CRLF line endings, because that is what the format specifies and what
 * Excel on Windows — the overwhelmingly likely destination for a file exported
 * from this app — reads without complaint.
 */
export function toCsv(columns: readonly string[], rows: readonly CanvasCell[][]): string {
  const header = columns.map(csvField).join(",");
  const body = rows.map((row) => row.map(csvField).join(","));
  return [header, ...body].join("\r\n");
}
