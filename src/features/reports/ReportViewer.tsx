import { useCallback, useEffect, useId, useRef, useState } from "react";
import { ArrowLeft, ChartNoAxesColumn, RefreshCw, Share2 } from "lucide-react";
import { cn } from "../../lib/cn";
import { previewDocumentSrc, reportExport, reportOpen, reportRefresh } from "../../lib/native";
import { IconButton } from "../../ui/IconButton";
import { Button } from "../../ui/Button";
import { SegmentedControl } from "../../ui/SegmentedControl";
import { Select } from "../../ui/Select";
import { Skeleton } from "../../ui/Skeleton";
import { toast } from "../../ui/toastStore";
import type { ReportDatasetFreshness, ReportView } from "../../lib/generated/contracts";
import { HighlightedText, SearchRevealHost, SearchRevealProvider } from "../../lib/highlightedText";

import { ReportDataStatus, type DataRefreshState } from "./ReportDataStatus";
import { ReportFilePreview } from "./ReportFilePreview";
import { ReportProjectChip } from "./ReportProjectChip";
import { ReportProjectCopy } from "./ReportProjectCopy";
import { ReportTools } from "./ReportTools";
import { applyReportUpdated } from "./reportsStore";
import { captureMissingPreviewOnce } from "./reportPreviewCapture";
import { isForbidden } from "./useReports";
import { freshnessSummary, isFileReport, reportAudience, reportFormatLabel, reportHandle } from "./reportPresentation";

type ViewerState = { status: "opening" } | { status: "ready"; src: string } | { status: "failed"; message: string };

/** How a page is brought in. A first open and a page switch blank the frame to a
 * skeleton; a reload after a refresh keeps the page on screen until the new one
 * has loaded, so a refresh never flashes the report away. */
type OpenMode = "replace" | "keep";

/** Above this many pages a segmented control stops fitting the header. */
const SEGMENTED_PAGE_LIMIT = 4;
const OPEN_FAILED = "This report could not be opened.";
const TOO_LARGE = "This report is too large to open.";

function refreshFailureToast(cause: unknown): string {
  if (isForbidden(cause)) return "You are not allowed to refresh this report";
  if (hasCode(cause, "not_refreshable")) return "This report has no stored queries, so it cannot be refreshed";
  return "Could not refresh report";
}

function hasCode(cause: unknown, code: string): boolean {
  return typeof cause === "object" && cause !== null && "code" in cause && cause.code === code;
}

function isTooLarge(cause: unknown): boolean {
  return hasCode(cause, "too_large_to_open");
}

/**
 * One saved report, rendered from the `xf-preview` origin.
 *
 * Isolation contract, carried across from `chat/richContent/cards.tsx` so the
 * reason travels with the second consumer: the document loads from its own
 * non-app origin under a per-response CSP that allows its inline styles and
 * scripts while denying ALL network (spec: preview-protocol). Local-scheme URLs
 * (`data:` / `blob:` / `srcdoc`) are banned here — WebView2 makes them inherit
 * the app CSP, which refuses the document's own styling, and a saved report
 * rendered that way is permanently wrong rather than momentarily wrong.
 * `sandbox="allow-scripts"` and NEVER `allow-same-origin` stays as defense in
 * depth: even on its own origin the document gets no DOM, storage, or Tauri IPC
 * reach into the app.
 *
 * The document id is resolved on every mount and never cached. The preview
 * store is in-memory and bounded with least-recently-served eviction, so an id
 * is not durable: other documents can evict it, and it is gone after a restart.
 * Re-opening on mount is what makes an evicted id survivable, and it is
 * complete rather than best-effort: `report_open` registers the bytes again,
 * which mints a live id AND freshens its recency, so the id handed to the frame
 * is the most-recently-served entry in the store.
 *
 * The spec asks for one re-open before the reader is told anything, and this is
 * where that happens — on a failed `report_open`, not on a failed frame load.
 * The frame cannot report its own failure: it is deliberately cross-origin and
 * sandboxed without same-origin access, and the preview protocol answers an
 * unknown id with a 404 whose body the frame then LOADS, so `onError` never
 * fires and `onLoad` does. Wiring a retry to that event would be decorative.
 *
 * A report too large to open is not retried: the answer will not change.
 *
 * Refresh re-runs the report's datasets in DA, then re-opens the page on a new
 * document id, so the frame shows the new data rather than the registered bytes
 * it held before.
 */
