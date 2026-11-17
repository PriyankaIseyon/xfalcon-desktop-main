import { useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowDown, ArrowUp, ArrowUpDown, Copy, Download, Search } from "lucide-react";
import { Button } from "../../ui/Button";
import { TruncatedText } from "../../ui/TruncatedText";
import { documentExportSave } from "../../lib/native";
import { formatNumber } from "../../lib/formatNumber";
import { readNumber } from "../../lib/readNumber";
import { observeScrollRect } from "../../lib/virtualScrollRect";
import { toast } from "../../ui/toastStore";
import { writeClipboardText } from "../chat/clipboard";
import { toCsv } from "./tabularExport";
import type { TableArtifact } from "./canvasStore";

export interface DataGridProps {
  artifact: TableArtifact;
}

type SortDirection = "asc" | "desc";
type Sort = { column: number; direction: SortDirection } | null;

const ROW_HEIGHT_PX = 34;
/** Rows kept mounted above and below the window, so a fast scroll does not
 * reveal blank space before the next measure. */
const OVERSCAN_ROWS = 12;
/** Row-number gutter, and the minimum a data column may shrink to. Shared by
 * the header and the body — the two are laid out by ONE grid template, because
 * a header measured independently of its rows is a header that does not line up
 * with them. */
const GUTTER_WIDTH = "3rem";
const MIN_COLUMN_WIDTH = "7rem";

/**
 * A sortable, filterable grid over one table artifact
 * (spec: dynamic-canvas-artifacts).
 *
 * Virtualized because a model can produce a table with tens of thousands of
 * rows and the panel must stay responsive; the scroll container is the grid's
 * own element, and the sticky header lives inside it so the columns cannot
 * drift out of step with the body.
 */
