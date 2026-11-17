import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Database, Plus, RotateCcw, Search, X, Upload } from "lucide-react";
import type { DatasetView, StagedDatasetView, UploadProjectionV1 } from "../../lib/generated/contracts";
import { datasetDetail, datasetPick, uploadCancel, uploadRetry } from "../../lib/native";
import { Alert } from "../../ui/Alert";
import { Button } from "../../ui/Button";
import { EmptyState } from "../../ui/EmptyState";
import { IconButton } from "../../ui/IconButton";
import { Progress } from "../../ui/Progress";
import { SearchField } from "../../ui/SearchField";
import { Skeleton } from "../../ui/Skeleton";
import { toast } from "../../ui/toastStore";
import { applyUploadQueue, useUploadQueue } from "../uploads/uploadQueueStore";
import "./datasets.css";
import "../../ui/catalog.css";
import { DatasetTable } from "./DatasetTable";
import { DatasetUploadDialog } from "./DatasetUploadDialog";
import { DeleteDatasetDialog } from "./DeleteDatasetDialog";
import { SchemaDrawer } from "./SchemaDrawer";
import { clearDroppedRefusal, getDroppedDatasets, shiftDroppedDataset, subscribeDroppedDataset } from "./datasetsStore";
import {
  describeError,
  describeStepError,
  formatBytes,
  sortDatasets,
  type DatasetSortDirection,
  type DatasetSortKey,
} from "./datasetPresentation";
import { datasetErrorOf, useDatasets } from "./useDatasets";

/**
 * The Datasets destination: files a person keeps in the workspace, where they
 * become queryable data.
 *
 * Not the composer. A chat attachment is 10 MiB and belongs to one message; a
 * dataset is up to 2 GiB and outlives every conversation. They share the
 * transfer runtime and nothing else, which is why this reads the
 * persistent-dataset rows of the shared queue and the composer dock reads its
 * conversation's.
 *
 * Several datasets upload at once, so the transfers live here as well as in the
 * upload dialog: the dialog follows the one it started, and this list holds
 * every other one — including that one, once the dialog is closed.
 */
export interface DatasetsViewProps {
  /** The sidebar's "+" asked for an upload: open the file picker once. */
  pickRequested?: boolean;
  /** Called once the picker has been opened for that request. */
  onPickHandled?: () => void;
}

