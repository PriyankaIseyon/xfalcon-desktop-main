import { Channel, invoke } from "@tauri-apps/api/core";
import type { ProjectOfficePreviewStarted } from "./generated/contracts";

const MAX_PDF_BYTES = 64 * 1024 * 1024;

function checkAborted(signal?: AbortSignal): void {
  if (signal?.aborted) throw signal.reason ?? new DOMException("The preview was abandoned.", "AbortError");
}

/** Registration is acknowledged before work starts. An abort before that
 * acknowledgement is remembered, so cancellation cannot race job creation. */
export interface ProjectOfficePdf {
  data: Uint8Array;
  /** Names this preview to "Save PDF"; the bytes stay with the backend for saving. */
  renderId: string;
}

/** The PDF preview of a project's Word, PowerPoint or Excel file. */
export function projectOfficePreview(projectId: string, path: string, signal?: AbortSignal): Promise<ProjectOfficePdf> {
  return officePreview("project_office_preview", { projectId, path }, signal);
}

/** The PDF preview of a workbook or deck in the Reports library. */
export function reportOfficePreview(reportId: string, signal?: AbortSignal): Promise<ProjectOfficePdf> {
  return officePreview("report_office_preview", { reportId }, signal);
}

async function officePreview(
  command: "project_office_preview" | "report_office_preview",
  args: Record<string, string>,
  signal?: AbortSignal,
): Promise<ProjectOfficePdf> {
  checkAborted(signal);
  let renderId: string | null = null;
  let cancellation: Promise<void> | null = null;
  const cancel = () => {
    if (renderId === null || cancellation !== null) return;
    cancellation = invoke<void>("project_office_preview_cancel", { renderId });
    // Observe immediately to avoid an unhandled rejection while the render is
    // still stopping; the same promise is awaited below to propagate failures.
    void cancellation.catch(() => undefined);
  };
  const onStarted = new Channel<ProjectOfficePreviewStarted>((started) => {
    renderId = started.render_id;
    if (signal?.aborted) cancel();
  });
  signal?.addEventListener("abort", cancel, { once: true });
  let buffer: ArrayBuffer;
  try {
    buffer = await invoke<ArrayBuffer>(command, { ...args, onStarted });
  } finally {
    signal?.removeEventListener("abort", cancel);
    if (cancellation !== null) await cancellation;
  }
  checkAborted(signal);
  if (!(buffer instanceof ArrayBuffer) || buffer.byteLength === 0 || buffer.byteLength > MAX_PDF_BYTES) {
    throw new Error("The Office preview response was invalid.");
  }
  if (renderId === null) throw new Error("The Office preview response was invalid.");
  return { data: new Uint8Array(buffer), renderId };
}
