import { useCallback, useEffect, useRef } from "react";
import { useSyncExternalStore } from "react";
import type { DatasetErrorCode, DatasetFileView, DatasetView } from "../../lib/generated/contracts";
import { datasetDelete, datasetDownload, datasetList } from "../../lib/native";
import { toast } from "../../ui/toastStore";
import {
  applyDatasetList,
  getDatasetsSnapshot,
  removeDataset,
  setDatasetsError,
  setDatasetsLoading,
  subscribeDatasets,
} from "./datasetsStore";
import { describeError } from "./datasetPresentation";

/**
 * A refused command's code, from whatever the boundary rejected with.
 *
 * A Tauri command that fails with a typed error rejects with the serialized
 * value, not with an `Error`, so this reads the shape rather than a message. A
 * rejection that is not one of ours — the boundary itself being unavailable —
 * reads as a network failure, which is what it is.
 */
export function datasetErrorOf(cause: unknown): {
  code: DatasetErrorCode;
  stepIndex: number | null;
} {
  if (typeof cause === "object" && cause !== null && "code" in cause) {
    const record = cause as { code: unknown; step_index?: unknown };
    if (isDatasetErrorCode(record.code)) {
      return {
        code: record.code,
        stepIndex:
          typeof record.step_index === "number" && Number.isInteger(record.step_index) && record.step_index >= 0
            ? record.step_index
            : null,
      };
    }
    // The backend deliberately drops server prose at this boundary. An
    // unfamiliar code is still a refusal, but it must not become an empty
    // sentence or leak its raw value into the surface.
    if (typeof record.code === "string") return { code: "server_refused", stepIndex: null };
  }
  return { code: "network", stepIndex: null };
}

const DATASET_ERROR_CODES = {
  auth_required: true,
  not_configured: true,
  uploads_disabled: true,
  update_pending: true,
  stage_not_found: true,
  stage_limit_reached: true,
  no_file_chosen: true,
  unsupported_format: true,
  content_mismatch: true,
  encrypted_file: true,
  empty_file: true,
  file_too_large: true,
  unreadable: true,
  recipe_too_long: true,
  recipe_invalid_step: true,
  recipe_file_too_large: true,
  not_found: true,
  server_refused: true,
  network: true,
  storage: true,
} satisfies Record<DatasetErrorCode, true>;

function isDatasetErrorCode(value: unknown): value is DatasetErrorCode {
  return typeof value === "string" && Object.hasOwn(DATASET_ERROR_CODES, value);
}

/**
 * The Datasets destination's catalog. A delete changes the local projection
 * only after the workspace confirms it, so a failed request needs no rollback
 * and cannot overwrite a newer background catalog result.
 */
export function useDatasets() {
  const snapshot = useSyncExternalStore(subscribeDatasets, getDatasetsSnapshot, getDatasetsSnapshot);
  const listRequest = useRef(0);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      listRequest.current += 1;
    };
  }, []);

  const load = useCallback(async (blank: boolean) => {
    const request = ++listRequest.current;
    if (blank) setDatasetsLoading();
    try {
      const result = await datasetList();
      if (!mounted.current || request !== listRequest.current) return;
      applyDatasetList(result.datasets, result.uploads_enabled, result.truncated);
    } catch (cause) {
      if (!mounted.current || request !== listRequest.current) return;
      setDatasetsError(datasetErrorOf(cause).code);
    }
  }, []);

  const refresh = useCallback(() => load(true), [load]);

  /**
   * Re-reads the catalog without emptying the surface first.
   *
   * For a refresh nobody asked for — a transfer just finished and its row is
   * now in the workspace. `refresh` blanks the table to skeletons, which is
   * right when a person pressed something and wrong when the list on screen is
   * still perfectly good. A failure keeps that list and says so once, rather
   * than replacing what a person can read with an error about a request they
   * did not make.
   */
  const revalidate = useCallback(async () => {
    const request = ++listRequest.current;
    try {
      const result = await datasetList();
      if (!mounted.current || request !== listRequest.current) return;
      applyDatasetList(result.datasets, result.uploads_enabled, result.truncated);
    } catch {
      if (!mounted.current || request !== listRequest.current) return;
      toast.error("The dataset list could not be refreshed.");
    }
  }, []);

  // Loaded when the destination mounts, which is when it opens: an account that
  // never opens Datasets should never ask the workspace for its catalog. The
  // drop listener is the shell's, not this hook's — see `useDatasetDrops`.
  // A revisit keeps the cached list on screen while the read runs behind it, and
  // a failed read is an error state like any other load. Only a first visit, or
  // one after a failed load, has nothing good to show and needs the skeleton.
  useEffect(() => {
    void load(getDatasetsSnapshot().status !== "ready");
  }, [load]);

  const remove = useCallback(async (dataset: DatasetView) => {
    // A list read that began before this write cannot replace the confirmed
    // result after it arrives.
    listRequest.current += 1;
    try {
      await datasetDelete(dataset.id);
      // This also invalidates any revalidation that began while deletion was
      // pending and could have read the row before the backend removed it.
      listRequest.current += 1;
      if (!mounted.current) return "The dataset was not deleted.";
      removeDataset(dataset.id);
      toast.success("Dataset deleted");
      return null;
    } catch (cause) {
      if (!mounted.current) return "The dataset was not deleted.";
      return describeError(datasetErrorOf(cause).code);
    }
  }, []);

  const download = useCallback(async (dataset: DatasetView, file: DatasetFileView) => {
    try {
      const result = await datasetDownload(dataset.id, file.file_id);
      if (result.saved) {
        toast.success(`Saved ${result.file_name ?? "file"}`);
      }
      return result.saved;
    } catch (cause) {
      toast.error(describeError(datasetErrorOf(cause).code));
      return false;
    }
  }, []);

  return { snapshot, refresh, revalidate, remove, download };
}
