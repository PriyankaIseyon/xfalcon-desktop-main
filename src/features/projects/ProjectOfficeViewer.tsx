import { Copy, Download, FileDown, RotateCcw } from "lucide-react";
import React from "react";
import type { DocumentExportResult, ProjectDocumentRead, ProjectFileStat } from "../../lib/generated/contracts";
import { formatBytes } from "../../lib/formatBytes";
import {
  projectFileStat,
  projectOfficePreview,
  projectOfficeSaveOriginal,
  projectOfficeSavePdf,
} from "../../lib/native";
import type { ProjectOfficePdf } from "../../lib/projectOfficePreview";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { SegmentedControl } from "../../ui/SegmentedControl";
import { Spinner } from "../../ui/Spinner";
import { OfficePdfViewer } from "./OfficePdfViewer";
import { FileLines } from "./ProjectFileRichView";
import { PROJECT_FILE_LIMITS } from "./projectFileLimits";
import { type LoadedFile, isOfficePath, loadProjectFile, sameVersion } from "./projectFileLoad";
import { describeProjectError, METADATA_ONLY_REASON, SENSITIVE_CLASS_LABEL } from "./projectFileText";

type OfficeText = { status: "available"; document: ProjectDocumentRead } | { status: "unavailable"; message: string };
type Lane<T> = { status: "loading" } | { status: "ready"; value: T } | { status: "error"; message: string };
/** `idle` only after a cancel: opening a file starts the PDF, switching tabs never does. */
type PdfLane = { status: "idle" } | Lane<ProjectOfficePdf>;
type SaveState =
  { kind: "idle" } | { kind: "saving" } | { kind: "saved"; name: string } | { kind: "failed"; message: string };
type OfficeLoad =
  | { status: "loading"; key: string }
  | { status: "error"; key: string; message: string }
  | {
      status: "ready";
      key: string;
      baseline: ProjectFileStat;
      pdf: PdfLane;
      text: Lane<OfficeText>;
    };

const OFFICE_SOURCE_LIMIT = 32 * 1024 * 1024;
const LOAD_ATTEMPTS = 3;
const STALE_POLL_MS = 3_000;

function loadKey(projectId: string, path: string, reloadToken: number): string {
  return `${projectId}\u0000${path}\u0000${reloadToken}`;
}

function isReadableOfficeStat(stat: ProjectFileStat): stat is Extract<ProjectFileStat, { kind: "text" | "not_text" }> {
  return stat.kind === "text" || stat.kind === "not_text";
}

function refusalForStat(stat: ProjectFileStat): string | null {
  if (stat.kind === "sensitive_withheld") {
    return `This path is classified as ${SENSITIVE_CLASS_LABEL[stat.sensitive_class]}. xFalcon did not read it.`;
  }
  if (stat.kind === "too_large") {
    return `This file is ${formatBytes(stat.size_bytes)}, larger than the ${formatBytes(stat.limit_bytes)} project view limit.`;
  }
  if (!isReadableOfficeStat(stat)) return "This file could not be opened.";
  if (stat.size_bytes > OFFICE_SOURCE_LIMIT) {
    return `This Office file is ${formatBytes(stat.size_bytes)}. Files larger than ${formatBytes(OFFICE_SOURCE_LIMIT)} cannot be previewed.`;
  }
  if (stat.kind === "not_text" && stat.reason === "oversized") return METADATA_ONLY_REASON.oversized;
  return null;
}

function textFromFile(file: LoadedFile): OfficeText {
  if (file.mode === "document") return { status: "available", document: file.stat };
  if (file.mode === "opaque" && file.stat.kind === "sensitive_withheld") {
    return { status: "unavailable", message: "Text is unavailable because this path is classified as sensitive." };
  }
  return { status: "unavailable", message: "Text could not be extracted from this Office file." };
}

async function readOfficeText(projectId: string, path: string, signal: AbortSignal): Promise<OfficeText> {
  try {
    const file = await loadProjectFile(projectId, path, PROJECT_FILE_LIMITS.wholeLoadBytes, signal);
    return textFromFile(file);
  } catch (cause) {
    if (signal.aborted) throw cause;
    return { status: "unavailable", message: describeProjectError(cause, "Text could not be extracted.") };
  }
}

