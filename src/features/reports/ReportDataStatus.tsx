import { useId, useState } from "react";
import { CircleAlert, CircleCheck, ChevronDown, Database } from "lucide-react";
import type { ReportDatasetFreshness, ReportRefreshOutcome } from "../../lib/generated/contracts";
import { cn } from "../../lib/cn";
import { formatDuration, formatUpdated, refreshHeadline } from "./reportPresentation";

/** What the last refresh did, or that one is running. */
export type DataRefreshState =
  { phase: "idle" } | { phase: "refreshing" } | { phase: "done"; outcomes: ReportRefreshOutcome[] };

/** One dataset as the table shows it: what DA stored, and what the last refresh did. */
type DatasetRow = {
  id: string;
  rows: string;
  stored: string;
  result: { ok: boolean; text: string } | null;
};

function rowsFor(datasets: ReportDatasetFreshness[], outcomes: ReportRefreshOutcome[]): DatasetRow[] {
  const byId = new Map(outcomes.map((outcome) => [outcome.dataset_id, outcome]));
  const known = datasets.map<DatasetRow>((dataset) => {
    const outcome = byId.get(dataset.dataset_id);
    byId.delete(dataset.dataset_id);
    return {
      id: dataset.dataset_id,
      rows: dataset.row_count === null ? "—" : Number(dataset.row_count).toLocaleString(),
      stored: dataset.executed_at_ms === null ? "Not refreshed yet" : formatUpdated(BigInt(dataset.executed_at_ms)),
      result: outcome === undefined ? null : resultOf(outcome),
    };
  });
  // A dataset the refresh reported that the page does not carry is still told.
  const extra = [...byId.values()].map<DatasetRow>((outcome) => ({
    id: outcome.dataset_id,
    rows: outcome.ok ? Number(outcome.rows).toLocaleString() : "—",
    stored: "—",
    result: resultOf(outcome),
  }));
  return [...known, ...extra];
}

function resultOf(outcome: ReportRefreshOutcome): { ok: boolean; text: string } {
  if (!outcome.ok) return { ok: false, text: outcome.error ?? "Refresh failed" };
  const source = outcome.data_source === null ? "" : ` from ${outcome.data_source}`;
  return { ok: true, text: `Refreshed in ${formatDuration(outcome.duration_ms)}${source}` };
}

/**
 * Where a report's data came from, in one quiet strip under the viewer's header.
 *
 * One line states the data's age (or, after a refresh, what the refresh did),
 * and a disclosure opens a per-dataset table. The strip keeps one height in
 * every state, so a refresh never moves the report beneath it: while one runs a
 * rule slides along the strip's lower edge instead of any text changing size.
 * A refresh that failed in any part opens the table by itself, because a reader
 * must not have to look for what went wrong. Nothing here says the data is
 * current: the age is the oldest time DA stored, and a failed dataset keeps its
 * last good rows.
 */
export function ReportDataStatus({
  summary,
  datasets,
  refresh,
}: {
  /** The data-age sentence, already worded (`freshnessSummary`). */
  summary: string;
  datasets: ReportDatasetFreshness[];
  refresh: DataRefreshState;
}) {
  const panelId = useId();
  const [openChoice, setOpenChoice] = useState<boolean | null>(null);
  const outcomes = refresh.phase === "done" ? refresh.outcomes : [];
  const headline = refresh.phase === "done" ? refreshHeadline(outcomes) : null;
  const failed = headline !== null && !headline.ok;
  // The reader's own toggle wins; until they use it a failure shows itself.
  const open = openChoice ?? failed;
  const rows = rowsFor(datasets, outcomes);
  const refreshing = refresh.phase === "refreshing";
  const tone = refreshing ? "busy" : failed ? "failed" : headline !== null ? "done" : "idle";
  const Icon = failed ? CircleAlert : headline !== null ? CircleCheck : Database;

  return (
    <section aria-label="Report data" className="report-data-status" data-open={open} data-tone={tone}>
      <div className="report-data-status__bar">
        <Icon aria-hidden className="report-data-status__icon" size={14} />
        <p className="report-data-status__text">
          {refreshing ? "Refreshing data…" : headline !== null ? headline.text : summary}
        </p>
        {rows.length > 0 ? (
          <button
            aria-controls={panelId}
            aria-expanded={open}
            className="report-data-status__toggle press"
            type="button"
            onClick={() => setOpenChoice(!open)}
          >
            Details
            <ChevronDown aria-hidden className="report-data-status__chevron" size={14} />
          </button>
        ) : null}
        <span aria-hidden className="report-data-status__rule" />
      </div>
      {/* A grid row that grows from 0 to its content: the only height transition
          that needs no measured pixel value. `inert` takes the closed table out of
          the tab order and the accessibility tree. */}
      <div className="report-data-status__panel" id={panelId} inert={!open}>
        <div className="report-data-status__panel-inner">
          {headline !== null ? <p className="report-data-status__summary">{summary}</p> : null}
          <table className="report-data-status__table">
            <thead>
              <tr>
                <th scope="col">Dataset</th>
                <th className="num" scope="col">
                  Rows
                </th>
                <th scope="col">Stored</th>
                {outcomes.length > 0 ? <th scope="col">Last refresh</th> : null}
              </tr>
            </thead>
            <tbody>
              {rows.map((row) => (
                <tr key={row.id}>
                  <th scope="row">{row.id}</th>
                  <td className="num">{row.rows}</td>
                  <td>{row.stored}</td>
                  {outcomes.length > 0 ? (
                    <td className={cn(row.result !== null && !row.result.ok && "is-failed")}>
                      {row.result === null ? "—" : row.result.text}
                    </td>
                  ) : null}
                </tr>
              ))}
            </tbody>
          </table>
          {failed ? (
            <p className="report-data-status__note" role="alert">
              A dataset that failed keeps its last stored rows. The report still shows them.
            </p>
          ) : null}
        </div>
      </div>
    </section>
  );
}
