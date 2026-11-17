import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { FileDown, Printer } from "lucide-react";
import { Button } from "../../ui/Button";
import { Alert } from "../../ui/Alert";
import { Checkbox } from "../../ui/Checkbox";
import { Modal } from "../../ui/Modal";
import { SegmentedControl } from "../../ui/SegmentedControl";
import { toast } from "../../ui/toastStore";
import { cn } from "../../lib/cn";
import { documentExportSave } from "../../lib/native";
import { PrintDocument, type PrintOutcome } from "./PrintDocument";
import {
  buildMarkdownDocument,
  exportableTurns,
  suggestedFileName,
  type ExportSelection,
  type ExportTurn,
} from "./exportDocument";

export type ExportFormat = "pdf" | "markdown";

type PrintSession = {
  operation: number;
  metadata: { title: string; generatedAtMs: number; reasoning: readonly string[] };
  selection: ExportSelection;
  turns: readonly ExportTurn[];
};

export type ExportDocumentDialogProps = {
  open: boolean;
  /** The conversation's title, used for the heading and the file name. */
  title: string;
  turns: readonly ExportTurn[];
  /** The live run's reasoning, oldest first. Empty for a reloaded conversation. */
  reasoning?: readonly string[];
  onClose: () => void;
};

/** Longest single-line preview of a turn in the checkbox list. Beyond this the
 * row would wrap into a paragraph and the list would stop being scannable. */
const TURN_PREVIEW_CHARS = 96;

const FORMATS = [
  { value: "pdf" as const, label: "PDF", icon: <Printer aria-hidden size={14} /> },
  { value: "markdown" as const, label: "Markdown", icon: <FileDown aria-hidden size={14} /> },
];

/**
 * "Export Document" — choose a format, choose what goes in, save it
 * (spec: lean-report-export).
 *
 * Everything is selected when the dialog opens, which is the answer to "export
 * this conversation" that a reader who does not touch the list expects. The
 * two payload toggles are the exception and start OFF: the spec requires an
 * export to omit internal tool payloads unless a reader asks, and reasoning is
 * the model's working, not its answer.
 *
 * PDF is written by the webview print engine through the same native save
 * dialog as Markdown. `window.print()` is not used on Windows: WebView2's
 * print preview blocks the renderer until it is dismissed, and that preview
 * can fail to appear, which left this dialog's Close and Cancel as no-ops.
 */
