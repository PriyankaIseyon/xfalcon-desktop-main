import { useEffect, useMemo, useRef, useState } from "react";
import { ImageUp } from "lucide-react";
import type { ReportView } from "../../lib/generated/contracts";
import { Button } from "../../ui/Button";
import { toast } from "../../ui/toastStore";
import { canCapturePreview, captureReportPreview } from "./reportPreviewCapture";

type Run = { total: number; done: number; failed: number };

/**
 * Makes the missing images of the reader's own dashboards, in Mine.
 *
 * A dashboard saved before DA kept images, or saved where none was captured,
 * draws itself live on its card. This makes each one's image in turn (captures
 * run one at a time), says how far it has got, and can be stopped; a report
 * stopped part way keeps what it had. Each image that lands replaces its card's
 * live drawing at once, through the library.
 */
export function ReportPreviewBackfill({ reports }: { reports: ReportView[] }) {
  const missing = useMemo(
    () => reports.filter((report) => canCapturePreview(report) && report.thumbnail_updated_at_ms === null),
    [reports],
  );
  const [run, setRun] = useState<Run | null>(null);
  const controller = useRef<AbortController | null>(null);
  // Leaving Mine, or the page, stops the run: nothing captures unseen.
  useEffect(() => () => controller.current?.abort(), []);

  if (run === null && missing.length === 0) return null;

  const start = async () => {
    const abort = new AbortController();
    controller.current = abort;
    const queue = missing.map((report) => report.id);
    let done = 0;
    let failed = 0;
    setRun({ total: queue.length, done, failed });
    for (const id of queue) {
      if (abort.signal.aborted) break;
      try {
        await captureReportPreview(id, abort.signal);
      } catch {
        if (abort.signal.aborted) break;
        failed += 1;
      }
      done += 1;
      setRun({ total: queue.length, done, failed });
    }
    controller.current = null;
    setRun(null);
    if (abort.signal.aborted) return;
    if (failed === 0) toast.success(done === 1 ? "Preview created" : `${done} previews created`);
    else toast.error(`${failed} of ${done} previews could not be made. Try again later.`);
  };

  return (
    <div className="reports-preview-backfill">
      {run === null ? (
        <Button size="sm" variant="ghost" onClick={() => void start()}>
          <ImageUp aria-hidden size={14} />
          {missing.length === 1 ? "Create missing preview" : `Create ${missing.length} missing previews`}
        </Button>
      ) : (
        <>
          <span aria-live="polite" className="type-caption text-muted" role="status">
            Creating previews · {run.done} of {run.total}
          </span>
          <Button size="sm" variant="ghost" onClick={() => controller.current?.abort()}>
            Stop
          </Button>
        </>
      )}
    </div>
  );
}