export function DataGrid({ artifact }: DataGridProps) {
  const [filterText, setFilterText] = useState("");
  const [sort, setSort] = useState<Sort>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  // Memoized, not `?? []` inline: a fresh empty array on every render would
  // change the sort memo's dependencies every render, re-sorting the whole table
  // for nothing.
  const formats = useMemo(() => artifact.numberFormats ?? [], [artifact.numberFormats]);

  /** Ascending, then descending, then back to the table's own order. Three
   * states rather than two, so a reader can always get the original back. */
  const cycleSort = (column: number) => {
    setSort((previous) => {
      if (previous === null || previous.column !== column) return { column, direction: "asc" };
      if (previous.direction === "asc") return { column, direction: "desc" };
      return null;
    });
  };

  const rows = useMemo(() => {
    let next = artifact.rows;

    const query = filterText.trim().toLowerCase();
    if (query !== "") {
      next = next.filter((row) => row.some((cell) => cell !== null && String(cell).toLowerCase().includes(query)));
    }

    // A sort column past this table's width cannot happen — the component is
    // keyed by artifact id, so a new table gets a fresh instance — but the
    // guard costs nothing and the alternative is comparing `undefined`.
    if (sort !== null && sort.column < artifact.columns.length) {
      const { column } = sort;
      const factor = sort.direction === "asc" ? 1 : -1;
      const format = formats[column] ?? null;
      // Decorate, sort, undecorate: each cell is read once per sort. A comparator
      // that parsed its cells would parse them n log n times.
      const keyed = next.map((row) => {
        const cell = row[column];
        const missing = cell === null || cell === undefined || cell === "";
        return {
          row,
          missing,
          number: format === null || missing ? null : readNumber(cell, format),
          text: missing ? "" : String(cell),
        };
      });
      keyed.sort((left, right) => {
        // Blanks sink in both directions: a column sorted descending should
        // begin with its largest value, not with the rows that have none.
        if (left.missing || right.missing) return left.missing && right.missing ? 0 : left.missing ? 1 : -1;
        if (left.number !== null && right.number !== null) return (left.number - right.number) * factor;
        return left.text.localeCompare(right.text, undefined, { numeric: true }) * factor;
      });
      next = keyed.map((entry) => entry.row);
    }

    return next;
  }, [artifact.rows, artifact.columns.length, filterText, formats, sort]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT_PX,
    overscan: OVERSCAN_ROWS,
    // The same rect observer the transcript uses. A virtualizer whose scroll
    // element measures zero renders NO items, which is the state this panel is
    // in on its first paint before the grid track has been sized -- the grid
    // would come up empty and only fill in on the next resize.
    observeElementRect: observeScrollRect,
  });

  // One template for the header and every row. `minmax` rather than `1fr`
  // alone so a wide table scrolls horizontally instead of crushing its columns
  // into unreadable slivers.
  const gridTemplate = `${GUTTER_WIDTH} repeat(${artifact.columns.length}, minmax(${MIN_COLUMN_WIDTH}, 1fr))`;

  const csv = () => toCsv(artifact.columns, rows);

  const copyCsv = async () => {
    try {
      await writeClipboardText(csv());
      toast.success("Table copied as CSV.");
    } catch {
      toast.error("The table could not be copied.");
    }
  };

  /**
   * Saves through the native dialog, never an `<a download>` blob.
   *
   * A blob anchor writes with no destination step, which is the download
   * affordance `src-tauri/src/security/tests.rs` records as deliberately
   * refused. The person names the file; the backend never takes a path.
   */
  const saveCsv = async () => {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    try {
      const result = await documentExportSave(artifact.title, csv(), "csv");
      // A dismissed dialog is an answer, not a failure — say nothing.
      if (result.saved) toast.success(`Saved ${result.file_name ?? "the file"}.`);
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : "The table could not be saved.");
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  const rowCount = `${formatNumber(rows.length)} row${rows.length === 1 ? "" : "s"}`;
  const columnCount = `${formatNumber(artifact.columns.length)} column${artifact.columns.length === 1 ? "" : "s"}`;

  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-card text-foreground">
      <div className="canvas-toolbar">
        <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2">
          <div className="relative flex min-w-0 flex-1 basis-48 items-center">
            <Search aria-hidden className="absolute left-2.5 text-muted" size={14} strokeWidth={1.8} />
            <input
              aria-label="Filter rows"
              className="canvas-input min-w-0 w-full pl-8 pr-3"
              placeholder="Filter data…"
              type="text"
              value={filterText}
              onChange={(event) => setFilterText(event.currentTarget.value)}
            />
          </div>
          <span aria-live="polite" className="canvas-meta-badge">
            {rowCount} · {columnCount}
          </span>
        </div>

        <div className="flex flex-wrap items-center gap-1.5">
          <Button aria-label="Copy as CSV" onClick={() => void copyCsv()} size="sm" variant="secondary">
            <Copy aria-hidden size={14} strokeWidth={1.8} />
            Copy
          </Button>
          <Button
            aria-label="Save file as CSV"
            loading={busy}
            onClick={() => void saveCsv()}
            size="sm"
            variant="secondary"
          >
            <Download aria-hidden size={14} strokeWidth={1.8} />
            Save file
          </Button>
        </div>
      </div>

      <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto">
        {/* `role="table"` and the row/cell roles below: a grid built from divs
            for one shared column template is still a table to a reader, and
            without them it is an unlabelled stack of text. */}
        <div className="min-w-max" role="table" aria-rowcount={rows.length + 1}>
          <div
            className="sticky top-0 z-10 grid border-b border-hairline bg-surface font-semibold text-secondary shadow-e1"
            role="row"
            style={{ gridTemplateColumns: gridTemplate }}
          >
            <span
              aria-label="Row number"
              className="border-r border-hairline/50 px-2 py-2 text-center text-xs text-muted"
              role="columnheader"
            >
              #
            </span>
            {artifact.columns.map((column, index) => {
              const sorted = sort?.column === index ? sort.direction : null;
              return (
                <button
                  // Index, not the name: a query can return two columns called
                  // the same thing, and a duplicate key would make React
                  // reuse one header for both.
                  key={index}
                  aria-label={`Sort by ${column === "" ? `column ${index + 1}` : column}`}
                  aria-sort={sorted === "asc" ? "ascending" : sorted === "desc" ? "descending" : "none"}
                  className="flex cursor-pointer select-none items-center justify-between gap-1 border-r border-hairline/50 px-3 py-2 text-left text-xs transition-colors hover:bg-hover hover:text-foreground"
                  role="columnheader"
                  type="button"
                  onClick={() => cycleSort(index)}
                >
                  <span className="truncate">{column === "" ? `Column ${index + 1}` : column}</span>
                  {sorted === "asc" ? (
                    <ArrowUp aria-hidden className="shrink-0 text-accent" size={14} strokeWidth={2.2} />
                  ) : sorted === "desc" ? (
                    <ArrowDown aria-hidden className="shrink-0 text-accent" size={14} strokeWidth={2.2} />
                  ) : (
                    <ArrowUpDown aria-hidden className="shrink-0 text-faint opacity-50" size={14} strokeWidth={1.8} />
                  )}
                </button>
              );
            })}
          </div>

          {rows.length === 0 ? (
            <div role="row">
              <p
                aria-colspan={artifact.columns.length + 1}
                className="px-4 py-12 text-center text-xs text-muted"
                role="cell"
              >
                {filterText.trim() === "" ? "This table has no rows." : "No rows match that filter."}
              </p>
            </div>
          ) : (
            <div className="relative" style={{ height: `${virtualizer.getTotalSize()}px` }}>
              {virtualizer.getVirtualItems().map((virtualRow) => {
                const row = rows[virtualRow.index];
                if (row === undefined) return null;
                return (
                  <div
                    key={virtualRow.key}
                    aria-rowindex={virtualRow.index + 2}
                    className="absolute left-0 top-0 grid w-full border-b border-hairline/40 transition-colors hover:bg-hover/60"
                    role="row"
                    style={{
                      // Fixed height, not measured: every cell truncates to one
                      // line, so measuring each row would cost a layout pass per
                      // scroll frame to learn a number already known.
                      gridTemplateColumns: gridTemplate,
                      height: `${virtualRow.size}px`,
                      transform: `translateY(${virtualRow.start}px)`,
                    }}
                  >
                    <span
                      className="border-r border-hairline/40 px-2 py-1.5 text-center text-xs tabular-nums text-faint"
                      role="cell"
                    >
                      {formatNumber(virtualRow.index + 1)}
                    </span>
                    {artifact.columns.map((_, columnIndex) => {
                      const cell = row[columnIndex];
                      const isMeasure = formats[columnIndex] != null;
                      return (
                        <TruncatedText
                          key={columnIndex}
                          className={`border-r border-hairline/40 px-3 py-1.5 text-xs text-foreground ${
                            isMeasure ? "text-right tabular-nums" : ""
                          }`}
                          role="cell"
                          text={cell === null || cell === undefined ? undefined : String(cell)}
                        >
                          {cell === null || cell === undefined || cell === "" ? (
                            <span aria-label="empty" className="italic text-faint">
                              —
                            </span>
                          ) : (
                            String(cell)
                          )}
                        </TruncatedText>
                      );
                    })}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
