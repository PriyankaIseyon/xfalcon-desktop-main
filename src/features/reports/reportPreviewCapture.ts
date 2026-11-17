import type { ReportView } from "../../lib/generated/contracts";
import { previewDocumentSrc, reportThumbnailCaptureOpen, reportThumbnailSave } from "../../lib/native";
import { applyReportUpdated } from "./reportsStore";

/** DA's capture script waits for the page to settle (fonts, images, two frames
 * and a 1.2 s pause) before it renders twice. A page still silent after this has
 * failed. DA's own viewer gives the same 20 s. */
const CAPTURE_TIMEOUT_MS = 20_000;

/** The frame's size: the width the image is drawn at, before DA's script scales
 * it down to at most 960 px. The same as xfalcon-web's capture frame, so one
 * report's image looks the same whichever surface made it. */
const FRAME_WIDTH = 1024;
const FRAME_HEIGHT = 640;

/** One capture at a time. Each runs a whole dashboard in a frame, and a library
 * backfill must not run twenty at once or spend the reader's DA request budget
 * in a burst. */
let queue: Promise<unknown> = Promise.resolve();

/**
 * Makes a report's thumbnail and stores it, for the report's owner.
 *
 * The backend opens the report's first page with DA's capture script in it. The
 * page runs in a sandboxed frame that is on screen but fully transparent (a frame
 * off screen is throttled and paints nothing), receives a private reply port with
 * the page's nonce, and sends back one PNG. The backend checks that image and
 * stores it on DA, and the library gets the report with its new image version,
 * which is what makes every card showing it load the new image.
 *
 * Rejects when the page cannot be opened, the capture fails or times out, the
 * reader does not own the report (`not_found` from DA), or `signal` aborts it.
 */
export function captureReportPreview(reportId: string, signal?: AbortSignal): Promise<ReportView> {
  const run = queue.then(() => captureNow(reportId, signal));
  queue = run.catch(() => undefined);
  return run;
}

async function captureNow(reportId: string, signal: AbortSignal | undefined): Promise<ReportView> {
  signal?.throwIfAborted();
  const page = await reportThumbnailCaptureOpen(reportId);
  if (page.id !== reportId) throw new Error("capture page answered for another report");
  signal?.throwIfAborted();
  const image = await imageFromFrame(previewDocumentSrc(page.document_id), page.nonce, signal);
  signal?.throwIfAborted();
  const saved = await reportThumbnailSave(reportId, image);
  if (saved.id !== reportId) throw new Error("thumbnail saved for another report");
  applyReportUpdated(saved);
  return saved;
}

function imageFromFrame(src: string, nonce: string, signal: AbortSignal | undefined): Promise<string> {
  return new Promise((resolve, reject) => {
    const frame = document.createElement("iframe");
    const channel = new MessageChannel();
    const finish = (settle: () => void) => {
      window.clearTimeout(timer);
      signal?.removeEventListener("abort", onAbort);
      channel.port1.close();
      frame.remove();
      settle();
    };
    const onAbort = () => finish(() => reject(signal?.reason));
    const timer = window.setTimeout(
      () => finish(() => reject(new Error("thumbnail capture timed out"))),
      CAPTURE_TIMEOUT_MS,
    );

    channel.port1.onmessage = (event: MessageEvent<unknown>) => {
      const reply = event.data;
      if (typeof reply !== "object" || reply === null) return;
      const { type, nonce: answered, image_base64: image } = reply as Record<string, unknown>;
      if (type !== "ida-thumbnail" || answered !== nonce) return;
      if (typeof image === "string") finish(() => resolve(image));
      else finish(() => reject(new Error("thumbnail capture failed")));
    };
    frame.addEventListener(
      "load",
      () => {
        // The wildcard is required: a sandboxed page has an opaque origin. Only
        // the nonce and a reply port go to it, the image comes back over that
        // port, and the page itself checks that this window is its parent.
        frame.contentWindow?.postMessage({ type: "ida-thumbnail-connect", nonce }, "*", [channel.port2]);
      },
      { once: true },
    );
    signal?.addEventListener("abort", onAbort, { once: true });

    frame.title = "Report preview capture";
    frame.setAttribute("sandbox", "allow-scripts");
    frame.setAttribute("referrerpolicy", "no-referrer");
    frame.setAttribute("aria-hidden", "true");
    frame.tabIndex = -1;
    Object.assign(frame.style, {
      position: "fixed",
      left: "0",
      top: "0",
      width: `${FRAME_WIDTH}px`,
      height: `${FRAME_HEIGHT}px`,
      border: "0",
      opacity: "0",
      pointerEvents: "none",
    });
    frame.src = src;
    document.body.append(frame);
  });
}

/** Reports the viewer has already tried to make a missing image for, so opening
 * one again in the same session does not run the capture again. */
const attempted = new Set<string>();

/** Whether this reader can make this report's image: an owner, and a dashboard
 * (a workbook or deck has no page to draw). */
export function canCapturePreview(report: Pick<ReportView, "is_owner" | "kind">): boolean {
  return report.is_owner && report.kind === "dashboard";
}

/**
 * Makes the image of an owner's dashboard that has none, once per session, in
 * the background. A failure is silent: the card still draws the report itself,
 * and the owner can make the image from the card's own action.
 */
export function captureMissingPreviewOnce(report: ReportView) {
  if (!canCapturePreview(report) || report.thumbnail_updated_at_ms !== null || attempted.has(report.id)) return;
  attempted.add(report.id);
  void captureReportPreview(report.id).catch(() => undefined);
}

/** Forgets which reports were tried, on sign-out. */
export function resetPreviewCaptureAttempts() {
  attempted.clear();
}
