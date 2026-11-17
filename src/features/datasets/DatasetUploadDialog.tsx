import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Check, Pencil, RotateCcw, Upload, X } from "lucide-react";
import type {
  DatasetPreview,
  DatasetRecipeStep,
  StagedDatasetView,
  UploadProjectionV1,
} from "../../lib/generated/contracts";
import { datasetPreview, datasetStageDiscard, datasetUploadStart, uploadCancel, uploadRetry } from "../../lib/native";
import { Button } from "../../ui/Button";
import { Alert } from "../../ui/Alert";
import { Field, Input } from "../../ui/Field";
import { IconButton } from "../../ui/IconButton";
import { Modal } from "../../ui/Modal";
import { Progress } from "../../ui/Progress";
import { Select } from "../../ui/Select";
import { Spinner } from "../../ui/Spinner";
import { toast } from "../../ui/toastStore";
import { applyUploadQueue, useUploadQueue } from "../uploads/uploadQueueStore";
import { DatasetPreviewTable, PreviewMeta, PreviewTruncationNotice } from "./DatasetPreviewTable";
import { WrangleEditor } from "./WrangleEditor";
import { describeError, describeStepError, formatBytes } from "./datasetPresentation";
import { datasetErrorOf } from "./useDatasets";

/**
 * One file, from picked to landed: confirm → transferring → complete.
 *
 * The preview is re-fetched from the backend whenever the worksheet, the header
 * row, or the cleanup recipe changes, because the backend is what applies the
 * recipe — the rows on screen are the rows that will upload. Typing into a
 * step's value would otherwise mean a round trip per keystroke, so the request
 * is debounced; nothing else here is.
 *
 * The dialog stays open through the transfer and reports its end, mirroring
 * xfalcon-web: the person who pressed Upload watches it land where they pressed
 * it, rather than being returned to a list to hunt for a bar. It reads the
 * transfer from the shared queue rather than holding progress of its own, so
 * what it shows and what the destination shows can never disagree.
 *
 * Closing it mid-transfer does not end the transfer — that is what the queue is
 * for, and the destination picks the bar back up. Only Cancel ends one.
 */

/** How long to wait after a change before asking for a new preview. Long
 * enough to swallow a burst of keystrokes, short enough that the rows feel
 * attached to the control that changed them. */
const PREVIEW_DEBOUNCE_MS = 250;

/** How long a finished upload stays on screen before the dialog closes itself.
 * Long enough to read the result, short enough that nobody has to dismiss a
 * dialog whose work is done. */
const AUTO_CLOSE_MS = 8000;

type Phase = "confirm" | "transferring" | "finished" | "failed";

export interface DatasetUploadDialogProps {
  staged: StagedDatasetView | null;
  /** Names already in the catalog, for warning about a collision before it
   * happens. Two datasets may share a name; nobody means to. */
  existingNames: string[];
  onClose: () => void;
  onChooseAnother: () => void;
  /** The transfer this dialog is now showing. The destination hides that one
   * row from its own progress list while the dialog owns it, so one upload is
   * never two bars. */
  onTransferStarted: (uploadId: string) => void;
}