export function ReportViewer({
  report,
  projectName = null,
  onBack,
  onShare,
  revealQuery = "",
}: {
  report: ReportView;
  /** The project this report was created in, when this device knows it. */
  projectName?: string | null;
  onBack: () => void;
  /** Opens the sharing dialog for this report. Only offered when it can be managed. */
  onShare: (report: ReportView) => void;
  revealQuery?: string;
}) {
  // A workbook or deck is one file DA built: there is no page to open, so the
  // viewer shows a file card and never calls `report_open` for it.
  const isFile = isFileReport(report);
  const [state, setState] = useState<ViewerState>({ status: "opening" });
  // When DA stored each dataset behind the page on screen, from the last open.
  const [datasets, setDatasets] = useState<ReportDatasetFreshness[]>([]);
  // Whether a refresh is running, and what each dataset did on the last one,
  // until the next page is opened.
  const [refreshState, setRefreshState] = useState<DataRefreshState>({ phase: "idle" });
  // Counts refreshes, so each one's status strip starts from its own default
  // (a failure opens its details even if the reader closed the last refresh's).
  const [refreshSeq, setRefreshSeq] = useState(0);
  // A page being brought in behind the one on screen after a refresh. It takes
  // over when it has loaded.
  const [incoming, setIncoming] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const exportInFlight = useRef(false);
  const [loaded, setLoaded] = useState(false);
  // An owner's dashboard with no stored image gets one once the reader has seen
  // it load, so the capture never competes with the page the reader is waiting for.
  useEffect(() => {
    if (loaded && !isFile) captureMissingPreviewOnce(report);
  }, [loaded, isFile, report]);
  // The page on screen and every page the report has, from the last open.
  const [pageState, setPageState] = useState<{ page: string; pages: string[] } | null>(null);
  // The latest state, for an open that needs to know whether a page is on screen
  // without being rebuilt every time one arrives.
  const stateRef = useRef(state);
  useEffect(() => {
    stateRef.current = state;
  }, [state]);
  const [refreshing, setRefreshing] = useState(false);
  const openRequest = useRef(0);
  const requestedPage = useRef<string | null>(null);
  const refreshRequest = useRef(0);
  const refreshInFlight = useRef(false);
  const pageLabelId = useId();
  const root = useRef<HTMLElement>(null);

  /** Resolves how the open ended: `ready`, `failed`, or `superseded` when a newer
   * open or an unmount took over, so a caller never reports on a frame that
   * is not the one on screen. */
  const open = useCallback(
    async (page: string | null, mode: OpenMode = "replace"): Promise<"ready" | "failed" | "superseded"> => {
      const request = ++openRequest.current;
      requestedPage.current = page;
      setIncoming(null);
      // Only a page that is on screen can be kept: after a failed open there is
      // nothing to keep, and the new page is brought in the ordinary way.
      const keeping = mode === "keep" && stateRef.current.status === "ready";
      if (!keeping) {
        setLoaded(false);
        setState({ status: "opening" });
      }
      for (const attempt of [0, 1]) {
        try {
          const document = await reportOpen(report.id, page);
          if (request !== openRequest.current) return "superseded";
          if (document.id !== report.id) throw new Error("report identity mismatch");
          setPageState({ page: document.page, pages: document.pages });
          setDatasets(document.datasets);
          const src = previewDocumentSrc(document.document_id);
          if (keeping) setIncoming(src);
          else setState({ status: "ready", src });
          return "ready";
        } catch (cause) {
          if (request !== openRequest.current) return "superseded";
          if (isTooLarge(cause)) {
            setIncoming(null);
            setState({ status: "failed", message: TOO_LARGE });
            return "failed";
          }
          // Only the second failure is worth showing: the first is far more
          // likely a transient read than a lost report.
          if (attempt === 1) {
            setIncoming(null);
            setState({ status: "failed", message: OPEN_FAILED });
          }
        }
      }
      return "failed";
    },
    [report.id],
  );

  // The card that was clicked is gone once the viewer replaces the list, which
  // would leave focus on the page: move it into the viewer instead.
  useEffect(() => {
    root.current?.focus({ preventScroll: true });
  }, []);

  useEffect(() => {
    if (!isFile) void open(null);
    return () => {
      // Unmounting (back to the list, or another report) abandons any open or
      // refresh still in flight, so its result cannot land on this viewer.
      openRequest.current += 1;
      refreshRequest.current += 1;
    };
  }, [open, isFile]);

  const choosePage = (page: string) => {
    // Re-selecting the page on screen is a no-op only while it is on screen: after a
    // failed switch the switcher still shows the old page, and choosing it must
    // reopen it rather than leave the reader with only "Try again" for the other.
    if (pageState === null || (page === pageState.page && state.status === "ready")) return;
    setRefreshState({ phase: "idle" });
    void open(page);
  };

  const exportReport = async () => {
    if (exportInFlight.current) return;
    exportInFlight.current = true;
    setExporting(true);
    try {
      const result = await reportExport(report.id, pageState?.page ?? null);
      // A dismissed save dialog is the reader's answer, not a failure.
      if (result.saved) toast.success(result.file_name === null ? "Report exported" : `Saved ${result.file_name}`);
    } catch (cause) {
      toast.error(isForbidden(cause) ? "You are not allowed to export this report" : "Could not export report");
    } finally {
      exportInFlight.current = false;
      setExporting(false);
    }
  };

  const summary = freshnessSummary(report, datasets);

  const refresh = async () => {
    if (refreshInFlight.current) return;
    refreshInFlight.current = true;
    const request = ++refreshRequest.current;
    setRefreshing(true);
    setRefreshSeq((count) => count + 1);
    setRefreshState({ phase: "refreshing" });
    try {
      const result = await reportRefresh(report.id);
      // The library learns what DA now holds even when the reader has left this
      // viewer: the refresh happened, and the list would otherwise keep the old
      // updated time and size until the destination is next opened.
      if (result.report.id === report.id) applyReportUpdated(result.report);
      if (request !== refreshRequest.current) return;
      const failed = result.outcomes.some((outcome) => !outcome.ok);
      setRefreshState({ phase: "done", outcomes: result.outcomes });
      // A file has no page to put back. A dashboard's control stays busy until the
      // page is on its way back, and success is only claimed when the reopen did
      // not fail: a "refreshed" toast followed by "could not be opened" would
      // contradict itself.
      const reopened = isFile ? "ready" : await open(requestedPage.current, "keep");
      if (request !== refreshRequest.current) return;
      if (!failed && reopened === "ready") toast.success("Report refreshed");
    } catch (cause) {
      if (request !== refreshRequest.current) return;
      setRefreshState({ phase: "idle" });
      toast.error(refreshFailureToast(cause));
    } finally {
      refreshInFlight.current = false;
      if (request === refreshRequest.current) setRefreshing(false);
    }
  };

  return (
    <SearchRevealProvider value={revealQuery ? { query: revealQuery, targetId: "report-reveal" } : null}>
      <section
        ref={root}
        aria-busy={refreshing || undefined}
        aria-label={`Report ${report.title}`}
        className="flex min-h-0 flex-1 flex-col outline-none"
        tabIndex={-1}
      >
        <header className="report-viewer-header flex shrink-0 items-center gap-2 border-b border-hairline px-4 py-2.5">
          <IconButton aria-label="Back to reports" type="button" onClick={onBack}>
            <ArrowLeft aria-hidden size={18} />
          </IconButton>
          <ChartNoAxesColumn aria-hidden className="shrink-0 text-muted" size={16} strokeWidth={1.8} />
          <div className="report-viewer-title">
            <SearchRevealHost id="report-reveal">
              <div className="min-w-0">
                <p className="truncate text-sm font-medium text-foreground">
                  <HighlightedText text={report.title} />
                </p>
                {report.description !== null ? (
                  <p className="truncate type-caption text-muted">
                    <HighlightedText text={report.description} />
                  </p>
                ) : null}
              </div>
            </SearchRevealHost>
          </div>
          {/* Where it was made, when this device knows, and who can open it, each as a
              word. The owner's control to change the second follows. */}
          {projectName !== null ? <ReportProjectChip name={projectName} /> : null}
          <span className="shrink-0 type-caption text-muted">{reportAudience(report)}</span>
          {report.can_manage ? (
            <IconButton
              aria-label={`Share ${reportHandle(report)}`}
              size="sm"
              tooltip="Change who can open this report"
              type="button"
              onClick={() => onShare(report)}
            >
              <Share2 aria-hidden size={14} />
            </IconButton>
          ) : null}
          {pageState !== null && pageState.pages.length > 1 ? (
            <div className="flex min-w-0 items-center gap-2">
              <span id={pageLabelId} className="type-caption text-muted">
                Page
              </span>
              {pageState.pages.length <= SEGMENTED_PAGE_LIMIT ? (
                <SegmentedControl
                  aria-labelledby={pageLabelId}
                  options={pageState.pages.map((name) => ({ value: name, label: name }))}
                  value={pageState.page}
                  onChange={choosePage}
                />
              ) : (
                <Select
                  aria-labelledby={pageLabelId}
                  options={pageState.pages.map((name) => ({ value: name, label: name }))}
                  value={pageState.page}
                  onChange={choosePage}
                />
              )}
            </div>
          ) : null}
          {report.can_refresh ? (
            // The name starts with the visible word and carries the report's handle,
            // as every report-scoped control does.
            <Button
              aria-label={`Refresh ${reportHandle(report)}`}
              type="button"
              variant="secondary"
              loading={refreshing}
              onClick={() => void refresh()}
            >
              {refreshing ? null : <RefreshCw aria-hidden size={14} />}
              {refreshing ? "Refreshing" : "Refresh"}
            </Button>
          ) : null}
          {/* Always in the page, with its words changing: a live region that
              arrives together with its text is not reliably announced. */}
          <span className="sr-only" role="status">
            {refreshing
              ? "Refreshing report…"
              : exporting
                ? "Exporting report…"
                : !isFile && (state.status === "opening" || (state.status === "ready" && !loaded))
                  ? "Opening report…"
                  : ""}
          </span>
          {isFile ? null : (
            <ReportProjectCopy
              disabled={state.status !== "ready" || !loaded}
              page={pageState?.page ?? null}
              report={report}
            />
          )}
          <ReportTools
            key={report.id}
            disabled={!isFile && (state.status !== "ready" || !loaded)}
            exportLabel={isFile ? reportFormatLabel(report.kind) : "an HTML file"}
            exporting={exporting}
            title={report.title}
            onExport={() => void exportReport()}
          />
        </header>

        {/* Where the data came from, said plainly: DA stamps when it stored each
            dataset, and a dataset never refreshed still shows the rows the page was
            saved with. The strip never claims the data is current. */}
        {isFile ? (
          // A file has no page to date, so its strip appears only to say what a
          // refresh did.
          refreshState.phase !== "idle" ? (
            <ReportDataStatus
              key={refreshSeq}
              datasets={[]}
              refresh={refreshState}
              summary="Download the file to get the refreshed data."
            />
          ) : null
        ) : state.status === "ready" && summary !== null ? (
          <ReportDataStatus key={refreshSeq} datasets={datasets} refresh={refreshState} summary={summary} />
        ) : null}

        {isFile ? (
          <ReportFilePreview exporting={exporting} onDownload={() => void exportReport()} report={report} />
        ) : (
          <div
            aria-busy={state.status === "opening" || (state.status === "ready" && !loaded)}
            className="relative min-h-0 flex-1"
          >
            {state.status === "opening" || (state.status === "ready" && !loaded) ? (
              <Skeleton className="absolute inset-0 h-full w-full" />
            ) : null}

            {state.status === "failed" ? (
              <div className="flex h-full flex-col items-center justify-center gap-2 p-6 text-center">
                <p className="text-sm text-foreground">{state.message}</p>
                <Button type="button" variant="secondary" onClick={() => void open(requestedPage.current)}>
                  Try again
                </Button>
              </div>
            ) : null}

            {state.status === "ready"
              ? // The page on screen, and while a refresh reloads it the new one
                // behind it. Both are mounted by their own address, so when the new
                // page takes over, its frame is the one already loaded and nothing
                // is fetched twice.
                [state.src, ...(incoming !== null && incoming !== state.src ? [incoming] : [])].map((src) => {
                  const visible = src === state.src;
                  return (
                    <iframe
                      key={src}
                      className={cn(
                        "absolute inset-0 h-full w-full border-0",
                        visible ? "opacity-0 transition-opacity" : "invisible pointer-events-none",
                        visible && loaded && "opacity-100",
                        visible && !loaded && "pointer-events-none",
                      )}
                      aria-hidden={visible ? !loaded : true}
                      referrerPolicy="no-referrer"
                      sandbox="allow-scripts"
                      src={src}
                      tabIndex={visible && loaded ? 0 : -1}
                      title={report.title}
                      onLoad={() => {
                        if (visible) setLoaded(true);
                        // The new page has loaded behind the old one: it takes over
                        // in one step, and the old frame leaves in the same commit.
                        else if (src === incoming) {
                          setState({ status: "ready", src });
                          setIncoming(null);
                          // It has loaded, even if the page it replaces had not.
                          setLoaded(true);
                        }
                      }}
                    />
                  );
                })
              : null}
          </div>
        )}
      </section>
    </SearchRevealProvider>
  );
}
