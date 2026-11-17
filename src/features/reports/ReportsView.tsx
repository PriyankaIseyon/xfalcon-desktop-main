import React, { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import {
  ChartNoAxesColumn,
  Search,
  LayoutGrid,
  List,
  Pin,
  Library,
  ArrowUpDown,
  EyeOff,
  Folder,
  FolderOpen,
  FolderX,
} from "lucide-react";
import { cn } from "../../lib/cn";
import type { ReportView } from "../../lib/generated/contracts";
import { AlertDialog } from "../../ui/AlertDialog";
import { Button } from "../../ui/Button";
import { EmptyState } from "../../ui/EmptyState";
import { Eyebrow } from "../../ui/Heading";
import { SearchField } from "../../ui/SearchField";
import { IconButton } from "../../ui/IconButton";
import { Select } from "../../ui/Select";
import { SegmentedControl } from "../../ui/SegmentedControl";
import { Switch } from "../../ui/Switch";
import "./reports.css";
import "../../ui/catalog.css";
import { Skeleton } from "../../ui/Skeleton";
import type { ReportActionHandlers } from "./ReportActions";
import { ReportCard } from "./ReportCard";
import { ReportNameDialog } from "./ReportNameDialog";
import { ReportPreviewBackfill } from "./ReportPreviewBackfill";
import { ReportTable } from "./ReportTable";
import { ReportShareDialog } from "./ReportShareDialog";
import { ReportViewer } from "./ReportViewer";
import { clearPendingReport, getPendingReportId, subscribePendingReport } from "./reportsStore";
import {
  isListed,
  isNameTaken,
  matchesProject,
  originOf,
  projectChoices,
  type ProjectFilter,
} from "./reportPresentation";
import { useReports } from "./useReports";

const RECENT_CARD_COUNT = 4;

/** Which reports the reader asks for: everything, what they made, what others
 * shared with them, or what they pinned. The first three are web's own; a pin is
 * the reader's. */
type ReportFilter = "all" | "mine" | "shared" | "pinned";

export function ReportsView({ active, revealQuery = "" }: { active: boolean; revealQuery?: string }) {
  const [filter, setFilter] = useState<ReportFilter>("all");
  // Which project's reports to show: all, only those made outside a project chat,
  // or one project's.
  const [projectFilter, setProjectFilter] = useState<ProjectFilter>("all");
  // A hidden report leaves the reader's list but is never lost: this brings it back
  // into view, with the control to show it again.
  const [showHidden, setShowHidden] = useState(false);
  const [shareTarget, setShareTarget] = useState<ReportView | null>(null);
  const [shareError, setShareError] = useState<string | null>(null);
  const [layout, setLayout] = useState<"grid" | "list">("grid");
  const [sort, setSort] = useState<"updated" | "created" | "name">("updated");
  const reports = useReports(active);
  const [query, setQuery] = useState("");
  const [openReportId, setOpenReportId] = useState<string | null>(null);
  const [renameTarget, setRenameTarget] = useState<ReportView | null>(null);
  const [renameError, setRenameError] = useState<string | null>(null);
  const [pendingDelete, setPendingDelete] = useState<ReportView | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const workspace = useRef<HTMLElement>(null);
  // Set when the reader leaves a report, so focus lands back in the library once
  // it is on screen again: the Back button went with the viewer.
  const restoreListFocus = useRef(false);
  const { status, reports: all, projectOrigins } = reports.snapshot;
  const viewing = all.find((report) => report.id === openReportId) ?? null;

  useEffect(() => {
    if (viewing !== null || !restoreListFocus.current) return;
    restoreListFocus.current = false;
    workspace.current?.focus({ preventScroll: true });
  });

  // The command palette asked for a specific report. The shell has already
  // switched destination on the same intent; this consumes it once the list
  // carrying that row has landed.
  const pendingReportId = useSyncExternalStore(subscribePendingReport, getPendingReportId, getPendingReportId);
  useEffect(() => {
    if (pendingReportId === null) return;
    if (!all.some((report) => report.id === pendingReportId)) {
      // A read that has ended, whether it landed without the report or failed,
      // ends the request: left alone, it would open that report unasked whenever
      // a later read brought it back.
      if (status !== "loading") clearPendingReport();
      return;
    }
    setOpenReportId(pendingReportId);
    clearPendingReport();
  }, [all, pendingReportId, status]);

  const hiddenCount = useMemo(() => all.filter((report) => !isListed(report)).length, [all]);
  const listed = useMemo(() => (showHidden ? all : all.filter(isListed)), [all, showHidden]);

  const choices = useMemo(
    () => projectChoices(listed, projectOrigins, projectFilter),
    [listed, projectOrigins, projectFilter],
  );
  // A chosen project that no origin names any more (it was removed) is the same
  // as no project filter, and the control says so by showing "All reports".
  const activeProject: ProjectFilter =
    projectFilter === "all" || projectFilter === "none" || choices.some((choice) => choice.id === projectFilter)
      ? projectFilter
      : "all";
  // A chosen project that has gone is dropped for good, so it cannot quietly apply
  // again if a project of that id comes back.
  useEffect(() => {
    if (status === "ready" && activeProject !== projectFilter) setProjectFilter(activeProject);
  }, [status, activeProject, projectFilter]);
  const activeProjectName = choices.find((choice) => choice.id === activeProject)?.name ?? null;
  // The control appears once any report is linked to a project, and stays while it
  // is in use. A library with no project reports has nothing to filter by.
  const showProjectFilter = projectOrigins.length > 0 || activeProject !== "all";

  const visible = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    const selected = listed.filter(
      (report) =>
        matchesFilter(report, filter) &&
        matchesProject(report, activeProject, projectOrigins) &&
        (needle === "" ||
          report.title.toLocaleLowerCase().includes(needle) ||
          (report.description ?? "").toLocaleLowerCase().includes(needle)),
    );
    if (sort === "updated") return selected;
    return selected.sort((a, b) =>
      sort === "name"
        ? a.title.localeCompare(b.title) || a.id.localeCompare(b.id)
        : Number(b.created_at_ms) - Number(a.created_at_ms) || a.id.localeCompare(b.id),
    );
  }, [listed, query, filter, activeProject, projectOrigins, sort]);

  const zones = useMemo(() => {
    if (sort !== "updated" || filter !== "all" || activeProject !== "all") {
      return { pinned: [], recent: visible, older: [] };
    }
    const pinned = visible.filter((report) => report.pinned);
    const rest = visible.filter((report) => !report.pinned);
    return { pinned, recent: rest.slice(0, RECENT_CARD_COUNT), older: rest.slice(RECENT_CARD_COUNT) };
  }, [visible, filter, activeProject, sort]);

  // Every action is keyed by the report's id, never by its title, and the card
  // grid and the table get the identical set — see `reportPresentation.ts`.
  const open = (report: ReportView) => setOpenReportId(report.id);
  const closeViewer = () => {
    restoreListFocus.current = true;
    setOpenReportId(null);
  };
  const actions: ReportActionHandlers = {
    onTogglePin: (report) => {
      if (reports.busyById[report.id] !== undefined) return;
      setActionError(null);
      const pinning = !report.pinned;
      void reports.setPinned(report, pinning).then((result) => {
        if (result === "failed")
          setActionError(
            pinning ? "This report could not be pinned. Try again." : "This report could not be unpinned. Try again.",
          );
        // A pinned report moves to another zone, so its card is rebuilt and the
        // button that was focused goes with the old one: put focus on its twin.
        if (result === "succeeded") {
          requestAnimationFrame(() =>
            workspace.current?.querySelector<HTMLElement>(`[data-pin-for="${CSS.escape(report.id)}"]`)?.focus(),
          );
        }
      });
    },
    onToggleHidden: (report) => {
      if (reports.busyById[report.id] !== undefined) return;
      setActionError(null);
      const hiding = !report.hidden;
      void reports.setHidden(report, hiding).then((result) => {
        if (result === "failed")
          setActionError(
            hiding ? "This report could not be hidden. Try again." : "This report could not be shown. Try again.",
          );
        // A hidden report leaves the list, and the button that was focused goes
        // with its card: keep focus in the library. A shown one stays where it is.
        if (result === "succeeded") {
          requestAnimationFrame(() => {
            const twin = workspace.current?.querySelector<HTMLElement>(`[data-hide-for="${CSS.escape(report.id)}"]`);
            (twin ?? workspace.current)?.focus({ preventScroll: true });
          });
        }
      });
    },
    onShare: (report) => {
      if (reports.busyById[report.id] !== undefined) return;
      setActionError(null);
      setShareError(null);
      setShareTarget(report);
    },
    onRename: (report) => {
      setActionError(null);
      setRenameError(null);
      setRenameTarget(report);
    },
    onDelete: (report) => {
      setActionError(null);
      setDeleteError(null);
      setPendingDelete(report);
    },
    // The outcome is said once, by the toast `regeneratePreview` raises.
    onRegeneratePreview: (report) => {
      if (reports.busyById[report.id] !== undefined) return;
      setActionError(null);
      void reports.regeneratePreview(report);
    },
  };

  const shareBusy = shareTarget !== null && reports.busyById[shareTarget.id] === "share";
  const renameBusy = renameTarget !== null && reports.busyById[renameTarget.id] === "rename";
  const deleteBusy = pendingDelete !== null && reports.busyById[pendingDelete.id] === "delete";

  // The open viewer shows the CURRENT row, so a share it just changed reads as it
  // now is; the dialog is about the row as it was when it was asked for.
  const shareDialog = (
    <ReportShareDialog
      busy={shareBusy}
      error={shareError}
      report={shareTarget}
      onCancel={() => {
        if (shareBusy) return;
        setShareError(null);
        setShareTarget(null);
      }}
      onConfirm={(target, scope) => {
        setShareError(null);
        void reports.setScope(target, scope).then((result) => {
          if (result === "succeeded") setShareTarget(null);
          if (result === "failed") setShareError("Sharing was not changed. Try again.");
          if (result === "forbidden") setShareError("Only the owner can change who can open this report.");
        });
      }}
    />
  );

  // A report whose row is gone (deleted, or dropped by a refresh) resolves to
  // null and simply falls through to the list — better than an empty frame, and
  // no state write during render to arrange it.
  if (viewing !== null) {
    return (
      <>
        <ReportViewer
          key={viewing.id}
          report={viewing}
          projectName={originOf(viewing.id, projectOrigins)?.project_name ?? null}
          revealQuery={revealQuery}
          onBack={closeViewer}
          onShare={actions.onShare}
        />
        {shareDialog}
      </>
    );
  }

  return (
    <section
      ref={workspace}
      aria-label="Saved reports"
      className="reports-workspace flex min-h-0 flex-1 flex-col"
      tabIndex={-1}
    >
      <div className="reports-workspace__scroll">
        <header className="reports-hero catalog-banner">
          <p className="type-body text-secondary">
            Ask the assistant to save a dashboard in a chat to add a report here.
          </p>
          <dl className="reports-stats" aria-label="Library overview">
            <div>
              <dt>
                <Library size={15} aria-hidden />
                Saved reports
              </dt>
              <dd>{status === "ready" ? listed.length : <Unknown />}</dd>
            </div>
            <div>
              <dt>
                <Pin size={15} aria-hidden />
                Pinned
              </dt>
              <dd>{status === "ready" ? listed.filter((r) => r.pinned).length : <Unknown />}</dd>
            </div>
          </dl>
        </header>
        <div className="reports-toolbar">
          <div className="reports-toolbar__filters">
            <SegmentedControl<ReportFilter>
              aria-label="Filter reports"
              className="reports-segmented"
              options={[
                { value: "all", label: "All reports" },
                { value: "mine", label: "Mine" },
                { value: "shared", label: "Shared with me" },
                { value: "pinned", label: "Pinned", icon: <Pin size={13} aria-hidden /> },
              ]}
              value={filter}
              onChange={setFilter}
            />
            {showProjectFilter ? (
              <div className="reports-project-filter">
                <span id="report-project-label" className="sr-only">
                  Project
                </span>
                <Select<ProjectFilter>
                  aria-labelledby="report-project-label"
                  value={activeProject}
                  onChange={setProjectFilter}
                  options={[
                    { value: "all", label: "All projects", icon: <Folder size={14} aria-hidden /> },
                    { value: "none", label: "No project", icon: <FolderX size={14} aria-hidden /> },
                    ...choices.map((choice) => ({
                      value: choice.id,
                      label: `${choice.name} (${choice.count})`,
                      icon: <FolderOpen size={14} aria-hidden />,
                    })),
                  ]}
                />
              </div>
            ) : null}
            {hiddenCount > 0 || showHidden ? (
              <Switch
                checked={showHidden}
                label={
                  <span className="reports-hidden-toggle">
                    <EyeOff size={14} aria-hidden />
                    {showHidden ? "Showing hidden" : `Show hidden (${hiddenCount})`}
                  </span>
                }
                onChange={setShowHidden}
              />
            ) : null}
          </div>
          <div className="reports-toolbar__tools">
            <SearchField
              aria-label="Search reports"
              placeholder="Search library"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
              onClear={() => setQuery("")}
            />
            <div className="reports-sort">
              <span id="report-sort-label" className="sr-only">
                Sort reports
              </span>
              <Select
                aria-labelledby="report-sort-label"
                value={sort}
                onChange={setSort}
                options={[
                  { value: "updated", label: "Recently updated", icon: <ArrowUpDown size={14} /> },
                  { value: "created", label: "Newest created" },
                  { value: "name", label: "Name A–Z" },
                ]}
              />
            </div>
            <div role="group" aria-label="Report layout" className="reports-layout">
              <IconButton aria-label="Grid view" selected={layout === "grid"} onClick={() => setLayout("grid")}>
                <LayoutGrid size={16} aria-hidden />
              </IconButton>
              <IconButton aria-label="List view" selected={layout === "list"} onClick={() => setLayout("list")}>
                <List size={16} aria-hidden />
              </IconButton>
            </div>
          </div>
        </div>
        <div className="reports-results-heading">
          <p aria-live="polite" className="type-caption text-muted" role="status">
            {summary(status, listed.length, visible.length, filter, activeProject, query)}
          </p>
          {filter === "mine" && status === "ready" ? <ReportPreviewBackfill reports={listed} /> : null}
          <span className="type-micro text-muted">Saved reports</span>
        </div>
        <div className="reports-results">
          {actionError !== null ? (
            <p className="mb-4 type-caption text-danger" role="alert">
              {actionError}
            </p>
          ) : null}
          {status === "loading" ? (
            // Shaped like what arrives — a card row then table rows — so the
            // surface does not reflow into a different layout once it lands.
            <div aria-hidden className="flex flex-col gap-6">
              <div className={CARD_GRID}>
                {["w-3/5", "w-4/5", "w-1/2"].map((width) => (
                  <div className="surface-card flex flex-col gap-2 p-3.5" key={width}>
                    <Skeleton className={cn("h-4", width)} />
                    <Skeleton className="h-3 w-full" />
                    <Skeleton className="h-3 w-1/3" />
                  </div>
                ))}
              </div>
              <div className="flex flex-col gap-2">
                {["w-2/5", "w-3/5", "w-1/3"].map((width) => (
                  <div className="flex items-center gap-3" key={width}>
                    <Skeleton className={cn("h-4", width)} />
                    <Skeleton className="h-4 w-16" />
                  </div>
                ))}
              </div>
            </div>
          ) : null}

          {status === "error" ? (
            <EmptyState
              actions={
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => {
                    setActionError(null);
                    void reports.refresh();
                  }}
                >
                  Try again
                </Button>
              }
              description="Saved reports could not be read."
              icon={<ChartNoAxesColumn aria-hidden size={28} strokeWidth={1.6} />}
              title="Reports unavailable"
            />
          ) : null}

          {status === "ready" && all.length === 0 ? (
            <EmptyState
              description="Ask the assistant to save a dashboard it built, and it will appear here. Reports are stored in xFalcon, so they open on any device you sign in to."
              icon={<ChartNoAxesColumn aria-hidden size={28} strokeWidth={1.6} />}
              title="No reports yet"
            />
          ) : null}

          {status === "ready" && all.length > 0 && listed.length === 0 ? (
            // Everything the reader has is hidden. The way back is said here, because
            // a library that reads as empty when it is not is how a report is lost.
            <EmptyState
              actions={
                <Button type="button" variant="secondary" onClick={() => setShowHidden(true)}>
                  Show hidden reports
                </Button>
              }
              description="Every report you can open is hidden from your list. Show them to bring one back."
              icon={<EyeOff aria-hidden size={28} strokeWidth={1.6} />}
              title="All reports are hidden"
            />
          ) : null}

          {status === "ready" && listed.length > 0 && visible.length === 0 ? (
            <EmptyState
              description={
                query.trim()
                  ? "The current search does not match a saved report. Try part of its name or description."
                  : emptyView(filter, activeProject, activeProjectName).text
              }
              actions={
                // Not "Clear search": the search field already offers a control by
                // that name, and two buttons with one accessible name leaves a
                // screen-reader user unable to tell them apart.
                <Button
                  type="button"
                  variant="secondary"
                  onClick={() => {
                    setQuery("");
                    setFilter("all");
                    setProjectFilter("all");
                  }}
                >
                  Show all reports
                </Button>
              }
              icon={<Search aria-hidden size={28} strokeWidth={1.6} />}
              title={
                query.trim()
                  ? `No reports match “${query.trim()}”`
                  : emptyView(filter, activeProject, activeProjectName).title
              }
            />
          ) : null}

          {status === "ready" && visible.length > 0 && layout === "list" ? (
            <ReportZone heading="Report library">
              <ReportTable
                actions={actions}
                busyById={reports.busyById}
                projectOrigins={projectOrigins}
                reports={visible}
                onOpen={open}
              />
            </ReportZone>
          ) : null}
          {status === "ready" && visible.length > 0 && layout === "grid" ? (
            <div className="flex flex-col gap-6">
              {zones.pinned.length > 0 ? (
                <ReportZone heading="Pinned">
                  <div className={CARD_GRID}>
                    {zones.pinned.map((report) => (
                      <ReportCard
                        busyAction={reports.busyById[report.id] ?? null}
                        key={report.id}
                        projectName={originOf(report.id, projectOrigins)?.project_name ?? null}
                        report={report}
                        onOpen={open}
                        {...actions}
                      />
                    ))}
                  </div>
                </ReportZone>
              ) : null}

              {zones.recent.length > 0 ? (
                <ReportZone
                  heading={
                    filter === "pinned"
                      ? "Pinned reports"
                      : sort !== "updated"
                        ? "Report library"
                        : zones.pinned.length > 0
                          ? "Recent"
                          : "Recent reports"
                  }
                >
                  <div className={CARD_GRID}>
                    {zones.recent.map((report) => (
                      <ReportCard
                        busyAction={reports.busyById[report.id] ?? null}
                        key={report.id}
                        projectName={originOf(report.id, projectOrigins)?.project_name ?? null}
                        report={report}
                        onOpen={open}
                        {...actions}
                      />
                    ))}
                  </div>
                </ReportZone>
              ) : null}

              {zones.older.length > 0 ? (
                // Named for what it holds. Web calls this "All Reports" while
                // excluding everything already in a card above it.
                <ReportZone heading="Older reports">
                  <ReportTable
                    actions={actions}
                    busyById={reports.busyById}
                    projectOrigins={projectOrigins}
                    reports={zones.older}
                    onOpen={open}
                  />
                </ReportZone>
              ) : null}
            </div>
          ) : null}
        </div>
      </div>

      {shareDialog}

      <ReportNameDialog
        busy={renameBusy}
        confirmLabel="Save"
        heading="Rename report"
        initialDescription={renameTarget?.description ?? ""}
        initialTitle={renameTarget?.title ?? ""}
        error={renameError}
        // Advisory: this surface holds the whole list, so it can say when a name
        // is already in use. DA allows duplicate names, so it never blocks.
        nameInUse={(title) => isNameTaken(all, title, renameTarget?.id ?? null)}
        open={renameTarget !== null}
        onCancel={() => {
          if (renameBusy) return;
          setRenameError(null);
          setRenameTarget(null);
        }}
        onConfirm={(title, description) => {
          if (renameTarget === null || renameBusy) return;
          const target = renameTarget;
          setRenameError(null);
          void reports.rename(target, title, description).then((result) => {
            if (result === "succeeded") setRenameTarget(null);
            if (result === "failed") setRenameError("The report kept its previous name. Check the name and try again.");
            if (result === "forbidden") setRenameError("Only the owner can rename this report.");
          });
        }}
      />

      {/* Deleting removes the report from the server for everyone it is shared
          with, and the conversation it came from may be long gone — so it always
          asks first. */}
      <AlertDialog
        confirmLabel="Delete"
        description={
          pendingDelete === null
            ? "This permanently removes the saved report. It cannot be undone."
            : `“${pendingDelete.title}” will be permanently removed. This cannot be undone.`
        }
        destructive
        busy={deleteBusy}
        error={deleteError}
        open={pendingDelete !== null}
        title="Delete report?"
        onCancel={() => {
          if (deleteBusy) return;
          setDeleteError(null);
          setPendingDelete(null);
        }}
        onConfirm={() => {
          if (pendingDelete === null || deleteBusy) return;
          const target = pendingDelete;
          setDeleteError(null);
          void reports.remove(target).then((result) => {
            if (result === "succeeded") {
              setPendingDelete(null);
              if (openReportId === target.id) closeViewer();
              // The Delete button that opened the dialog went with its card, so
              // focus would fall to the page: keep it in the library instead.
              workspace.current?.focus({ preventScroll: true });
            }
            if (result === "failed") setDeleteError("The report was not deleted. Try again.");
            if (result === "forbidden") setDeleteError("Only the owner can delete this report.");
          });
        }}
      />
    </section>
  );
}

