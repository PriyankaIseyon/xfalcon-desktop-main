import type {
  ReportDatasetFreshness,
  ReportKind,
  ReportProjectOrigin,
  ReportRefreshOutcome,
  ReportView,
} from "../../lib/generated/contracts";

/**
 * How a report is named on the surface — and why a name is never how one is
 * identified.
 *
 * Nothing about a report is keyed by its title. Every operation crosses the
 * boundary carrying DA's report id and nothing else. Duplicate detection is
 * DA's, by the artifact the bytes belong to, never by name. A title is ordinary
 * DA data with no uniqueness constraint. Two reports may therefore carry exactly
 * the same name, legitimately — "save a copy" of an edited dashboard is the
 * ordinary way to get there — and the surface has to stay usable when they do.
 *
 * Duplicate visible text is harmless: two rows reading "Sales snapshot" are two
 * reports called "Sales snapshot". Duplicate control LABELS are not. "Delete
 * report" repeated down a list leaves a screen-reader user choosing blind, and
 * two same-named reports make it unrecoverable. So every report-scoped control
 * names its report AND the time it last changed — the one piece of visible
 * information that separates two rows sharing a title.
 */
export function reportHandle(report: ReportView): string {
  // The report's own title leads, verbatim and unquoted, because it is also the
  // visible text of the control that opens it: an accessible name that does not
  // contain the visible label is a WCAG 2.5.3 failure and breaks voice control.
  return `${report.title} — updated ${formatUpdated(report.updated_at_ms)}`;
}

/** Absolute, short, and local. A relative stamp ("2h ago", as xfalcon-web
 * renders) would need a ticking clock, and the frontend privilege boundary bans
 * `setInterval` (`src/security/frontend-boundary.test.ts`). */
export function formatUpdated(updatedAtMs: bigint): string {
  return new Date(Number(updatedAtMs)).toLocaleString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

/**
 * When a report changed, the way a card says it: "just now", "5 minutes ago",
 * "yesterday", "3 days ago", and the date itself past a week, when a count of
 * days stops being easier to read than the day. It is worked out when drawn and
 * does not tick, so a page left open can read "just now" for longer; the exact
 * time stays in each report's accessible name (`reportHandle`).
 */
export function formatRelativeUpdated(updatedAtMs: bigint, now: number = Date.now()): string {
  const then = Number(updatedAtMs);
  const seconds = Math.round((then - now) / 1000);
  const relative = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });
  const ago = -seconds;
  if (ago < 45) return "just now";
  if (ago < 3_600) return relative.format(Math.round(seconds / 60), "minute");
  if (ago < 86_400) return relative.format(Math.round(seconds / 3_600), "hour");
  if (ago < 7 * 86_400) return relative.format(Math.round(seconds / 86_400), "day");
  return new Date(then).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" });
}

/**
 * Whether some OTHER report already carries this name.
 *
 * Advisory, never a refusal: the store has no opinion about titles and this
 * function is not allowed to invent one. It exists so a reader who is about to
 * create two rows they cannot tell apart is told first. Compared case-folded
 * and trimmed, because "Sales " and "sales" are the same name to a reader even
 * though they are different strings to the store.
 */
export function isNameTaken(reports: ReportView[], title: string, exceptReportId: string | null): boolean {
  const needle = title.trim().toLocaleLowerCase();
  if (needle === "") return false;
  return reports.some((report) => report.id !== exceptReportId && report.title.trim().toLocaleLowerCase() === needle);
}

/**
 * Who shares a report, as one line for the surface. `null` for a report only
 * its owner can see, so no sharing line is shown at all. `global` reports are
 * shared across the organization, and the line names the owner when it is
 * someone else.
 */
export function reportSharing(report: ReportView): string | null {
  if (report.scope === "user") return null;
  if (report.is_owner) return "Shared with your organization";
  return `Shared by ${report.owner_display_name ?? "a teammate"}`;
}

/** What a report is, in the words its card and its viewer use. */
export function reportKindLabel(kind: ReportKind): string {
  switch (kind) {
    case "dashboard":
      return "Dashboard";
    case "workbook":
      return "Excel workbook";
    case "presentation":
      return "PowerPoint deck";
  }
}

/** The file format a report is stored as, for a card's size line. */
export function reportFormatLabel(kind: ReportKind): string {
  switch (kind) {
    case "dashboard":
      return "HTML";
    case "workbook":
      return "Excel";
    case "presentation":
      return "PowerPoint";
  }
}

/** A workbook or deck is one file, not a page: it has no preview to frame. */
export function isFileReport(report: Pick<ReportView, "kind">): boolean {
  return report.kind !== "dashboard";
}

/**
 * Who a report is visible to, as the one word a control states. The owner sees
 * the same word the sharing dialog offers; a reader who cannot manage it sees
 * it as a label only.
 */