export function DatasetsView({ pickRequested = false, onPickHandled }: DatasetsViewProps = {}) {
  const { snapshot, refresh, revalidate, remove, download } = useDatasets();
  const uploads = useUploadQueue();
  const [query, setQuery] = useState("");
  const [sortKey, setSortKey] = useState<DatasetSortKey>("uploaded");
  const [sortDirection, setSortDirection] = useState<DatasetSortDirection>("desc");
  const [staged, setStaged] = useState<StagedDatasetView | null>(null);
  const [opened, setOpened] = useState<DatasetView | null>(null);
  const [pendingDelete, setPendingDelete] = useState<DatasetView | null>(null);
  const [picking, setPicking] = useState(false);
  /** The transfer the upload dialog is showing. One upload, one bar: while the
   * dialog has it, the list below does not. */
  const [dialogUpload, setDialogUpload] = useState<string | null>(null);
  const [saving, setSaving] = useState<string | null>(null);
  const [actingUploads, setActingUploads] = useState<Record<string, "cancel" | "retry">>({});
  const pickingRef = useRef(false);
  const savingRef = useRef(false);
  const actingRef = useRef(new Set<string>());

  const { status, datasets: all, uploadsEnabled, truncated, error } = snapshot;

  // A dropped file was staged by the backend before this ever ran, and the
  // shell brought the person here to claim it (`useDatasetDrops`). Consumed
  // here because this is where the confirmation step lives.
  const drops = useSyncExternalStore(subscribeDroppedDataset, getDroppedDatasets, getDroppedDatasets);
  useEffect(() => {
    if (drops.refusal === null) return;
    toast.error(describeStepError(drops.refusal.error, drops.refusal.stepIndex));
    clearDroppedRefusal();
  }, [drops.refusal]);
  useEffect(() => {
    // One confirmation step at a time, and the dialog holds its slot until it
    // is closed. The rest of the drop waits its turn rather than being
    // discarded — a staged file this never claims is one nobody can see or
    // release.
    if (staged !== null) return;
    const next = drops.pending[0];
    if (next === undefined) return;
    setStaged(next);
    shiftDroppedDataset();
  }, [drops.pending, staged]);

  const existingNames = useMemo(() => all.map((dataset) => dataset.display_name), [all]);

  const transfers = useMemo(() => uploads.filter((upload) => upload.purpose === "persistent_dataset"), [uploads]);

  /**
   * A finished transfer leaves the progress list and joins the table.
   *
   * The workspace only has the dataset once the transfer settles, so the row
   * cannot appear when the upload starts. Nothing was re-reading the catalog at
   * that moment either, which left a completed transfer sitting at "Uploaded"
   * above a table that did not contain it until the destination was reopened.
   *
   * A cancelled transfer leaves too — the person ended it. A failed one stays,
   * because it is the only place the reason and the retry live.
   *
   * The one the upload dialog is showing is left out for as long as the dialog
   * is open, and rejoins the list the moment it closes: a transfer someone
   * walked away from still needs a bar, and one they are watching does not need
   * two.
   */
  const shown = useMemo(
    () =>
      transfers.filter(
        (upload) => upload.state !== "ready" && upload.state !== "cancelled" && upload.id !== dialogUpload,
      ),
    [dialogUpload, transfers],
  );
  const settled = useRef(new Set<string>());
  useEffect(() => {
    const landed = transfers.filter((upload) => upload.state === "ready" && !settled.current.has(upload.id));
    if (landed.length === 0) return;
    for (const upload of landed) settled.current.add(upload.id);
    void revalidate();
  }, [transfers, revalidate]);

  const visible = useMemo(() => {
    const needle = query.trim().toLocaleLowerCase();
    const matching =
      needle === "" ? all : all.filter((dataset) => dataset.display_name.toLocaleLowerCase().includes(needle));
    return sortDatasets(matching, sortKey, sortDirection);
  }, [all, query, sortDirection, sortKey]);

  const pick = useCallback(async () => {
    if (pickingRef.current) return;
    pickingRef.current = true;
    setPicking(true);
    try {
      setStaged(await datasetPick());
    } catch (cause) {
      const { code } = datasetErrorOf(cause);
      // Dismissing the picker is a decision, not a failure.
      if (code !== "no_file_chosen") toast.error(describeError(code));
    } finally {
      pickingRef.current = false;
      setPicking(false);
    }
  }, []);

  useEffect(() => {
    if (!pickRequested) return;
    onPickHandled?.();
    void pick();
  }, [pick, pickRequested, onPickHandled]);

  /**
   * Saves a row's file without making the person open the drawer first.
   *
   * A dataset's files are only known from its detail, and the download command
   * addresses one file — so the id comes from a detail read here. A dataset that
   * holds more than one file has no single answer to "save it", so that opens
   * the drawer, where each file has its own control.
   */
  const save = async (dataset: DatasetView) => {
    if (savingRef.current) return;
    savingRef.current = true;
    setSaving(dataset.id);
    try {
      const detail = await datasetDetail(dataset.id);
      if (detail.dataset.id !== dataset.id) {
        toast.error("The dataset details did not match the selected dataset.");
        return;
      }
      const [only] = detail.files;
      if (only === undefined) {
        toast.error("That dataset has no file to save.");
        return;
      }
      if (detail.files.length > 1) {
        setOpened(dataset);
        return;
      }
      await download(dataset, only);
    } catch (cause) {
      toast.error(describeError(datasetErrorOf(cause).code));
    } finally {
      savingRef.current = false;
      setSaving(null);
    }
  };

  const actOnTransfer = async (uploadId: string, action: "cancel" | "retry") => {
    if (actingRef.current.has(uploadId)) return;
    actingRef.current.add(uploadId);
    setActingUploads((current) => ({ ...current, [uploadId]: action }));
    try {
      const queue = action === "retry" ? await uploadRetry(uploadId) : await uploadCancel(uploadId);
      applyUploadQueue(queue);
    } catch {
      toast.error(action === "retry" ? "Could not retry the upload." : "Could not cancel the upload.");
    } finally {
      actingRef.current.delete(uploadId);
      setActingUploads((current) => {
        const next = { ...current };
        delete next[uploadId];
        return next;
      });
    }
  };

  const sort = (key: DatasetSortKey) => {
    if (key === sortKey) {
      setSortDirection(sortDirection === "asc" ? "desc" : "asc");
      return;
    }
    setSortKey(key);
    setSortDirection(key === "name" ? "asc" : "desc");
  };

  return (
    <section aria-label="Datasets" className="datasets-workspace flex min-h-0 flex-1 flex-col">
      <div className="datasets-scroll">
        <header className="datasets-hero catalog-banner">
          <div className="datasets-hero__actions">
            <Button
              type="button"
              variant="primary"
              disabled={!uploadsEnabled}
              loading={picking}
              onClick={() => void pick()}
            >
              {picking ? null : <Plus aria-hidden size={14} />}
              {picking ? "Choosing…" : "Upload dataset"}
            </Button>
            <p className="type-caption text-secondary">CSV and Excel files for analysis</p>
          </div>
          <dl className="datasets-stats" aria-label="Dataset overview">
            <div>
              <dt>
                <Database size={15} aria-hidden />
                {truncated ? "Datasets shown" : "Datasets"}
              </dt>
              <dd>{status === "ready" ? all.length : "—"}</dd>
            </div>
            <div>
              <dt>
                <Upload size={15} aria-hidden />
                Active uploads
              </dt>
              <dd>
                {transfers.filter((u) => u.state !== "ready" && u.state !== "cancelled" && u.state !== "failed").length}
              </dd>
            </div>
          </dl>
        </header>

        <div className="datasets-toolbar">
          <div>
            <h2 className="type-heading font-semibold">Dataset library</h2>
            <p aria-live="polite" className="type-caption text-muted" role="status">
              {summary(status, all.length, visible.length, query)}
            </p>
          </div>
          {status === "ready" && all.length > 0 ? (
            <SearchField
              aria-label="Search datasets"
              className="datasets-search"
              placeholder="Search by name"
              value={query}
              onChange={(event) => setQuery(event.currentTarget.value)}
              onClear={() => setQuery("")}
            />
          ) : null}
        </div>
        <div className="datasets-content">
          {!uploadsEnabled ? (
            <Alert className="mx-4 mb-3" tone="warn" title="Uploads are turned off">
              An administrator deactivated this workspace's upload source. The datasets already here are still listed,
              but file uploads and downloads are blocked.
            </Alert>
          ) : null}

          {truncated ? (
            <Alert className="mx-4 mb-3" tone="info" title="Showing part of the list">
              This workspace holds more datasets than this list shows. Some older datasets may not appear here.
            </Alert>
          ) : null}

          {shown.length > 0 ? (
            <ul aria-label="Dataset transfers" className="mb-3 space-y-2 px-4">
              {shown.map((upload) => (
                <li
                  key={upload.id}
                  className="flex flex-wrap items-center gap-3 rounded-md border border-hairline bg-card px-3 py-2"
                >
                  <div className="min-w-60 flex-1">
                    <p className="type-body-sm truncate">{upload.display_name}</p>
                    <p className="type-caption text-muted">
                      {upload.safe_error ??
                        `${stateLabel(upload.state)} · ${formatBytes(Number(upload.bytes_sent))} of ${formatBytes(Number(upload.size))}`}
                    </p>
                    <Progress
                      className="mt-1"
                      label={`${upload.display_name} upload progress`}
                      value={progressPercent(Number(upload.bytes_sent), Number(upload.size))}
                      indeterminate={!determinate(upload.state)}
                    />
                  </div>
                  {upload.state === "failed" ? (
                    <>
                      <Button
                        type="button"
                        variant="secondary"
                        loading={actingUploads[upload.id] === "retry"}
                        disabled={actingUploads[upload.id] !== undefined}
                        onClick={() => void actOnTransfer(upload.id, "retry")}
                      >
                        {actingUploads[upload.id] === "retry" ? null : <RotateCcw aria-hidden size={14} />}
                        {actingUploads[upload.id] === "retry" ? "Retrying…" : "Try again"}
                      </Button>
                      <IconButton
                        aria-label={`Dismiss failed upload ${upload.display_name}`}
                        loading={actingUploads[upload.id] === "cancel"}
                        disabled={actingUploads[upload.id] !== undefined}
                        onClick={() => void actOnTransfer(upload.id, "cancel")}
                      >
                        <X aria-hidden size={15} />
                      </IconButton>
                    </>
                  ) : (
                    <IconButton
                      aria-label={`Cancel ${upload.display_name}`}
                      loading={actingUploads[upload.id] === "cancel"}
                      disabled={actingUploads[upload.id] !== undefined}
                      onClick={() => void actOnTransfer(upload.id, "cancel")}
                    >
                      <X aria-hidden size={15} />
                    </IconButton>
                  )}
                </li>
              ))}
            </ul>
          ) : null}

          {status === "loading" ? (
            <div className="space-y-2 px-4">
              <Skeleton className="h-10" />
              <Skeleton className="h-10" />
              <Skeleton className="h-10" />
            </div>
          ) : null}

          {/* Nothing else is on screen to align a left edge to, so these sit in
            the middle of the space they are explaining. */}
          {status === "error" ? (
            <div className="flex min-h-full items-center justify-center px-6 py-10">
              <EmptyState
                align="center"
                icon={<Database aria-hidden size={28} strokeWidth={1.6} />}
                title="Datasets could not be loaded"
                description={error === null ? "The workspace could not be reached." : describeError(error)}
                actions={
                  <Button type="button" variant="secondary" onClick={() => void refresh()}>
                    Try again
                  </Button>
                }
              />
            </div>
          ) : null}

          {status === "ready" && all.length === 0 ? (
            <div className="flex min-h-full items-center justify-center px-6 py-10">
              <EmptyState
                align="center"
                icon={<Database aria-hidden size={28} strokeWidth={1.6} />}
                title="No datasets yet"
                description="Upload a .csv or .xlsx file and the workspace can query it in any conversation. You can also drop a file on this window."
                actions={
                  uploadsEnabled ? (
                    <Button type="button" variant="primary" loading={picking} onClick={() => void pick()}>
                      {picking ? null : <Plus aria-hidden size={14} />}
                      {picking ? "Choosing…" : "Upload dataset"}
                    </Button>
                  ) : undefined
                }
              />
            </div>
          ) : null}

          {status === "ready" && all.length > 0 && visible.length === 0 ? (
            <div className="flex min-h-full items-center justify-center px-6 py-10">
              <EmptyState
                align="center"
                icon={<Search aria-hidden size={28} strokeWidth={1.6} />}
                title={`No dataset matches “${query.trim()}”`}
                description={
                  truncated
                    ? "No dataset in this partial list matches. Try another part of the name."
                    : "Every dataset in this workspace is listed here. Try part of a name."
                }
                actions={
                  // Not "Clear search": the search field already offers a control
                  // by that name, and two buttons with one accessible name leaves
                  // a screen-reader user unable to tell them apart.
                  <Button type="button" variant="secondary" onClick={() => setQuery("")}>
                    Show all datasets
                  </Button>
                }
              />
            </div>
          ) : null}

          {status === "ready" && visible.length > 0 ? (
            <DatasetTable
              datasets={visible}
              sortKey={sortKey}
              sortDirection={sortDirection}
              onSort={sort}
              onOpen={setOpened}
              onDownload={(dataset) => void save(dataset)}
              onDelete={setPendingDelete}
              busyId={saving}
              downloadsEnabled={uploadsEnabled}
            />
          ) : null}
        </div>
      </div>

      <DatasetUploadDialog
        staged={staged}
        existingNames={existingNames}
        onClose={() => {
          setStaged(null);
          setDialogUpload(null);
        }}
        onChooseAnother={() => {
          // The staged file goes with the dialog; the picker opens on top of
          // nothing, so a dismissed picker leaves the person where they were
          // rather than back on a file they had already rejected.
          setStaged(null);
          void pick();
        }}
        onTransferStarted={setDialogUpload}
      />
      <SchemaDrawer dataset={opened} onClose={() => setOpened(null)} onDownload={download} />
      <DeleteDatasetDialog
        dataset={pendingDelete}
        onCancel={() => setPendingDelete(null)}
        onConfirm={async (dataset) => {
          const problem = await remove(dataset);
          if (problem === null) {
            setPendingDelete(null);
            if (opened?.id === dataset.id) setOpened(null);
          }
          return problem;
        }}
      />
    </section>
  );
}

function summary(status: "loading" | "ready" | "error", total: number, shown: number, query: string): string {
  if (status === "loading") return "Loading datasets…";
  if (status === "error") return "Datasets unavailable";
  if (total === 0) return "No datasets";
  if (query.trim() !== "") return `${shown} of ${total} datasets`;
  return total === 1 ? "1 dataset" : `${total} datasets`;
}

function stateLabel(state: UploadProjectionV1["state"]): string {
  switch (state) {
    case "selected":
      return "Waiting";
    case "preparing":
      return "Preparing";
    case "uploading":
      return "Uploading";
    case "resigning":
      return "Renewing the transfer";
    case "finalizing":
      return "Finishing";
    case "ready":
      return "Uploaded";
    case "failed":
      return "Failed";
    case "cancelled":
      return "Cancelled";
    default:
      return unreachable(state);
  }
}

function unreachable(value: never): never {
  throw new Error(`unhandled case: ${String(value)}`);
}

function determinate(state: UploadProjectionV1["state"]): boolean {
  return state === "uploading" || state === "ready";
}

function progressPercent(sent: number, size: number): number {
  if (size <= 0) return 0;
  return Math.max(0, Math.min(100, Math.round((sent / size) * 100)));
}
