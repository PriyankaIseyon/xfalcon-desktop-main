import { ColumnTypeBadge } from "./ColumnTypeBadge";
import { AlertTriangle } from "lucide-react";
import type { DatasetPreview } from "../../lib/generated/contracts";
import { TruncatedText } from "../../ui/TruncatedText";

/**
 * The rows a person checks before uploading.
 *
 * Cells arrive as the file wrote them — the backend does not reformat a number,
 * because which decimal separator a file uses is not a guess to make on the one
 * screen where someone confirms what their data says.
 *
 * An absent cell renders as an em dash rather than as an empty cell, so a gap in
 * the data is visible as a gap. That distinction decides whether a
 * `remove_nulls` step would drop the row.
 *
 * Row numbers sit in a frozen gutter, and the header stays put while the rows
 * scroll under it. A person picking a header row is counting rows, and a person
 * checking a wide export is reading across it — both need a fixed edge to count
 * and read from. This mirrors xfalcon-web's preview, so one file is confirmed
 * the same way in both products.
 */
export function DatasetPreviewTable({ preview }: { preview: DatasetPreview }) {
  if (preview.columns.length === 0) {
    return (
      <PreviewBanner tone="danger">
        The chosen header row names no columns. Pick a different row above, or a different worksheet.
      </PreviewBanner>
    );
  }

  return (
    <div className="max-h-80 overflow-auto rounded-md border border-hairline">
      <table className="w-full border-separate text-xs" style={{ borderSpacing: 0 }}>
        <thead>
          <tr>
            <th
              className="type-micro sticky left-0 top-0 z-20 w-11 border-b border-r border-hairline bg-surface px-2 py-1.5 text-right font-medium text-muted"
              scope="col"
            >
              #
            </th>
            {preview.columns.map((column, index) => (
              <th
                key={`${column.name}-${index}`}
                className="sticky top-0 z-10 whitespace-nowrap border-b border-hairline bg-surface px-3 py-1.5 text-left align-bottom font-medium text-secondary"
                scope="col"
              >
                <span className="block">{column.name}</span>
                <span className="mt-1 block font-normal">
                  <ColumnTypeBadge type={column.inferred_type} />
                </span>
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {preview.rows.length === 0 ? (
            <tr>
              <td className="px-3 py-6 text-center text-muted" colSpan={preview.columns.length + 1}>
                No rows below the header row.
              </td>
            </tr>
          ) : (
            preview.rows.map((row, rowIndex) => (
              <tr key={rowIndex} className="transition-colors hover:bg-hover">
                <td className="type-micro sticky left-0 z-10 w-11 border-b border-r border-hairline bg-card px-2 py-1 text-right tabular-nums text-muted">
                  {rowIndex + 1}
                </td>
                {preview.columns.map((_, columnIndex) => {
                  const cell = row[columnIndex] ?? null;
                  return (
                    <td
                      key={columnIndex}
                      className="max-w-64 border-b border-hairline px-3 py-1 font-mono text-secondary"
                    >
                      {cell === null ? (
                        <span className="text-muted">—</span>
                      ) : (
                        <TruncatedText as="span" className="block max-w-64" text={cell}>
                          {cell}
                        </TruncatedText>
                      )}
                    </td>
                  );
                })}
              </tr>
            ))
          )}
        </tbody>
      </table>
    </div>
  );
}

/**
 * What the preview is, in one line: how wide, how much of the file, and which
 * worksheet or delimiter it came from.
 *
 * Above the table rather than below it, because it frames what follows.
 */
export function PreviewMeta({ preview }: { preview: DatasetPreview }) {
  const columns = preview.columns.length;
  const rows = preview.rows.length;
  const parts = [
    `${columns} ${columns === 1 ? "column" : "columns"}`,
    preview.truncated_rows ? `first ${rows} rows` : `${rows} ${rows === 1 ? "row" : "rows"}`,
  ];
  if (preview.truncated_columns) parts.push("some columns not shown");
  if (preview.truncated_cell) parts.push("a long value shortened");
  return <p className="type-caption text-muted">{parts.join(" · ")}</p>;
}

/** What the preview had to leave out, said once rather than per cell. */
export function PreviewTruncationNotice({ preview }: { preview: DatasetPreview }) {
  if (!preview.truncated_rows && !preview.truncated_columns && !preview.truncated_cell) return null;
  return <p className="type-caption text-muted">The preview is shortened. The whole file uploads.</p>;
}

function PreviewBanner({ tone, children }: { tone: "danger" | "neutral"; children: React.ReactNode }) {
  return (
    <div
      className={
        tone === "danger"
          ? "flex items-start gap-2 rounded-md border border-danger bg-danger-bg px-3 py-2 text-danger"
          : "flex items-start gap-2 rounded-md border border-hairline bg-surface px-3 py-2 text-secondary"
      }
      role="status"
    >
      <AlertTriangle aria-hidden className="mt-0.5 shrink-0" size={14} />
      <span className="type-caption leading-relaxed">{children}</span>
    </div>
  );
}
