import type { DatasetErrorCode, DatasetView, StagedDatasetView } from "../../lib/generated/contracts";

/**
 * Module store behind the Datasets destination, split from its hook the way
 * `reportsStore` is split from `useReports`: the mutators are importable from
 * plain modules and tests without dragging a component in.
 *
 * Two stores, not one. The catalog is a list the server owns; a dropped file is
 * an intent that arrives from the backend at a moment nobody asked for, and the
 * shell must not re-render its whole list every time one lands.
 */
export type DatasetsStoreSnapshot = {
  status: "loading" | "ready" | "error";
  datasets: DatasetView[];
  /**
   * False only when an administrator deactivated the workspace's upload source.
   * True until the first list lands, so the upload control does not flash
   * disabled on a healthy workspace.
   */
  uploadsEnabled: boolean;
  /** The page cap stopped the walk before the server's reported total. */
  truncated: boolean;
  error: DatasetErrorCode | null;
};

type Listener = () => void;

const initialState: DatasetsStoreSnapshot = {
  status: "loading",
  datasets: [],
  uploadsEnabled: true,
  truncated: false,
  error: null,
};

let state: DatasetsStoreSnapshot = { ...initialState };
const listeners = new Set<Listener>();

function commit(next: DatasetsStoreSnapshot) {
  state = next;
  for (const listener of listeners) listener();
}

export function subscribeDatasets(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getDatasetsSnapshot(): DatasetsStoreSnapshot {
  return state;
}

export function resetDatasetsStore() {
  commit({ ...initialState });
  commitDrops(noDrops);
}

export function setDatasetsLoading() {
  commit({ ...state, status: "loading", error: null });
}

export function applyDatasetList(datasets: DatasetView[], uploadsEnabled: boolean, truncated: boolean) {
  commit({ status: "ready", datasets, uploadsEnabled, truncated, error: null });
}

export function setDatasetsError(error: DatasetErrorCode) {
  commit({ ...state, status: "error", error });
}

export function removeDataset(datasetId: string) {
  commit({ ...state, datasets: state.datasets.filter((dataset) => dataset.id !== datasetId) });
}

/**
 * "A file was dropped", in its own tiny store.
 *
 * The drop is handled in the backend, so it can land while any destination is
 * on screen. The destination reads this only while it is the one showing, which
 * is a rendering decision — the backend has already staged or refused the file
 * either way, and nothing the webview believes changes that.
 *
 * Separate listener set from the list above, so a drop does not re-render the
 * catalog and a list refresh does not re-run the drop effect.
 *
 * Staged files queue; a refusal does not.
 *
 * One gesture can drop eight files, and the confirmation step holds one at a
 * time. A single slot would let each arrival overwrite the last, leaving seven
 * files staged in the backend that nobody can see, confirm, or discard — each
 * holding one of the eight staging slots until the app exits. So they wait
 * their turn. A refusal is a sentence, not a step: the newest replaces the
 * previous one rather than making a person read eight of them, and it is shown
 * as soon as it arrives instead of queueing behind files still being confirmed.
 */
export type DroppedRefusal = { error: DatasetErrorCode; stepIndex: number | null };

export type DropsSnapshot = {
  /** Staged files waiting for the confirmation step, oldest first. */
  pending: StagedDatasetView[];
  refusal: DroppedRefusal | null;
};

const noDrops: DropsSnapshot = { pending: [], refusal: null };

let drops: DropsSnapshot = noDrops;
const dropListeners = new Set<Listener>();

export function subscribeDroppedDataset(listener: Listener): () => void {
  dropListeners.add(listener);
  return () => dropListeners.delete(listener);
}

export function getDroppedDatasets(): DropsSnapshot {
  return drops;
}

function commitDrops(next: DropsSnapshot) {
  drops = next;
  for (const listener of dropListeners) listener();
}

/** A staged file joins the back of the queue. */
export function enqueueDroppedDataset(staged: StagedDatasetView) {
  commitDrops({ ...drops, pending: [...drops.pending, staged] });
}

/** The confirmation step took the head of the queue. */
export function shiftDroppedDataset() {
  if (drops.pending.length === 0) return;
  commitDrops({ ...drops, pending: drops.pending.slice(1) });
}

export function applyDroppedRefusal(refusal: DroppedRefusal) {
  commitDrops({ ...drops, refusal });
}

export function clearDroppedRefusal() {
  if (drops.refusal === null) return;
  commitDrops({ ...drops, refusal: null });
}
