import type { ChartArtifact } from "../../canvas/canvasStore";
import { readNumber, resolveNumberFormats, type NumberFormat } from "../../../lib/readNumber";
import type { TableData } from "./detect";

/** CSV byte limits do not bound SVG elements: even a small file can contain
 * hundreds of thousands of categories or numeric values. Refuse oversized
 * project charts rather than silently dropping rows or series. */
export const PROJECT_TABLE_CHART_MAX_VALUES = 5_000;
export const PROJECT_TABLE_CHART_LIMIT_MESSAGE = "This table has too many values to chart (maximum 5,000).";

function isIdentifierColumn(header: string): boolean {
  const name = header.trim().toLowerCase();
  return name === "id" || name === "#" || name.endsWith("_id");
}

/**
 * How each column reads, or `null` for a label column. Resolved over the WHOLE
 * table, so an ambiguous cell (`1,234`) takes the convention the rest of the
 * table settles, and the card and the chart read every cell the same way.
 * Identifier columns are excluded before the grammar sees them: never measures.
 */
export function numberFormatsFor(table: TableData): (NumberFormat | null)[] {
  const columns = table.headers.map((_, index) => table.rows.map((row) => row[index]));
  return resolveNumberFormats(
    columns,
    table.headers.map((header) => isIdentifierColumn(header)),
  );
}

/** Project delimited files also support category counts and a row-number axis.
 * Chat tables retain their existing label-plus-measure rule. */
export function chartSpecFor(
  table: TableData,
  formats: (NumberFormat | null)[],
  allowRowFallback = false,
  firstRow = 0,
): Pick<ChartArtifact, "data" | "xKey" | "yKeys" | "seriesLabels"> | null {
  if (table.rows.length === 0 || table.headers.length === 0) return null;
  const labelColumns = table.headers.map((_, index) => index).filter((index) => !formats[index]);
  const valueColumns: { index: number; format: NumberFormat }[] = [];
  table.headers.forEach((header, index) => {
    const format = formats[index];
    if (format && !isIdentifierColumn(header)) valueColumns.push({ index, format });
  });

  // Synthetic keys keep duplicate headers and a column named "x" independent.
  const xKey = "x";
  if (valueColumns.length > 0 && (labelColumns.length > 0 || allowRowFallback)) {
    if (allowRowFallback && table.rows.length * valueColumns.length > PROJECT_TABLE_CHART_MAX_VALUES) return null;
    const labelIndex = labelColumns[0];
    const yKeys = valueColumns.map((_, position) => `y${position}`);
    const seriesLabels: Record<string, string> = {};
    valueColumns.forEach(({ index }, position) => {
      seriesLabels[yKeys[position]!] = table.headers[index] ?? `Series ${position + 1}`;
    });
    const data = table.rows.map((row, rowIndex) => {
      const point: Record<string, string | number | null> = {
        [xKey]: labelIndex === undefined ? `Row ${firstRow + rowIndex + 1}` : (row[labelIndex] ?? ""),
      };
      // Read once, here at the boundary: a blank or refused cell is a gap, not a string.
      valueColumns.forEach(({ index, format }, position) => {
        point[yKeys[position]!] = readNumber(row[index] ?? "", format);
      });
      return point;
    });
    return { data, xKey, yKeys, seriesLabels };
  }
  if (!allowRowFallback) return null;

  // Prefer the first category over an ID. Map keeps untrusted cell values such
  // as "__proto__" as plain labels, and counts every row, including blanks.
  const labelIndex = labelColumns.find((index) => !isIdentifierColumn(table.headers[index] ?? "")) ?? 0;
  const counts = new Map<string, number>();
  for (const row of table.rows) {
    const label = row[labelIndex] ?? "";
    counts.set(label, (counts.get(label) ?? 0) + 1);
    if (counts.size > PROJECT_TABLE_CHART_MAX_VALUES) return null;
  }
  return {
    data: [...counts].map(([x, y0]) => ({ x, y0 })),
    xKey,
    yKeys: ["y0"],
    seriesLabels: { y0: `Row count by ${table.headers[labelIndex] ?? "category"}` },
  };
}