export function ExportDocumentDialog({ open, title, turns, reasoning = [], onClose }: ExportDocumentDialogProps) {
  const available = useMemo(() => exportableTurns(turns), [turns]);
  const [format, setFormat] = useState<ExportFormat>("pdf");
  const [chosenIds, setChosenIds] = useState<ReadonlySet<string>>(new Set());
  const [includeToolCalls, setIncludeToolCalls] = useState(false);
  const [includeReasoning, setIncludeReasoning] = useState(false);
  const [busy, setBusy] = useState(false);
  /** Frozen when a print starts, so the sheet and its header agree on one time
   * however many times the tree re-renders while the dialog is open. */
  const [printSession, setPrintSession] = useState<PrintSession | null>(null);
  const [error, setError] = useState<string | null>(null);
  const operation = useRef(0);
  // React state disables the controls on the next render. This ref closes the
  // same-tick gap before that render, so one gesture can admit only one native
  // save dialog or PDF job.
  const operationActiveRef = useRef(false);
  // Updated during render, before passive effect cleanup. This closes the
  // narrow race where a native save settles after a parent has closed the
  // controlled dialog but before the close effect has retired its operation.
  const openRef = useRef(open);
  openRef.current = open;
  const printing = printSession !== null;
  const operationActive = busy || printing;

  /**
   * Reset on the open transition, and only there.
   *
   * A run that settles while this dialog is up changes the transcript. Resetting
   * on that would wipe the reader's selection under their cursor, so the
   * transition is what this watches rather than the turns themselves — new turns
   * simply arrive unselected, which the "n of m" count above the list shows.
   */
  const wasOpen = useRef(false);
  useEffect(() => {
    if (open && !wasOpen.current) {
      setChosenIds(new Set(exportableTurns(turns).map((turn) => turn.id)));
      setIncludeToolCalls(false);
      setIncludeReasoning(false);
      setError(null);
      setBusy(false);
      setPrintSession(null);
      operationActiveRef.current = false;
      operation.current += 1;
    } else if (!open && wasOpen.current) {
      operation.current += 1;
      operationActiveRef.current = false;
      setBusy(false);
      setPrintSession(null);
    }
    wasOpen.current = open;
  }, [open, turns]);

  useEffect(
    () => () => {
      operation.current += 1;
    },
    [],
  );

  const selection: ExportSelection = useMemo(
    () => ({ turnIds: chosenIds, includeToolCalls, includeReasoning }),
    [chosenIds, includeToolCalls, includeReasoning],
  );

  const chosenCount = available.filter((turn) => chosenIds.has(turn.id)).length;
  const allChosen = available.length > 0 && chosenCount === available.length;
  // Clearing the selection is a valid export. The document names that choice
  // instead of producing an empty file (spec: lean-report-export).
  const canExport = available.length > 0 && !operationActive;
  const hasToolCalls = available.some((turn) => turn.toolCalls.length > 0);
  const formatOptions = useMemo(
    () => FORMATS.map((option) => ({ ...option, disabled: operationActive })),
    [operationActive],
  );

  const toggleTurn = (id: string) => {
    setChosenIds((previous) => {
      const next = new Set(previous);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const toggleAll = () => {
    setChosenIds(allChosen ? new Set() : new Set(available.map((turn) => turn.id)));
  };

  const saveMarkdown = async () => {
    if (!canExport || operationActiveRef.current) return;
    operationActiveRef.current = true;
    const currentOperation = ++operation.current;
    setBusy(true);
    setError(null);
    try {
      const body = buildMarkdownDocument(available, selection, {
        title,
        generatedAtMs: Date.now(),
        reasoning,
      });
      const result = await documentExportSave(suggestedFileName(title), body, "markdown");
      if (operation.current !== currentOperation || !openRef.current) return;
      // A dismissed save dialog is an answer, not a fault: the dialog stays
      // open on the selection they already made, and nothing is announced.
      if (!result.saved) return;
      toast.success(`Saved ${result.file_name ?? "the document"}.`);
      onClose();
    } catch (cause) {
      if (operation.current === currentOperation && openRef.current) setError(exportFailureText(cause));
    } finally {
      if (operation.current === currentOperation && openRef.current) {
        operationActiveRef.current = false;
        setBusy(false);
      }
    }
  };

  const onPrintDone = useCallback(
    (currentOperation: number, outcome: PrintOutcome, fileName?: string | null) => {
      if (operation.current !== currentOperation || !openRef.current) return;
      operationActiveRef.current = false;
      setPrintSession(null);
      if (outcome === "printed") {
        toast.success(`Saved ${fileName ?? "the document"}.`);
        onClose();
        return;
      }
      if (outcome === "cancelled") return;
      setFormat("markdown");
      setError("This platform could not write a PDF. Export as Markdown instead — the selection is unchanged.");
    },
    [onClose],
  );

  const dismiss = useCallback(() => {
    operation.current += 1;
    operationActiveRef.current = false;
    setBusy(false);
    setPrintSession(null);
    onClose();
  }, [onClose]);

  return (
    <>
      <Modal
        open={open}
        size="lg"
        title="Export document"
        onClose={dismiss}
        footer={
          <div className="flex items-center justify-end gap-2">
            <Button type="button" variant="secondary" onClick={dismiss}>
              Cancel
            </Button>
            {format === "pdf" ? (
              <Button
                disabled={!canExport}
                loading={printing}
                type="button"
                variant="primary"
                onClick={() => {
                  if (!canExport || operationActiveRef.current) return;
                  operationActiveRef.current = true;
                  const currentOperation = ++operation.current;
                  setError(null);
                  setPrintSession({
                    operation: currentOperation,
                    metadata: { title, generatedAtMs: Date.now(), reasoning: [...reasoning] },
                    selection: {
                      turnIds: new Set(chosenIds),
                      includeToolCalls,
                      includeReasoning,
                    },
                    turns: [...available],
                  });
                }}
              >
                Save PDF
              </Button>
            ) : (
              <Button
                disabled={!canExport}
                loading={busy}
                type="button"
                variant="primary"
                onClick={() => void saveMarkdown()}
              >
                Save Markdown
              </Button>
            )}
          </div>
        }
      >
        <div className="flex min-h-0 flex-col gap-4">
          <SegmentedControl
            aria-label="Export format"
            className="self-start"
            options={formatOptions}
            value={format}
            onChange={(next) => {
              setFormat(next as ExportFormat);
              setError(null);
            }}
          />

          <p className="type-caption text-secondary">
            {format === "pdf"
              ? "Writes a PDF wherever you point the save dialog."
              : "Writes a GitHub-flavoured Markdown file, tables and code fences intact."}
          </p>

          {available.length === 0 ? (
            <p className="type-caption text-muted">This conversation has nothing to export yet.</p>
          ) : (
            <fieldset className="min-h-0 border-0 p-0">
              <legend className="flex w-full items-center justify-between gap-2 pb-2">
                <span className="type-label font-medium text-foreground">
                  Turns ({chosenCount} of {available.length})
                </span>
                <Button disabled={operationActive} size="sm" type="button" variant="ghost" onClick={toggleAll}>
                  {allChosen ? "Clear all" : "Select all"}
                </Button>
              </legend>
              <ul className="m-0 flex max-h-64 list-none flex-col gap-1 overflow-y-auto p-0">
                {available.map((turn) => (
                  <li className="min-w-0" key={turn.id}>
                    <label
                      className={cn(
                        "flex items-start gap-2.5 rounded-lg px-2 py-1.5 transition-colors",
                        operationActive ? "cursor-default" : "cursor-pointer hover:bg-hover",
                      )}
                    >
                      <Checkbox
                        checked={chosenIds.has(turn.id)}
                        className="mt-0.5"
                        disabled={operationActive}
                        onChange={() => toggleTurn(turn.id)}
                      />
                      <span className="min-w-0 flex-1">
                        <span className="type-micro block text-faint uppercase tracking-wide">
                          {turn.role === "user" ? "Question" : "Answer"}
                        </span>
                        <span className="type-caption block truncate text-secondary">{preview(turn.text)}</span>
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
            </fieldset>
          )}

          <div className="flex flex-col gap-2 border-t border-hairline pt-3">
            <label className={cn("flex items-start gap-2.5", operationActive ? "cursor-default" : "cursor-pointer")}>
              <Checkbox
                checked={includeToolCalls}
                className="mt-0.5"
                disabled={operationActive || !hasToolCalls}
                onChange={(event) => setIncludeToolCalls(event.currentTarget.checked)}
              />
              <span className="min-w-0">
                <span className="type-label block text-foreground">Include tool calls</span>
                <span className="type-caption block text-muted">
                  {hasToolCalls
                    ? "Adds each tool's name, arguments, and result. These can contain paths and raw data."
                    : "No turn in this conversation called a tool."}
                </span>
              </span>
            </label>

            <label className={cn("flex items-start gap-2.5", operationActive ? "cursor-default" : "cursor-pointer")}>
              <Checkbox
                checked={includeReasoning}
                className="mt-0.5"
                disabled={operationActive || reasoning.length === 0}
                onChange={(event) => setIncludeReasoning(event.currentTarget.checked)}
              />
              <span className="min-w-0">
                <span className="type-label block text-foreground">Include reasoning</span>
                <span className="type-caption block text-muted">
                  {reasoning.length === 0
                    ? "Reasoning is never saved with a conversation, so there is none to export here."
                    : "Adds the thinking from the run still on screen."}
                </span>
              </span>
            </label>
          </div>

          {error !== null ? (
            <Alert title="Export failed" tone="danger">
              <p className="type-caption">{error}</p>
            </Alert>
          ) : null}
        </div>
      </Modal>

      {printSession !== null ? (
        <PrintDocument
          metadata={printSession.metadata}
          onDone={(outcome, fileName) => onPrintDone(printSession.operation, outcome, fileName)}
          selection={printSession.selection}
          turns={printSession.turns}
        />
      ) : null}
    </>
  );
}

function exportFailureText(cause: unknown): string {
  const message = (cause instanceof Error ? cause.message : typeof cause === "string" ? cause : "").toLowerCase();
  if (message.includes("too large")) return "This document is too large to export.";
  if (message.includes("nothing to export")) return "There is nothing to export.";
  if (message.includes("save dialog")) return "The save dialog could not be opened. Try again.";
  if (message.includes("location") || message.includes("write")) {
    return "That location could not be written to. Choose another location and try again.";
  }
  return "The document could not be saved. Try again.";
}

function preview(text: string): string {
  const flattened = text.replace(/\s+/g, " ").trim();
  return flattened.length > TURN_PREVIEW_CHARS ? `${flattened.slice(0, TURN_PREVIEW_CHARS).trimEnd()}…` : flattened;
}
