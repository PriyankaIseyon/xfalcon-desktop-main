import React from "react";
import { ChartNoAxesColumn, ChevronFirst, ChevronLeft, ChevronRight } from "lucide-react";
import type { ProjectTableRows } from "../../lib/generated/contracts";
import { projectTableRows } from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { IconButton } from "../../ui/IconButton";
import { Spinner } from "../../ui/Spinner";
import { Tooltip } from "../../ui/Tooltip";
import { describeProjectError } from "./projectFileText";
import { chartSpecFor, numberFormatsFor, PROJECT_TABLE_CHART_LIMIT_MESSAGE } from "../chat/richContent/tableChart";
import { openCanvasArtifact } from "../canvas/canvasStore";

/** Rows one page shows. The backend returns at most 1,000 in one window. */
const LARGE_TABLE_PAGE_ROWS = 500;

type TableState =
  | { status: "loading"; start: number }
  | { status: "ready"; rows: ProjectTableRows }
  | { status: "error"; start: number; message: string };

/**
 * A delimited file too large to parse in the webview, a page of rows at a time
 * from the backend's own parser (spec: project-file-editing, "Large tables page
 * from the backend"). Cells are text nodes, as in the chat's table.
 */
export function LargeTableView({
  projectId,
  path,
  delimiter,
}: {
  projectId: string;
  path: string;
  delimiter: "," | "\t";
}) {
  const [state, setState] = React.useState<TableState>({ status: "loading", start: 0 });
  const [jump, setJump] = React.useState("");
  const request = React.useRef(0);

  const load = React.useCallback(
    (start: number) => {
      const current = ++request.current;
      setState({ status: "loading", start });
      projectTableRows(projectId, path, delimiter, start, LARGE_TABLE_PAGE_ROWS).then(
        (rows) => {
          if (current !== request.current) return;
          if (rows.path !== path || rows.start_row !== start) {
            setState({ status: "error", start, message: "The rows returned were not the rows asked for." });
            return;
          }
          setState({ status: "ready", rows });
        },
        (cause: unknown) => {
          if (current === request.current) {
            setState({ status: "error", start, message: describeProjectError(cause, "These rows could not be read.") });
          }
        },
      );
    },
    [delimiter, path, projectId],
  );

  React.useEffect(() => {
    load(0);
    return () => {
      request.current += 1;
    };
  }, [load]);

  const rows = state.status === "ready" ? state.rows : null;
  const start = rows?.start_row ?? (state.status === "ready" ? 0 : state.start);
  const total = rows?.total_rows ?? null;
  const shownEnd = rows ? start + rows.rows.length : start;
  const chart = React.useMemo(() => {
    if (rows === null || rows.truncated) return null;
    const table = { headers: rows.headers, rows: rows.rows };
    return chartSpecFor(table, numberFormatsFor(table), true, rows.start_row);
  }, [rows]);

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="type-caption text-muted" role="status">
          {rows
            ? rows.rows.length === 0
              ? "No rows here."
              : `Rows ${(start + 1).toLocaleString()}–${shownEnd.toLocaleString()} of ${
                  total === null ? `${shownEnd.toLocaleString()}+` : total.toLocaleString()
                }`
            : `Reading from row ${(start + 1).toLocaleString()}…`}
        </span>
        <div className="flex flex-wrap items-center justify-end gap-1">
          {rows ? (
            <Tooltip
              label={
                rows.truncated
                  ? "Truncated rows cannot be charted."
                  : rows.rows.length === 0
                    ? "No rows on this page to chart."
                    : chart === null
                      ? PROJECT_TABLE_CHART_LIMIT_MESSAGE
                      : "Chart only the rows on this page."
              }
            >
              <Button
                className="shrink-0 whitespace-nowrap"
                disabled={chart === null}
                onClick={() => {
                  if (chart === null) return;
                  openCanvasArtifact({
                    id: `chart:project:${projectId}:${path}:rows:${start}`,
                    type: "chart",
                    title: `${path.split("/").pop() ?? path} — rows ${(start + 1).toLocaleString()}–${shownEnd.toLocaleString()}`,
                    chartType: "bar",
                    ...chart,
                  });
                }}
                size="sm"
                variant="secondary"
              >
                <ChartNoAxesColumn aria-hidden size={14} strokeWidth={1.8} />
                Chart page in panel
              </Button>
            </Tooltip>
          ) : null}
          <Tooltip label="First page">
            <IconButton aria-label="First page" disabled={start === 0} onClick={() => load(0)} size="sm">
              <ChevronFirst aria-hidden size={14} />
            </IconButton>
          </Tooltip>
          <Tooltip label="Previous page">
            <IconButton
              aria-label="Previous page"
              disabled={start === 0}
              onClick={() => load(Math.max(0, start - LARGE_TABLE_PAGE_ROWS))}
              size="sm"
            >
              <ChevronLeft aria-hidden size={14} />
            </IconButton>
          </Tooltip>
          <Tooltip label="Next page">
            <IconButton
              aria-label="Next page"
              disabled={rows?.next_row == null}
              onClick={() => rows?.next_row != null && load(rows.next_row)}
              size="sm"
            >
              <ChevronRight aria-hidden size={14} />
            </IconButton>
          </Tooltip>
          <form
            className="flex items-center gap-1"
            onSubmit={(event) => {
              event.preventDefault();
              const row = Number.parseInt(jump, 10);
              if (Number.isFinite(row) && row >= 1) load(row - 1);
            }}
          >
            <input
              aria-label="Go to row"
              className="type-caption w-24 rounded-md border border-default bg-card px-2 py-1 text-foreground"
              inputMode="numeric"
              onChange={(event) => setJump(event.currentTarget.value)}
              placeholder="Row"
              value={jump}
            />
            <Button size="sm" type="submit" variant="secondary">
              Go
            </Button>
          </form>
        </div>
      </div>
      {rows?.truncated ? (
        <p className="type-caption text-muted">Some cells or columns are too long to show whole and are cut here.</p>
      ) : null}
      {rows && !rows.truncated && rows.rows.length > 0 && rows.headers.length > 0 && chart === null ? (
        <p className="type-caption text-muted">{PROJECT_TABLE_CHART_LIMIT_MESSAGE}</p>
      ) : null}
      {state.status === "error" ? (
        <Alert title="These rows were not read" tone="danger">
          <p>{state.message}</p>
          <Button className="mt-2" onClick={() => load(state.start)} size="sm" variant="secondary">
            Try again
          </Button>
        </Alert>
      ) : null}
      {rows ? (
        <div
          aria-label={`${path} rows`}
          className="rich-card__scroll min-h-0 flex-1 surface-card"
          role="region"
          tabIndex={0}
        >
          <table className="rich-table">
            <thead>
              <tr>
                <th scope="col">#</th>
                {rows.headers.map((header, index) => (
                  <th key={`${index}-${header}`} scope="col">
                    {header}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {rows.rows.map((row, rowIndex) => (
                <tr key={start + rowIndex}>
                  <td className="text-faint tabular-nums">{(start + rowIndex + 1).toLocaleString()}</td>
                  {row.map((cell, cellIndex) => (
                    <td key={cellIndex}>{cell}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : state.status === "loading" ? (
        <p className="type-body flex items-center gap-2 text-secondary">
          <Spinner decorative size={16} />
          Reading rows…
        </p>
      ) : null}
    </div>
  );
}
