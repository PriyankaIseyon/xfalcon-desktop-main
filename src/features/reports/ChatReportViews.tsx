import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { LayoutGrid } from "lucide-react";
import { cn } from "../../lib/cn";
import type { ChatToolCallView, ReportDocument } from "../../lib/generated/contracts";
import { previewDocumentSrc, reportOpen } from "../../lib/native";
import { Skeleton } from "../../ui/Skeleton";
import { ARTIFACT_ACTION_CLASS } from "../chat/richContent/artifactAction";
import { ArtifactFrame } from "../chat/richContent/cards";
import { REMEASURE_EVENT } from "../chat/ToolActivityCard";
import { requestOpenReport } from "./reportsStore";
import { chatReportDocument } from "./chatReportDocuments";

/** The model-facing name of xFalcon's "show a saved report" tool on the bundled
 * server: `mcp.tenant.ida_view_dashboard` in provider charset, with the `_N`
 * suffix a name collision would add. Only the bundled server's reports are the
 * ones this app opens over REST, so another server's tool of that name is not
 * matched. */
const VIEW_CALL = /^mcp_tenant_ida_view_dashboard(?:_\d+)?$/;

type ReportView = { callId: string; kitId: string; page: string | null };

/**
 * The saved reports a turn asked xFalcon to show, drawn in the chat under that
 * turn's tool activity.
 *
 * xfalcon-web shows these through DA's MCP Apps viewer. This app hosts no MCP
 * Apps view, so it reads the same saved report the Reports page opens, over the
 * same REST path, into the same isolated preview frame. What the model sent is
 * only a report id and a page: the report itself comes from xFalcon, checked to
 * be that report, under the signed-in account.
 */
export function ChatReportViews({ calls }: { calls: readonly ChatToolCallView[] }) {
  const views = useMemo(() => reportViews(calls), [calls]);
  if (views.length === 0) return null;
  return (
    <div className="flex flex-col gap-3 pt-2">
      {views.map((view) => (
        <ChatReportView key={view.callId} {...view} />
      ))}
    </div>
  );
}

/** Each answered view call's report, once per report page in a turn. */
function reportViews(calls: readonly ChatToolCallView[]): ReportView[] {
  const views: ReportView[] = [];
  const seen = new Set<string>();
  for (const call of calls) {
    if (call.status !== "answered" || !VIEW_CALL.test(call.name)) continue;
    if (call.arguments === null || call.arguments_truncated) continue;
    const target = viewTarget(call.arguments);
    if (target === null) continue;
    const key = `${target.kitId}\n${target.page ?? ""}`;
    if (seen.has(key)) continue;
    seen.add(key);
    views.push({ callId: call.call_id, ...target });
  }
  return views;
}

/** The report and page a call's arguments name, or `null` when they name none
 * this app could open. The id is checked again by the backend. */
function viewTarget(argumentsText: string): { kitId: string; page: string | null } | null {
  let parsed: unknown;
  try {
    parsed = JSON.parse(argumentsText);
  } catch {
    return null;
  }
  if (typeof parsed !== "object" || parsed === null) return null;
  const { kit_id: kitId, page } = parsed as Record<string, unknown>;
  if (typeof kitId !== "string" || kitId.length === 0 || kitId.length > 64) return null;
  if (page !== undefined && page !== null && (typeof page !== "string" || page.length > 512)) return null;
  return { kitId, page: typeof page === "string" && page.length > 0 ? page : null };
}

type State = { status: "loading" } | { status: "ready"; document: ReportDocument } | { status: "failed" };

function ChatReportView({ callId, kitId, page }: ReportView) {
  const [state, setState] = useState<State>({ status: "loading" });
  const [loaded, setLoaded] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  // A failed card is shorter than the frame it replaced, and the transcript's
  // row cache only grows on its own: tell the row to measure again.
  useLayoutEffect(() => {
    if (state.status === "failed") root.current?.dispatchEvent(new CustomEvent(REMEASURE_EVENT, { bubbles: true }));
  }, [state.status]);

  useEffect(() => {
    let current = true;
    chatReportDocument(callId, () => reportOpen(kitId, page)).then(
      (document) => {
        if (!current) return;
        setState(document.id === kitId ? { status: "ready", document } : { status: "failed" });
      },
      () => {
        // Deleted, not the reader's, a workbook or deck with no page, or xFalcon
        // unreachable: the Reports page says which, so the card points there.
        if (current) setState({ status: "failed" });
      },
    );
    return () => {
      current = false;
    };
  }, [callId, kitId, page]);

  const title = state.status === "ready" ? state.document.title : "Saved report";
  const openInReports = (
    <button
      aria-label={`Open ${title} in Reports`}
      className={ARTIFACT_ACTION_CLASS}
      type="button"
      onClick={() => requestOpenReport(kitId)}
    >
      <LayoutGrid aria-hidden size={14} strokeWidth={1.8} />
      Open in Reports
    </button>
  );

  if (state.status === "failed") {
    return (
      <div ref={root}>
        <ArtifactFrame
          canvasAction={openInReports}
          degraded
          description="Saved report"
          footer={
            <p className="rich-card__footer type-caption text-muted" role="status">
              This report could not be shown here. Open it from Reports.
            </p>
          }
          title={title}
        />
      </div>
    );
  }

  return (
    <div ref={root}>
      <ArtifactFrame
        canvasAction={state.status === "ready" ? openInReports : undefined}
        description={
          state.status === "ready" && state.document.pages.length > 1
            ? `Saved report · ${state.document.page}`
            : "Saved report"
        }
        title={title}
      >
        {state.status === "ready" ? (
          <iframe
            className={cn("rich-card__frame", loaded && "rich-card__frame--loaded")}
            referrerPolicy="no-referrer"
            sandbox="allow-scripts"
            src={previewDocumentSrc(state.document.document_id)}
            title={title}
            onLoad={() => setLoaded(true)}
          />
        ) : (
          <Skeleton className="rich-card__frame-ghost" />
        )}
      </ArtifactFrame>
    </div>
  );
}
