import { beforeEach, describe, expect, it } from "vitest";
import type { DatasetView } from "../../lib/generated/contracts";
import type { StagedDatasetView } from "../../lib/generated/contracts";
import {
  applyDatasetList,
  applyDroppedRefusal,
  clearDroppedRefusal,
  enqueueDroppedDataset,
  getDatasetsSnapshot,
  getDroppedDatasets,
  removeDataset,
  shiftDroppedDataset,
  resetDatasetsStore,
  setDatasetsError,
  subscribeDatasets,
  subscribeDroppedDataset,
} from "./datasetsStore";

function dataset(id: string, name: string): DatasetView {
  return {
    version: 1,
    id,
    display_name: name,
    status: "ready",
    created_at_ms: 1_783_500_000_000n,
    updated_at_ms: 1_783_500_000_000n,
  };
}

beforeEach(resetDatasetsStore);

describe("the catalog store", () => {
  it("treats uploads as enabled until a list says otherwise", () => {
    // The upload control must not flash disabled on a healthy workspace while
    // the first list is in flight.
    expect(getDatasetsSnapshot().uploadsEnabled).toBe(true);
    applyDatasetList([], false, false);
    expect(getDatasetsSnapshot().uploadsEnabled).toBe(false);
  });

  it("carries the truncation flag through", () => {
    applyDatasetList([dataset("d-1", "Sales")], true, true);
    expect(getDatasetsSnapshot().truncated).toBe(true);
  });

  it("removes only the confirmed dataset", () => {
    const before = [dataset("d-1", "Sales"), dataset("d-2", "Inventory")];
    applyDatasetList(before, false, true);

    removeDataset("d-1");
    expect(getDatasetsSnapshot()).toEqual({
      status: "ready",
      datasets: [before[1]],
      uploadsEnabled: false,
      truncated: true,
      error: null,
    });
  });

  it("keeps the rows it had when a refresh fails", () => {
    applyDatasetList([dataset("d-1", "Sales")], true, false);
    setDatasetsError("network");
    const snapshot = getDatasetsSnapshot();
    expect(snapshot.status).toBe("error");
    expect(snapshot.error).toBe("network");
    expect(snapshot.datasets).toHaveLength(1);
  });

  it("notifies its subscribers and stops when they leave", () => {
    let calls = 0;
    const unsubscribe = subscribeDatasets(() => {
      calls += 1;
    });
    applyDatasetList([], true, false);
    expect(calls).toBe(1);
    unsubscribe();
    applyDatasetList([], true, false);
    expect(calls).toBe(1);
  });
});

describe("the dropped-file store", () => {
  const dropped = (id: string): StagedDatasetView => ({
    version: 1,
    id,
    display_name: `${id}.csv`,
    size: 1024,
    format: "csv",
    worksheets: [],
  });

  it("queues every staged file, because the confirmation step holds one", () => {
    // One gesture can drop eight files. A single slot would overwrite seven,
    // leaving them staged in the backend with nothing able to show or release
    // them — each holding a staging slot until the app exits.
    enqueueDroppedDataset(dropped("first"));
    enqueueDroppedDataset(dropped("second"));
    expect(getDroppedDatasets().pending.map((file) => file.id)).toEqual(["first", "second"]);

    shiftDroppedDataset();
    expect(getDroppedDatasets().pending.map((file) => file.id)).toEqual(["second"]);
    shiftDroppedDataset();
    expect(getDroppedDatasets().pending).toEqual([]);
    // Shifting an empty queue is what a destination mounting with nothing
    // dropped does, and it must not notify.
    let calls = 0;
    const unsubscribe = subscribeDroppedDataset(() => {
      calls += 1;
    });
    shiftDroppedDataset();
    expect(calls).toBe(0);
    unsubscribe();
  });

  it("keeps only the newest refusal, and does not queue it behind staged files", () => {
    // A refusal is a sentence, not a step. Eight of them for one dropped folder
    // is noise, and one that waited behind three confirmations would arrive long
    // after the gesture that caused it.
    enqueueDroppedDataset(dropped("first"));
    applyDroppedRefusal({ error: "unsupported_format", stepIndex: null });
    applyDroppedRefusal({ error: "unreadable", stepIndex: null });
    expect(getDroppedDatasets().refusal).toEqual({ error: "unreadable", stepIndex: null });
    expect(getDroppedDatasets().pending).toHaveLength(1);

    clearDroppedRefusal();
    expect(getDroppedDatasets().refusal).toBeNull();
    expect(getDroppedDatasets().pending).toHaveLength(1);
  });

  it("is emptied with the catalog", () => {
    enqueueDroppedDataset(dropped("first"));
    applyDroppedRefusal({ error: "unreadable", stepIndex: null });
    resetDatasetsStore();
    expect(getDroppedDatasets()).toEqual({ pending: [], refusal: null });
  });

  it("does not wake the catalog's subscribers", () => {
    // A drop can land at any moment. If it re-rendered the list, every drop
    // anywhere in the app would redraw the catalog.
    let calls = 0;
    const unsubscribe = subscribeDatasets(() => {
      calls += 1;
    });
    applyDroppedRefusal({ error: "unreadable", stepIndex: null });
    enqueueDroppedDataset(dropped("first"));
    expect(calls).toBe(0);
    unsubscribe();
  });
});