/** A count that is not known (still loading, or the read failed): a dash on screen, words for a screen reader. */
function Unknown() {
  return (
    <>
      <span aria-hidden>—</span>
      <span className="sr-only">Not available</span>
    </>
  );
}

/** Whether a report passes the reader's filter. */
function matchesFilter(report: ReportView, filter: ReportFilter): boolean {
  switch (filter) {
    case "all":
      return true;
    case "mine":
      return report.is_owner;
    case "shared":
      return !report.is_owner;
    case "pinned":
      return report.pinned;
  }
}

/** What an empty result says, by what the reader filtered on. A project wins over
 * the relationship filter, because it is the more specific thing they asked for. */
function emptyView(
  filter: ReportFilter,
  project: ProjectFilter,
  projectName: string | null,
): { title: string; text: string } {
  if (project === "none") {
    return {
      title: "No reports outside a project",
      text: "Every report here was made in a project chat. Reports from regular chats appear here.",
    };
  }
  if (project !== "all") {
    return {
      title: `No reports in ${projectName ?? "this project"}`,
      text: "Ask the assistant to save a dashboard in a chat of this project, and it appears here.",
    };
  }
  switch (filter) {
    case "mine":
      return {
        title: "You have not saved a report yet",
        text: "Reports you ask the assistant to save appear here.",
      };
    case "shared":
      return {
        title: "No one has shared a report with you",
        text: "A report someone shares with your organization appears here.",
      };
    default:
      return { title: "No pinned reports yet", text: "Pin a report to keep it at the top of your list." };
  }
}

function summary(
  status: "loading" | "ready" | "error",
  total: number,
  matching: number,
  filter: ReportFilter,
  project: ProjectFilter,
  query: string,
): string {
  if (status === "loading") return "Loading…";
  if (status === "error") return "Could not be read";
  if (matching !== total && (query.trim() !== "" || filter !== "all" || project !== "all")) {
    return `${matching} of ${total} report${total === 1 ? "" : "s"} shown`;
  }
  return `${total} saved report${total === 1 ? "" : "s"}`;
}

/** Shared responsive tracks keep pinned and recent cards aligned. */
const CARD_GRID = "report-card-list";

/** A titled band of the list. The heading is a real `h2` so the zones are
 * navigable landmarks to a screen reader rather than three visually separated
 * groups of identical-sounding controls. */
function ReportZone({ heading, children }: { heading: string; children: React.ReactNode }) {
  return (
    <section aria-label={heading} className="report-zone flex flex-col gap-2">
      <Eyebrow className="mb-0" level={2}>
        {heading}
      </Eyebrow>
      {children}
    </section>
  );
}