export function reportAudience(report: Pick<ReportView, "scope">): "Private" | "Organization" {
  return report.scope === "global" ? "Organization" : "Private";
}

/** The reports a reader's list shows: everything they have not hidden. */
export function isListed(report: Pick<ReportView, "hidden">): boolean {
  return !report.hidden;
}

/**
 * One line on how old the opened page's data is, or `null` when the report has no
 * stored queries and so no data age to state.
 *
 * It never says the data is current: DA stamps the time it stored each dataset,
 * and a dataset that was never refreshed still shows the rows the page was saved
 * with. With several datasets the OLDEST time is the one stated, because a
 * report is only as fresh as its stalest dataset.
 */
export function freshnessSummary(
  report: Pick<ReportView, "refreshable">,
  datasets: ReportDatasetFreshness[],
): string | null {
  if (!report.refreshable) return null;
  const times = datasets.flatMap((dataset) =>
    dataset.executed_at_ms === null ? [] : [Number(dataset.executed_at_ms)],
  );
  if (times.length === 0) {
    return "Saved data has not been refreshed yet. The page shows the rows it was saved with.";
  }
  const oldest = formatUpdated(BigInt(Math.min(...times)));
  const missing = datasets.length - times.length;
  return missing === 0
    ? `Data stored ${oldest}.`
    : `Data stored ${oldest}. ${missing} dataset${missing === 1 ? " has" : "s have"} not been refreshed yet.`;
}

/** A refresh's duration for a line of text: milliseconds under a second, seconds above. */
export function formatDuration(durationMs: bigint | number): string {
  const ms = Number(durationMs);
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toLocaleString(undefined, { maximumFractionDigits: 1 })} s`;
}

/** The headline of a finished refresh: how many datasets ran and what they returned. */
export function refreshHeadline(outcomes: ReportRefreshOutcome[]): { ok: boolean; text: string } {
  const failed = outcomes.filter((outcome) => !outcome.ok);
  const total = outcomes.length;
  if (failed.length === 0) {
    const rows = outcomes.reduce((sum, outcome) => sum + Number(outcome.rows), 0);
    return {
      ok: true,
      text: `Refreshed ${total} ${total === 1 ? "dataset" : "datasets"} · ${rows.toLocaleString()} ${rows === 1 ? "row" : "rows"}`,
    };
  }
  if (failed.length === total) {
    return { ok: false, text: `Refresh failed for ${total === 1 ? "the dataset" : `all ${total} datasets`}` };
  }
  return { ok: false, text: `${total - failed.length} of ${total} datasets refreshed · ${failed.length} failed` };
}

/**
 * The library's Project filter. `all` shows everything, `none` the reports made
 * outside a project chat (and those saved before the link existed), and a
 * project id the reports made in that project.
 *
 * One string, so it travels through a `Select` as an ordinary value: the two
 * words can never be a project id, which is always a UUID.
 */
export type ProjectFilter = "all" | "none" | (string & {});

/** The project a report was created in, by the list's own origin rows. */
export function originOf(reportId: string, origins: readonly ReportProjectOrigin[]): ReportProjectOrigin | null {
  return origins.find((origin) => origin.report_id === reportId) ?? null;
}

export function matchesProject(
  report: Pick<ReportView, "id">,
  filter: ProjectFilter,
  origins: readonly ReportProjectOrigin[],
): boolean {
  if (filter === "all") return true;
  const origin = originOf(report.id, origins);
  return filter === "none" ? origin === null : origin?.project_id === filter;
}

/** A project the filter offers, and how many listed reports it holds. */
export type ProjectChoice = { id: string; name: string; count: number };

/**
 * The projects the filter offers: those with a report in `listed`, named by the
 * project's current name, A to Z. Counted over what the reader can see now
 * (hidden reports only when they have asked for them), so a number in the menu
 * is the number of cards it leads to before any other filter.
 *
 * The project the reader has chosen stays on offer with a zero count when its
 * last listed report has gone (hidden, or deleted), so the view they are in is
 * still named and not silently swapped for another. A chosen project no origin
 * names any more (it was removed) is not offered, and the caller falls back to
 * all reports.
 */
export function projectChoices(
  listed: readonly Pick<ReportView, "id">[],
  origins: readonly ReportProjectOrigin[],
  selected: ProjectFilter = "all",
): ProjectChoice[] {
  const shown = new Set(listed.map((report) => report.id));
  const byId = new Map<string, ProjectChoice>();
  for (const origin of origins) {
    const existing = byId.get(origin.project_id) ?? { id: origin.project_id, name: origin.project_name, count: 0 };
    if (shown.has(origin.report_id)) existing.count += 1;
    byId.set(origin.project_id, existing);
  }
  return [...byId.values()]
    .filter((choice) => choice.count > 0 || choice.id === selected)
    .sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
}
