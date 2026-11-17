import { Download, FileSpreadsheet, Presentation, RotateCcw } from "lucide-react";
import React from "react";
import type { ReportView } from "../../lib/generated/contracts";
import { reportOfficePreview } from "../../lib/native";
import type { ProjectOfficePdf } from "../../lib/projectOfficePreview";
import { Button } from "../../ui/Button";
import { Spinner } from "../../ui/Spinner";
import { OfficePdfViewer } from "../projects/OfficePdfViewer";
import { reportKindLabel, reportFormatLabel } from "./reportPresentation";

type PreviewLane =
  | { status: "idle" }
  | { status: "loading" }
  | { status: "ready"; pdf: ProjectOfficePdf }
  | { status: "error"; message: string };

const FALLBACK = "The preview could not be made.";

/** The backend rejects with the sentence to show; anything else gets the fallback. */
function messageFrom(cause: unknown): string {
  if (typeof cause === "string" && cause.length > 0) return cause;
  if (cause instanceof Error && cause.message.length > 0) return cause.message;
  return FALLBACK;
}

/**
 * A workbook or deck report, shown as a PDF made from the file DA builds. It
 * starts when the report opens and again when a refresh changes the report;
 * the backend reuses a finished PDF for an unchanged report, so reopening it
 * fetches nothing. When no preview can be made, the card below is what the
 * reader had before: the file can still be downloaded.
 */
export function ReportFilePreview({
  report,
  exporting,
  onDownload,
}: {
  report: ReportView;
  exporting: boolean;
  onDownload: () => void;
}) {
  const [lane, setLane] = React.useState<PreviewLane>({ status: "loading" });
  const [attempt, setAttempt] = React.useState(0);
  const controllerRef = React.useRef<AbortController | null>(null);

  React.useEffect(() => {
    const controller = new AbortController();
    controllerRef.current = controller;
    setLane({ status: "loading" });
    reportOfficePreview(report.id, controller.signal).then(
      (pdf) => {
        if (!controller.signal.aborted) setLane({ status: "ready", pdf });
      },
      (cause: unknown) => {
        if (!controller.signal.aborted) setLane({ status: "error", message: messageFrom(cause) });
      },
    );
    return () => {
      controller.abort();
    };
    // A refresh changes `updated_at_ms`, which makes the new file's preview.
  }, [report.id, report.updated_at_ms, attempt]);

  const cancel = () => {
    controllerRef.current?.abort();
    setLane({ status: "idle" });
  };
  const start = () => setAttempt((count) => count + 1);

  if (lane.status === "ready") {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <OfficePdfViewer
          data={lane.pdf.data}
          name={`${report.title}.${report.kind === "workbook" ? "xlsx" : "pptx"}`}
        />
      </div>
    );
  }

  const program = report.kind === "workbook" ? "Excel" : "PowerPoint";
  return (
    <div className="report-file-card">
      {report.kind === "workbook" ? (
        <FileSpreadsheet aria-hidden className="text-muted" size={40} strokeWidth={1.4} />
      ) : (
        <Presentation aria-hidden className="text-muted" size={40} strokeWidth={1.4} />
      )}
      <p className="text-sm font-medium text-foreground">{reportKindLabel(report.kind)}</p>
      {lane.status === "loading" ? (
        <>
          <p className="type-body flex items-center gap-2 text-secondary" role="status">
            <Spinner decorative size={16} />
            Making the PDF preview…
          </p>
          <Button onClick={cancel} size="sm" variant="secondary">
            Cancel
          </Button>
        </>
      ) : (
        <>
          <p className="max-w-md type-caption text-muted">
            {lane.status === "error"
              ? lane.message
              : "A program on this computer opens the file in the background to make a PDF."}{" "}
            xFalcon builds this file from the report&apos;s stored queries. Download it to open it in {program}; refresh
            it first for current data.
          </p>
          <div className="flex items-center gap-2">
            <Button onClick={start} size="sm" variant="secondary">
              <RotateCcw aria-hidden size={14} />
              {lane.status === "error" ? "Try again" : "Preview as PDF"}
            </Button>
            <Button
              aria-label={`Download ${report.title} as ${reportFormatLabel(report.kind)}`}
              loading={exporting}
              type="button"
              variant="primary"
              onClick={onDownload}
            >
              {exporting ? null : <Download aria-hidden size={14} />}
              Download
            </Button>
          </div>
        </>
      )}
    </div>
  );
}