/** Read-only visual and extracted-text views for a trusted project Office file. */
export function ProjectOfficeViewer({ projectId, path }: { projectId: string; path: string }) {
  const [reloadToken, setReloadToken] = React.useState(0);
  const hasText = isOfficePath(path);
  const [view, setView] = React.useState<"preview" | "text">("preview");
  const [copyState, setCopyState] = React.useState<"idle" | "copied" | "failed">("idle");
  const [saveState, setSaveState] = React.useState<SaveState>({ kind: "idle" });
  const startPreviewRef = React.useRef<(() => void) | null>(null);
  const cancelPreviewRef = React.useRef<(() => void) | null>(null);
  const key = loadKey(projectId, path, reloadToken);
  const [result, setResult] = React.useState<OfficeLoad>({ status: "loading", key });
  const retriesRef = React.useRef(0);

  React.useEffect(() => {
    const controller = new AbortController();
    const signal = controller.signal;
    let active = true;
    let sourceValid = true;
    let timer = 0;
    let cleanupPoll = () => {};
    const textExtractable = isOfficePath(path);
    setResult({ status: "loading", key });
    setView("preview");
    setCopyState("idle");
    setSaveState({ kind: "idle" });

    const failAccess = (cause: unknown) => {
      if (!active || !sourceValid) return;
      sourceValid = false;
      cleanupPoll();
      controller.abort();
      setResult({
        status: "error",
        key,
        message: describeProjectError(cause, "Access to this Office file is no longer available."),
      });
    };

    const retryChangedSource = () => {
      if (!active || !sourceValid) return;
      sourceValid = false;
      cleanupPoll();
      controller.abort();
      if (retriesRef.current < LOAD_ATTEMPTS - 1) {
        retriesRef.current += 1;
        setResult({ status: "loading", key });
        setReloadToken((token) => token + 1);
      } else {
        setResult({
          status: "error",
          key,
          message: "This file kept changing while it was opened. Refresh to try again.",
        });
      }
    };

    const verifyVersion = async (baseline: ProjectFileStat): Promise<boolean> => {
      let currentStat: ProjectFileStat;
      try {
        currentStat = await projectFileStat(projectId, path);
      } catch (cause) {
        failAccess(cause);
        return false;
      }
      if (!active || signal.aborted || !sourceValid) return false;
      if (currentStat.version !== 1 || currentStat.path !== path) {
        failAccess(new Error("The file response did not match this path."));
        return false;
      }
      if (!sameVersion(baseline, currentStat)) {
        retryChangedSource();
        return false;
      }
      const refusal = refusalForStat(currentStat);
      if (refusal !== null) {
        failAccess(new Error(refusal));
        return false;
      }
      return true;
    };

    const completeTextLane = () => {
      if (!active || !sourceValid || signal.aborted) return;
      retriesRef.current = 0;
    };

    void (async () => {
      try {
        const baseline = await projectFileStat(projectId, path);
        if (!active || signal.aborted) return;
        if (baseline.version !== 1 || baseline.path !== path) {
          throw new Error("The file response did not match this path.");
        }
        const refusal = refusalForStat(baseline);
        if (refusal !== null) {
          setResult({ status: "error", key, message: refusal });
          return;
        }

        setResult({
          status: "ready",
          key,
          baseline,
          pdf: { status: "idle" },
          text: textExtractable
            ? { status: "loading" }
            : { status: "ready", value: { status: "unavailable", message: "Text is not shown for spreadsheets." } },
        });
        if (!textExtractable) retriesRef.current = 0;

        const checkCurrent = async () => verifyVersion(baseline);
        const updateLane = (field: "pdf" | "text", lane: PdfLane | Lane<OfficeText>) => {
          if (!active || signal.aborted || !sourceValid) return;
          setResult((currentResult) => {
            if (currentResult.status !== "ready" || currentResult.key !== key) return currentResult;
            return { ...currentResult, [field]: lane };
          });
        };

        let previewRunning = false;
        let previewAbort: AbortController | null = null;
        startPreviewRef.current = () => {
          if (previewRunning || !active || !sourceValid || signal.aborted) return;
          previewRunning = true;
          const run = new AbortController();
          previewAbort = run;
          const stop = () => run.abort();
          signal.addEventListener("abort", stop, { once: true });
          updateLane("pdf", { status: "loading" });
          void (async () => {
            try {
              const pdf = await projectOfficePreview(projectId, path, run.signal);
              if (run.signal.aborted) return;
              if (!(await checkCurrent())) return;
              updateLane("pdf", { status: "ready", value: pdf });
            } catch (cause) {
              if (!run.signal.aborted && active && sourceValid && (await checkCurrent())) {
                updateLane("pdf", {
                  status: "error",
                  message: describeProjectError(cause, "Visual preview is unavailable."),
                });
              }
            } finally {
              signal.removeEventListener("abort", stop);
              // A cancelled run ends after a newer one may have started; it must
              // not mark that newer run as finished.
              if (previewAbort === run) previewRunning = false;
            }
          })();
        };
        cancelPreviewRef.current = () => {
          previewAbort?.abort();
          previewRunning = false;
          updateLane("pdf", { status: "idle" });
        };
        // Preview is the default view, so the PDF starts when the file opens. A
        // finished PDF for an unchanged file comes straight back from the backend.
        startPreviewRef.current?.();

        if (textExtractable)
          void (async () => {
            try {
              const text = await readOfficeText(projectId, path, signal);
              if (signal.aborted || !active || !sourceValid) return;
              if (text.status === "available" && !sameVersion(baseline, text.document)) {
                retryChangedSource();
                return;
              }
              if (!(await checkCurrent())) return;
              updateLane("text", { status: "ready", value: text });
            } catch (cause) {
              if (!signal.aborted && active && sourceValid) {
                if (await checkCurrent()) {
                  updateLane("text", {
                    status: "error",
                    message: describeProjectError(cause, "Text could not be extracted."),
                  });
                }
              }
            } finally {
              completeTextLane();
            }
          })();

        const onFocus = () => {
          if (document.visibilityState === "visible") void checkCurrent();
        };
        const tick = () => {
          if (!active || !sourceValid || signal.aborted) return;
          if (document.visibilityState === "visible") void checkCurrent();
          timer = window.setTimeout(tick, STALE_POLL_MS);
        };
        timer = window.setTimeout(tick, STALE_POLL_MS);
        window.addEventListener("focus", onFocus);
        document.addEventListener("visibilitychange", onFocus);
        cleanupPoll = () => {
          window.clearTimeout(timer);
          window.removeEventListener("focus", onFocus);
          document.removeEventListener("visibilitychange", onFocus);
        };
      } catch (cause) {
        if (active && !signal.aborted)
          setResult({
            status: "error",
            key,
            message: describeProjectError(cause, "This Office file could not be opened."),
          });
      }
    })();

    return () => {
      active = false;
      sourceValid = false;
      startPreviewRef.current = null;
      cancelPreviewRef.current = null;
      cleanupPoll();
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [projectId, path, reloadToken, key]);

  React.useEffect(
    () => () => {
      retriesRef.current = 0;
    },
    [projectId, path],
  );

  const current = result.key === key ? result : ({ status: "loading", key } as const);
  const refresh = () => {
    retriesRef.current = 0;
    setReloadToken((token) => token + 1);
  };

  const copyText = async () => {
    if (
      current.status !== "ready" ||
      current.text.status !== "ready" ||
      current.text.value.status !== "available" ||
      current.text.value.document.text.length === 0
    ) {
      return;
    }
    try {
      await navigator.clipboard.writeText(current.text.value.document.text);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  };

  const finishSave = (result: DocumentExportResult) => {
    // A dismissed dialog is a decision, not a failure: say nothing.
    setSaveState(result.saved && result.file_name ? { kind: "saved", name: result.file_name } : { kind: "idle" });
  };
  const saveFailed = (cause: unknown, fallback: string) => {
    setSaveState({ kind: "failed", message: describeProjectError(cause, fallback) });
  };
  const savePdf = async () => {
    if (current.status !== "ready" || current.pdf.status !== "ready") return;
    setSaveState({ kind: "saving" });
    try {
      finishSave(await projectOfficeSavePdf(projectId, path, current.pdf.value.renderId));
    } catch (cause) {
      saveFailed(cause, "The PDF could not be saved.");
    }
  };
  const saveCopy = async () => {
    setSaveState({ kind: "saving" });
    try {
      finishSave(await projectOfficeSaveOriginal(projectId, path));
    } catch (cause) {
      saveFailed(cause, "The file could not be saved.");
    }
  };

  if (current.status === "loading") {
    return (
      <p className="type-body flex items-center gap-2 p-4 text-secondary" role="status">
        <Spinner decorative size={16} />
        Preparing Office preview…
      </p>
    );
  }

  if (current.status === "error") {
    return (
      <div className="p-4">
        <Alert title="This Office file could not be opened" tone="danger">
          <p>{current.message}</p>
          <Button className="mt-2" onClick={refresh} size="sm" variant="secondary">
            Refresh
          </Button>
        </Alert>
      </div>
    );
  }

  const textDocument =
    current.text.status === "ready" && current.text.value.status === "available" ? current.text.value.document : null;
  const textAvailable = textDocument !== null;
  const extractedText = textDocument?.text ?? "";

  const pdf = current.pdf;
  const saving = saveState.kind === "saving";

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 p-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        {hasText ? (
          <SegmentedControl
            aria-label="Office file view"
            onChange={(next) => {
              // Switching views only shows what is already there: it never
              // starts or restarts the conversion.
              setView(next);
              setSaveState({ kind: "idle" });
            }}
            options={[
              { value: "preview", label: "Preview" },
              { value: "text", label: "Text" },
            ]}
            value={view}
          />
        ) : (
          <span />
        )}
        <div className="flex items-center gap-2">
          {view === "text" && textAvailable && extractedText.length > 0 ? (
            <Button onClick={() => void copyText()} size="sm" variant="secondary">
              <Copy aria-hidden size={14} strokeWidth={1.8} />
              Copy text
            </Button>
          ) : null}
          {view === "preview" && pdf.status === "ready" ? (
            <Button disabled={saving} onClick={() => void savePdf()} size="sm" variant="secondary">
              <FileDown aria-hidden size={14} strokeWidth={1.8} />
              Save PDF
            </Button>
          ) : null}
          <Button disabled={saving} onClick={() => void saveCopy()} size="sm" variant="secondary">
            <Download aria-hidden size={14} strokeWidth={1.8} />
            Save a copy
          </Button>
          <Button onClick={refresh} size="sm" variant="secondary">
            <RotateCcw aria-hidden size={14} strokeWidth={1.8} />
            Refresh
          </Button>
        </div>
      </div>
      {saveState.kind === "saved" ? (
        <p className="type-caption text-muted" role="status">
          Saved {saveState.name}.
        </p>
      ) : null}
      {saveState.kind === "failed" ? (
        <Alert title="Could not save" tone="danger">
          {saveState.message}
        </Alert>
      ) : null}
      {view === "preview" ? (
        pdf.status === "idle" ? (
          <div className="flex flex-col items-start gap-2">
            <p className="type-body text-secondary">
              An Office program on this computer opens this file read-only in the background to make a PDF.
            </p>
            <Button onClick={() => startPreviewRef.current?.()} size="sm" variant="primary">
              Preview as PDF
            </Button>
          </div>
        ) : pdf.status === "loading" ? (
          <div className="flex items-center gap-3">
            <p className="type-body flex items-center gap-2 text-secondary" role="status">
              <Spinner decorative size={16} />
              Making the PDF preview…
            </p>
            <Button onClick={() => cancelPreviewRef.current?.()} size="sm" variant="secondary">
              Cancel
            </Button>
          </div>
        ) : pdf.status === "ready" ? (
          <div className="flex min-h-0 flex-1 flex-col">
            <OfficePdfViewer data={pdf.value.data} name={path.split(/[\\/]/).at(-1) ?? path} />
          </div>
        ) : (
          <Alert title="Visual preview unavailable" tone="warn">
            <p>{pdf.message}</p>
            <Button className="mt-2" onClick={() => startPreviewRef.current?.()} size="sm" variant="secondary">
              Try again
            </Button>
          </Alert>
        )
      ) : current.text.status === "loading" ? (
        <p className="type-body flex items-center gap-2 text-secondary" role="status">
          <Spinner decorative size={16} />
          Reading extracted text…
        </p>
      ) : current.text.status === "error" ? (
        <Alert title="Text is unavailable" tone="info">
          <p>{current.text.message}</p>
        </Alert>
      ) : textAvailable ? (
        extractedText.length > 0 ? (
          <div className="flex min-h-0 flex-1 flex-col gap-2 overflow-auto rounded-lg border border-default bg-surface p-4">
            <FileLines
              note="Images, layout and styling are not interpreted. Images are not OCR-scanned."
              path={path}
              text={extractedText}
            />
            {copyState === "copied" ? (
              <p className="type-caption text-muted" role="status">
                Text copied.
              </p>
            ) : null}
            {copyState === "failed" ? <Alert className="mt-2" title="Text could not be copied" tone="danger" /> : null}
          </div>
        ) : (
          <p className="type-body text-secondary" role="status">
            No text could be extracted from this Office file. Images are not OCR-scanned.
          </p>
        )
      ) : (
        <Alert title="Text is unavailable" tone="info">
          <p>{current.text.value.status === "unavailable" ? current.text.value.message : "Text is unavailable."}</p>
        </Alert>
      )}
    </div>
  );
}