export function DatasetUploadDialog({
  staged,
  existingNames,
  onClose,
  onChooseAnother,
  onTransferStarted,
}: DatasetUploadDialogProps) {
  const [name, setName] = useState("");
  const [worksheetIndex, setWorksheetIndex] = useState(0);
  const [headerRowIndex, setHeaderRowIndex] = useState<number | null>(null);
  const [steps, setSteps] = useState<DatasetRecipeStep[]>([]);
  const [preview, setPreview] = useState<DatasetPreview | null>(null);
  const [sourceColumns, setSourceColumns] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const [actionProblem, setActionProblem] = useState<string | null>(null);
  const [invalidStep, setInvalidStep] = useState<{ index: number; message: string } | null>(null);
  const [confirmAction, setConfirmAction] = useState<"discard" | "upload" | null>(null);
  const [transferAction, setTransferAction] = useState<"cancel" | "retry" | null>(null);
  const [uploadId, setUploadId] = useState<string | null>(null);
  const [closingIn, setClosingIn] = useState(Math.ceil(AUTO_CLOSE_MS / 1000));
  /**
   * What a person typed into the fallback worksheet and header-row boxes.
   *
   * Held as text, not derived from the index. A box whose value is
   * `String(index + 1)` snaps back to "1" the moment it is emptied, so it
   * cannot be cleared and retyped — the next digit lands beside the old one and
   * "2" becomes "12".
   */
  const [typed, setTyped] = useState({ worksheet: "1", header: "1" });
  const requestRef = useRef(0);
  const drainRef = useRef<HTMLSpanElement | null>(null);
  const confirmActionRef = useRef(false);
  const transferActionRef = useRef(false);

  const uploads = useUploadQueue();
  const transfer = useMemo<UploadProjectionV1 | null>(
    () => (uploadId === null ? null : (uploads.find((upload) => upload.id === uploadId) ?? null)),
    [uploadId, uploads],
  );

  // A newly staged file resets everything: the previous file's header row and
  // cleanup steps name columns this one may not have. Keyed on the object
  // rather than on its id, because one staged file is one object — a new one
  // only ever arrives from a new pick or a new drop.
  useEffect(() => {
    // Without the extension. It is the dataset's name, not the file's, and
    // `sales.csv` reads as a file sitting in a list of datasets.
    setName(staged === null ? "" : withoutExtension(staged.display_name));
    setWorksheetIndex(0);
    setHeaderRowIndex(null);
    setSteps([]);
    setPreview(null);
    setSourceColumns([]);
    setProblem(null);
    setActionProblem(null);
    setInvalidStep(null);
    setUploadId(null);
    setConfirmAction(null);
    setTransferAction(null);
    confirmActionRef.current = false;
    transferActionRef.current = false;
    setTyped({ worksheet: "1", header: "1" });
  }, [staged]);

  const stagedId = staged?.id ?? null;
  const recipe = useMemo(() => ({ steps }), [steps]);

  useEffect(() => {
    const request = ++requestRef.current;
    // Nothing left to preview once the transfer owns the file: the staged entry
    // was released when the upload was admitted, and the rows on screen are
    // already the rows being sent.
    if (stagedId === null || uploadId !== null) {
      setLoading(false);
      return;
    }
    setLoading(true);
    const timer = setTimeout(() => {
      void datasetPreview({
        stagedId,
        worksheetIndex,
        headerRowIndex,
        recipe: steps.length === 0 ? null : recipe,
      })
        .then((next) => {
          if (request !== requestRef.current) return;
          setPreview(next);
          if (steps.length === 0 && next.unavailable === null) {
            setSourceColumns(next.columns.map((column) => column.name));
          }
          setProblem(null);
          setInvalidStep(null);
        })
        .catch((cause) => {
          if (request !== requestRef.current) return;
          const { code, stepIndex } = datasetErrorOf(cause);
          if (code === "recipe_invalid_step" && stepIndex !== null) {
            setProblem(null);
            setInvalidStep({ index: stepIndex, message: describeStepError(code, stepIndex) });
          } else {
            setInvalidStep(null);
            setProblem(describeError(code));
          }
        })
        .finally(() => {
          if (request === requestRef.current) setLoading(false);
        });
    }, PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [stagedId, uploadId, worksheetIndex, headerRowIndex, recipe, steps.length]);

  const phase: Phase = ((): Phase => {
    if (transfer === null) return "confirm";
    switch (transfer.state) {
      case "failed":
        return "failed";
      case "ready":
        return "finished";
      default:
        return "transferring";
    }
  })();

  /**
   * The close this dialog runs on its own, kept out of the countdown's
   * dependencies.
   *
   * The parent passes a fresh closure every render, and a countdown that
   * restarted on every render would push its own deadline away forever.
   */
  const closeRef = useRef(onClose);
  useEffect(() => {
    closeRef.current = onClose;
  }, [onClose]);

  // A transfer the person cancelled, or one the queue no longer holds, has
  // nothing left to show. The dialog goes with it rather than sitting on an
  // empty frame.
  const gone = uploadId !== null && (transfer === null || transfer.state === "cancelled");
  useEffect(() => {
    if (gone) closeRef.current();
  }, [gone]);

  useEffect(() => {
    if (phase !== "finished") return;
    let remaining = AUTO_CLOSE_MS;
    let deadline = Date.now() + remaining;
    let frame = 0;
    let shown = -1;

    const tick = () => {
      const left = Math.max(0, deadline - Date.now());
      if (drainRef.current !== null) {
        drainRef.current.style.transform = `scaleX(${left / AUTO_CLOSE_MS})`;
      }
      const seconds = Math.ceil(left / 1000);
      if (seconds !== shown) {
        shown = seconds;
        setClosingIn(seconds);
      }
      if (left <= 0) {
        closeRef.current();
        return;
      }
      frame = requestAnimationFrame(tick);
    };
    const start = () => {
      deadline = Date.now() + remaining;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(tick);
    };
    // A dialog nobody can see is not a dialog anybody read. The clock holds
    // where it stopped rather than expiring behind a hidden window.
    const onVisibility = () => {
      if (document.hidden) {
        remaining = Math.max(0, deadline - Date.now());
        cancelAnimationFrame(frame);
        return;
      }
      start();
    };

    start();
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      cancelAnimationFrame(frame);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, [phase]);

  const discardStage = useCallback(
    async (after: () => void) => {
      if (stagedId === null || confirmActionRef.current) return;
      confirmActionRef.current = true;
      setConfirmAction("discard");
      setActionProblem(null);
      try {
        await datasetStageDiscard(stagedId);
        after();
      } catch (cause) {
        setActionProblem(describeError(datasetErrorOf(cause).code));
      } finally {
        confirmActionRef.current = false;
        setConfirmAction(null);
      }
    },
    [stagedId],
  );

  const dismiss = useCallback(() => {
    // Not while the transfer is being started. `dataset_upload_start` is
    // reading that staged entry, and releasing it mid-call would refuse an
    // upload the person already confirmed — the dialog's own Cancel is disabled
    // for the same moment, and closing the modal must not be the way around it.
    if (confirmActionRef.current) return;
    // Once a transfer exists the staged entry is already released, and the
    // transfer holds the only copy of the path. Leave both alone: closing here
    // hands the bar back to the destination, it does not end the upload.
    if (uploadId === null && stagedId !== null) {
      void discardStage(onClose);
      return;
    }
    onClose();
  }, [discardStage, onClose, stagedId, uploadId]);

  const upload = useCallback(async () => {
    if (staged === null || confirmActionRef.current) return;
    confirmActionRef.current = true;
    setConfirmAction("upload");
    setActionProblem(null);
    try {
      const shaped = steps.length > 0;
      const result = await datasetUploadStart({
        staged_id: staged.id,
        display_name: name.trim(),
        // A recipe is materialized locally before upload. The backend needs
        // the same worksheet and header choices that produced this preview so
        // it cleans the reviewed data, then removes both indices from the
        // later workspace ingest request for the resulting CSV.
        worksheet_index: staged.worksheets.length > 0 ? worksheetIndex : null,
        header_row_index: headerRowIndex ?? preview?.header_row_index ?? null,
        recipe: shaped ? { steps } : null,
      });
      // The response is a newer snapshot than the store holds, and the record
      // this dialog is about to follow has to be in it before the next render
      // looks for it — a queue event racing behind the response would leave the
      // dialog pointing at a transfer nobody can find.
      applyUploadQueue(result.queue);
      const admitted = result.queue.uploads.find((candidate) => candidate.id === result.upload_id);
      if (admitted === undefined) {
        // The command says it admitted this exact transfer. If its attached
        // snapshot is malformed, hand ownership to the shared queue rather
        // than leaving a released stage on a confirm button that cannot work.
        setUploadId(result.upload_id);
        onTransferStarted(result.upload_id);
        return;
      }
      setClosingIn(Math.ceil(AUTO_CLOSE_MS / 1000));
      setUploadId(admitted.id);
      onTransferStarted(admitted.id);
    } catch (cause) {
      const { code, stepIndex } = datasetErrorOf(cause);
      if (code === "recipe_invalid_step" && stepIndex !== null) {
        setInvalidStep({ index: stepIndex, message: describeStepError(code, stepIndex) });
      } else {
        setActionProblem(describeError(code));
      }
    } finally {
      confirmActionRef.current = false;
      setConfirmAction(null);
    }
  }, [headerRowIndex, name, onTransferStarted, preview, staged, steps, worksheetIndex]);

  const actOnTransfer = useCallback(
    async (action: "cancel" | "retry") => {
      if (transfer === null || transferActionRef.current) return;
      transferActionRef.current = true;
      setTransferAction(action);
      try {
        const queue = action === "retry" ? await uploadRetry(transfer.id) : await uploadCancel(transfer.id);
        applyUploadQueue(queue);
      } catch {
        toast.error(action === "retry" ? "Could not retry the upload." : "Could not cancel the upload.");
      } finally {
        transferActionRef.current = false;
        setTransferAction(null);
      }
    },
    [transfer],
  );

  const collides = useMemo(() => {
    const target = name.trim().toLocaleLowerCase();
    if (target === "") return false;
    return existingNames.some((existing) => existing.trim().toLocaleLowerCase() === target);
  }, [existingNames, name]);

  if (staged === null) return null;

  const columns = preview?.columns.map((column) => column.name) ?? [];
  const candidates = preview?.header_candidates ?? [];
  const chosenHeaderRow = headerRowIndex ?? preview?.header_row_index ?? 0;
  const confirming = phase === "confirm";
  const readable = confirming && preview !== null && preview.unavailable === null;
  const fallbackInvalid =
    confirming &&
    preview?.unavailable !== null &&
    preview?.unavailable !== "not_delimited" &&
    ((staged.worksheets.length > 0 && oneBasedToIndex(typed.worksheet) === null) ||
      oneBasedToIndex(typed.header) === null);
  // Nothing to confirm about a table with no columns, and DA would take the
  // file's own first row instead of the one on screen.
  const uploadable =
    preview !== null &&
    !loading &&
    problem === null &&
    invalidStep === null &&
    !fallbackInvalid &&
    name.trim() !== "" &&
    (!readable || preview.columns.length > 0);
  const sent = transfer === null ? 0 : Number(transfer.bytes_sent);
  const total = transfer === null ? Number(staged.size) : Number(transfer.size);
  const percent = total > 0 ? Math.min(100, Math.round((sent / total) * 100)) : 0;

  return (
    <Modal
      open
      size="xl"
      onClose={dismiss}
      dismissible={confirmAction === null}
      title={heading(phase, readable)}
      description={subheading(phase, readable)}
      footer={
        <>
          {phase === "confirm" ? (
            <>
              <Button
                type="button"
                variant="secondary"
                loading={confirmAction === "discard"}
                disabled={confirmAction !== null}
                onClick={() => void discardStage(onChooseAnother)}
              >
                {confirmAction === "discard" ? "Releasing…" : "Choose different file"}
              </Button>
              <Button
                type="button"
                variant="primary"
                loading={confirmAction === "upload"}
                disabled={!uploadable || confirmAction !== null}
                onClick={() => void upload()}
              >
                {confirmAction === "upload" ? null : <Upload aria-hidden size={14} />}
                {confirmAction === "upload" ? "Starting…" : "Upload dataset"}
              </Button>
            </>
          ) : null}

          {phase === "transferring" && transfer !== null ? (
            <>
              <Button
                type="button"
                variant="secondary"
                loading={transferAction === "cancel"}
                disabled={transferAction !== null}
                onClick={() => void actOnTransfer("cancel")}
              >
                {transferAction === "cancel" ? "Cancelling…" : "Cancel upload"}
              </Button>
              <Button type="button" variant="primary" loading>
                {transferLabel(transfer.state)}
              </Button>
            </>
          ) : null}

          {phase === "finished" ? (
            <div className="flex w-full flex-wrap items-center justify-end gap-3">
              <span aria-live="polite" className="type-caption tabular-nums text-muted">
                Closing in {closingIn}s
              </span>
              <Button
                type="button"
                variant="primary"
                autoFocus
                className="relative overflow-hidden"
                onClick={() => closeRef.current()}
              >
                <span className="relative z-10">Close now</span>
                <span
                  aria-hidden
                  ref={drainRef}
                  className="pointer-events-none absolute inset-x-0 bottom-0 h-0.5 origin-left bg-accent-ink/40 will-change-transform motion-reduce:hidden"
                  style={{ transform: "scaleX(1)" }}
                />
              </Button>
            </div>
          ) : null}

          {phase === "failed" && transfer !== null ? (
            <>
              <Button type="button" variant="secondary" onClick={dismiss}>
                Close
              </Button>
              <Button
                type="button"
                variant="primary"
                loading={transferAction === "retry"}
                disabled={transferAction !== null}
                onClick={() => void actOnTransfer("retry")}
              >
                {transferAction === "retry" ? null : <RotateCcw aria-hidden size={14} />}
                {transferAction === "retry" ? "Retrying…" : "Try again"}
              </Button>
            </>
          ) : null}
        </>
      }
    >
      <div className="dataset-upload-body space-y-4">
        <div className="flex items-center gap-3 rounded-md border border-hairline bg-surface p-3">
          <span
            aria-hidden
            className={`type-micro flex h-9 w-11 shrink-0 items-center justify-center rounded font-semibold uppercase tracking-wide ${formatBadge(staged.format)}`}
          >
            {staged.format}
          </span>
          <div className="min-w-0 flex-1">
            <p className="truncate font-mono text-xs font-medium text-foreground">{staged.display_name}</p>
            <p className="type-caption tabular-nums text-muted">{formatBytes(staged.size)}</p>
          </div>
          {phase === "confirm" ? (
            <IconButton
              aria-label="Remove file"
              variant="danger"
              size="sm"
              loading={confirmAction === "discard"}
              disabled={confirmAction !== null}
              onClick={dismiss}
            >
              <X aria-hidden size={14} />
            </IconButton>
          ) : null}
          {phase === "finished" ? (
            <span aria-hidden className="flex h-6 w-6 items-center justify-center rounded-pill bg-success text-card">
              <Check size={14} />
            </span>
          ) : null}
        </div>

        {phase === "confirm" && collides ? (
          <Alert tone="warn">
            A dataset named “{name.trim()}” is already in this workspace. Uploading is still allowed, and both will
            carry the same name — rename this one to tell them apart.
          </Alert>
        ) : null}

        {phase === "confirm" && actionProblem !== null ? <Alert tone="danger">{actionProblem}</Alert> : null}

        {phase === "confirm" ? (
          <Field
            label={
              <span className="inline-flex items-center gap-1.5">
                Dataset name
                <Pencil aria-hidden className="text-muted" size={14} />
              </span>
            }
            htmlFor="dataset-name"
          >
            <Input
              id="dataset-name"
              aria-invalid={collides || undefined}
              value={name}
              onChange={(event) => setName(event.currentTarget.value)}
              placeholder="my_dataset"
            />
          </Field>
        ) : null}

        {readable && staged.worksheets.length > 1 ? (
          <Field
            label="Worksheet"
            hint={`${staged.worksheets.length} worksheets — the one uploaded is the one shown`}
            htmlFor="dataset-worksheet"
          >
            <Select
              value={String(worksheetIndex)}
              onChange={(value) => {
                setWorksheetIndex(Number.parseInt(value, 10));
                // A different worksheet has different columns, so the header
                // row and every step that names a column start over.
                setHeaderRowIndex(null);
                setSteps([]);
              }}
              options={staged.worksheets.map((sheet, index) => ({
                value: String(index),
                label: sheet,
              }))}
            />
          </Field>
        ) : null}

        {confirming && candidates.length > 1 ? (
          <Field label="Header row" hint="Skip the title rows above the real header. Rows above it are not uploaded.">
            <Select
              value={String(chosenHeaderRow)}
              onChange={(value) => {
                setHeaderRowIndex(Number.parseInt(value, 10));
                setSteps([]);
              }}
              options={candidates.map((row, index) => ({
                value: String(index),
                label: `Row ${index + 1} — ${rowLabel(row)}`,
              }))}
            />
          </Field>
        ) : null}

        {confirming && problem !== null ? <Alert tone="danger">{problem}</Alert> : null}

        {confirming && loading && preview === null ? (
          <div className="flex items-center gap-2 py-6">
            <Spinner decorative size={16} />
            <span className="type-caption text-muted">Reading the file…</span>
          </div>
        ) : null}

        {readable ? (
          <div className="space-y-2">
            <PreviewMeta preview={preview} />
            <DatasetPreviewTable preview={preview} />
            <PreviewTruncationNotice preview={preview} />
          </div>
        ) : null}

        {confirming && preview !== null && preview.unavailable !== null ? (
          <Alert tone="info" title="No preview for this file">
            <p>{unavailableMessage(preview)}</p>
            {preview.unavailable === "not_delimited" ? null : (
              <div className="mt-3 flex flex-wrap gap-3">
                {staged.worksheets.length > 0 ? (
                  <Field
                    className="min-w-40 flex-1"
                    label="Worksheet number"
                    htmlFor="dataset-worksheet-fallback"
                    error={oneBasedToIndex(typed.worksheet) === null ? "Enter a positive whole number." : undefined}
                  >
                    <Input
                      id="dataset-worksheet-fallback"
                      inputMode="numeric"
                      value={typed.worksheet}
                      onChange={(event) => {
                        const raw = event.currentTarget.value;
                        setTyped((current) => ({ ...current, worksheet: raw }));
                        const index = oneBasedToIndex(raw);
                        if (index !== null) setWorksheetIndex(index);
                      }}
                    />
                  </Field>
                ) : null}
                <Field
                  className="min-w-40 flex-1"
                  label="Header row number"
                  htmlFor="dataset-header-fallback"
                  error={oneBasedToIndex(typed.header) === null ? "Enter a positive whole number." : undefined}
                >
                  <Input
                    id="dataset-header-fallback"
                    inputMode="numeric"
                    value={typed.header}
                    onChange={(event) => {
                      const raw = event.currentTarget.value;
                      setTyped((current) => ({ ...current, header: raw }));
                      const index = oneBasedToIndex(raw);
                      if (index !== null) setHeaderRowIndex(index);
                    }}
                  />
                </Field>
              </div>
            )}
          </Alert>
        ) : null}

        {readable && preview.recipe_available ? (
          <WrangleEditor
            columns={sourceColumns.length > 0 ? sourceColumns : columns}
            steps={steps}
            onChange={setSteps}
            invalidStep={invalidStep}
            disabledReason={null}
          />
        ) : null}

        {readable && !preview.recipe_available ? (
          <Alert tone="info" title="No cleanup for a file this size">
            This file is {formatBytes(staged.size)}, which is more than the cleanup steps can read here. It uploads as
            it is, and the worksheet and header row above still apply. Clean it after upload, or save a smaller extract
            to use the steps.
          </Alert>
        ) : null}

        {transfer !== null && phase !== "failed" ? (
          <div>
            <div aria-live="polite" className="type-caption mb-1.5 flex justify-between text-muted">
              <span>{transferLabel(transfer.state)}</span>
              {determinate(transfer.state) ? <span className="tabular-nums">{percent}%</span> : null}
            </div>
            <Progress
              className="h-2"
              indeterminate={!determinate(transfer.state)}
              label={phase === "finished" ? "Upload complete" : "Upload progress"}
              tone={phase === "finished" ? "success" : "accent"}
              value={percent}
            />
            {determinate(transfer.state) ? (
              <p className="type-caption mt-1 tabular-nums text-muted">
                {formatBytes(sent)} of {formatBytes(total)}
              </p>
            ) : null}
          </div>
        ) : null}

        {phase === "finished" ? (
          <Alert tone="success" title="Upload complete">
            Ready to query in chat.
          </Alert>
        ) : null}

        {phase === "failed" && transfer !== null ? (
          <Alert tone="danger" title="The upload did not finish">
            {transfer.safe_error ?? "The transfer stopped before the workspace had the whole file."}
          </Alert>
        ) : null}
      </div>
    </Modal>
  );
}

/** Which dialog this is, at each step of the one job it does. */
function heading(phase: Phase, readable: boolean): string {
  switch (phase) {
    case "confirm":
      return readable ? "Confirm upload" : "New dataset";
    case "transferring":
      return "Uploading dataset";
    case "finished":
      return "Upload complete";
    case "failed":
      return "Upload failed";
    default:
      return unreachable(phase);
  }
}

/** What is being asked of the reader, or what just happened without them. */
function subheading(phase: Phase, readable: boolean): string | undefined {
  switch (phase) {
    case "confirm":
      return readable
        ? "Review the first rows, then upload to the workspace"
        : "A .csv or .xlsx file becomes data the workspace can query";
    case "transferring":
      // Worth saying: on the web this dialog owns the transfer and closing it
      // ends the upload. Here it does not.
      return "This keeps going if you close this dialog";
    case "finished":
      return undefined;
    case "failed":
      return undefined;
    default:
      return unreachable(phase);
  }
}

/** What the transfer is doing, in the words the destination's list uses. */
function transferLabel(state: UploadProjectionV1["state"]): string {
  switch (state) {
    case "selected":
      return "Waiting to start…";
    case "preparing":
      return "Preparing…";
    case "uploading":
      return "Uploading…";
    case "resigning":
      return "Renewing the transfer…";
    case "finalizing":
      return "Processing…";
    case "ready":
      return "Upload complete";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    default:
      return unreachable(state);
  }
}

/**
 * Whether a percentage means anything yet.
 *
 * Bytes only move in `uploading`; before and after it the work is the
 * workspace's, and a bar frozen at 4% reads as a stall rather than as progress
 * nobody can count.
 */
function determinate(state: UploadProjectionV1["state"]): boolean {
  return state === "uploading" || state === "ready";
}

function unreachable(value: never): never {
  throw new Error(`unhandled case: ${String(value)}`);
}

/** A header candidate, as one line. */
function rowLabel(row: (string | null)[]): string {
  const cells = row.filter((cell): cell is string => cell !== null && cell.trim() !== "");
  if (cells.length === 0) return "(blank row)";
  return cells
    .slice(0, 4)
    .map((cell) => cell.trim().slice(0, 24))
    .join(" · ");
}

/**
 * The file's own extension, dropped from the dataset's default name.
 *
 * Only the two this destination accepts: a name that happens to end in
 * something else is a name, not an extension.
 */
function withoutExtension(fileName: string): string {
  return fileName.replace(/\.(csv|xlsx)$/i, "");
}

/** A number a person typed, counting from one, as an index counting from zero. */
function oneBasedToIndex(raw: string): number | null {
  if (!/^\d+$/.test(raw)) return null;
  const parsed = Number(raw);
  if (!Number.isSafeInteger(parsed) || parsed < 1 || parsed > 4_294_967_296) return null;
  return parsed - 1;
}

/**
 * The badge behind a file's kind. Two accepted formats, two tints — enough to
 * tell them apart at a glance without inventing a colour vocabulary for a
 * three-letter label.
 */
function formatBadge(format: string): string {
  return format === "csv" ? "bg-success-bg text-success" : "bg-accent-muted text-accent-hover";
}

function unavailableMessage(preview: DatasetPreview): string {
  switch (preview.unavailable) {
    case "file_too_large":
      return "This file is too large to read here. It still uploads, and the workspace reads it on its own — set the worksheet and header row below if its data does not start on the first worksheet's first row.";
    case "not_delimited":
      return "This file does not look like a table of rows and columns. Check the export before uploading.";
    case "unreadable":
      return "This file's rows could not be read here. It still uploads, and the workspace reads it on its own — set the worksheet and header row below if its data does not start on the first worksheet's first row.";
    default:
      return "";
  }
}
