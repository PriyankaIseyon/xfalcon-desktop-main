import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { UpdateCheckResult } from "../../lib/generated/contracts";
import { appBootstrap, onUpdateDownloadProgress, updateCheck, updateInstall, updateRestart } from "../../lib/native";
import { getChatSnapshot } from "../chat/chatStore";
import { isRunPending } from "../chat/runState";
import { getUploadQueue } from "../uploads/uploadQueueStore";

const UPDATE_CHECK_EVENT = "xfalcon://update-check-requested";
const TRANSFERRING_UPLOAD_STATES = new Set(["selected", "preparing", "uploading", "resigning", "finalizing"]);

export type UpdateCheckStatus = "idle" | "checking" | "installing" | "ready_to_restart" | "up_to_date" | "error";

export interface UpdateStoreState {
  currentVersion: string;
  status: UpdateCheckStatus;
  availableVersion: string | null;
  releaseNotes: string | null;
  lastCheckTime: number | null;
  lastCheckSucceeded: boolean | null;
  errorMessage: string | null;
  downloadProgress: number | null;
  downloadedBytes: number;
  totalBytes: number | null;
  noticeDismissed: boolean;
  restarting: boolean;
}

let state: UpdateStoreState = {
  currentVersion: "…",
  status: "idle",
  availableVersion: null,
  releaseNotes: null,
  lastCheckTime: null,
  lastCheckSucceeded: null,
  errorMessage: null,
  downloadProgress: 0,
  downloadedBytes: 0,
  totalBytes: null,
  noticeDismissed: false,
  restarting: false,
};

let checkInFlight = false;
let stopScheduledChecks: (() => void) | null = null;
const listeners = new Set<() => void>();
let successFeedbackTimer: ReturnType<typeof setTimeout> | null = null;

function emitChange() {
  for (const listener of listeners) listener();
}

function commit(next: UpdateStoreState) {
  if (successFeedbackTimer !== null) clearTimeout(successFeedbackTimer);
  successFeedbackTimer = null;
  state = next;
  if (next.status === "up_to_date") {
    successFeedbackTimer = setTimeout(
      () => {
        if (state === next) commit({ ...state, status: "idle" });
      },
      Math.max(0, (next.lastCheckTime ?? Date.now()) + 5_000 - Date.now()),
    );
  }
  emitChange();
}

function updateWorkIsActive(): boolean {
  const chat = getChatSnapshot();
  return (
    chat.awaitingRun ||
    isRunPending(chat.runState) ||
    getUploadQueue().uploads.some((upload) => TRANSFERRING_UPLOAD_STATES.has(upload.state))
  );
}

export function getUpdateSnapshot(): Readonly<UpdateStoreState> {
  return state;
}

export function subscribeUpdate(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function setUpdateStoreForTests(next: Partial<UpdateStoreState>): void {
  commit({ ...state, ...next });
}

/**
 * Reads the running application's version.
 *
 * From `app_bootstrap` rather than the Tauri app plugin's `getVersion`, which
 * needs a core permission this build deliberately does not grant: the main
 * window's capability is pinned to the app's own commands plus four plugin
 * permissions, and `security::tests` fails closed on any addition. The bootstrap
 * command is already admitted, already reports `application_version`, and is
 * what the sign-in screen and the account menu display -- so this is one source
 * of truth rather than a second route to the same string.
 */
export async function initAppVersion(): Promise<string> {
  try {
    const { application_version: version } = await appBootstrap();
    commit({ ...state, currentVersion: version });
    return version;
  } catch {
    commit({ ...state, currentVersion: "Unknown" });
    return "Unknown";
  }
}

/** Subscribe to the native four-hour cadence and run the startup check. */
export async function startBackgroundUpdateChecks(): Promise<() => void> {
  if (stopScheduledChecks) return stopBackgroundUpdateChecks;
  const subscriptions = await Promise.allSettled([
    listen(UPDATE_CHECK_EVENT, () => void checkForUpdates(true)),
    onUpdateDownloadProgress((progress) => {
      if (state.status !== "installing") return;
      commit({
        ...state,
        downloadedBytes: progress.downloaded_bytes,
        totalBytes: progress.total_bytes,
        downloadProgress: progress.percent,
      });
    }),
  ]);
  const unlisteners: UnlistenFn[] = subscriptions.flatMap((result) =>
    result.status === "fulfilled" ? [result.value] : [],
  );
  stopScheduledChecks = () => {
    for (const unlisten of unlisteners) unlisten();
  };
  void checkForUpdates(true);
  return stopBackgroundUpdateChecks;
}

export function stopBackgroundUpdateChecks(): void {
  stopScheduledChecks?.();
  stopScheduledChecks = null;
}

export function dismissUpdateNotice(): void {
  commit({ ...state, noticeDismissed: true });
}

export async function checkForUpdates(silent = false): Promise<void> {
  if (state.restarting || checkInFlight || ["checking", "installing"].includes(state.status)) return;
  // A restart error stays on screen until the person acts. Only "Check again" refreshes it.
  if (state.status === "ready_to_restart" && silent && state.errorMessage) return;
  checkInFlight = true;
  try {
    await refreshUpdate(silent);
  } finally {
    checkInFlight = false;
  }
}

async function refreshUpdate(silent: boolean): Promise<void> {
  const previous = state;
  // A staged update stays on offer while a background check reads the manifest again.
  // The notice neither flickers nor disappears if that check fails.
  const keepStaged = silent && previous.status === "ready_to_restart";
  if (!keepStaged) commit({ ...state, status: "checking", errorMessage: null });
  if (state.currentVersion === "…") await initAppVersion();

  try {
    await adoptCheckResult(await updateCheck(), silent, previous);
  } catch (error) {
    const lastCheckTime = Date.now();
    const message = error instanceof Error ? error.message : String(error);
    commit({
      ...state,
      lastCheckTime,
      lastCheckSucceeded: false,
      status: keepStaged ? "ready_to_restart" : silent ? "idle" : "error",
      errorMessage: silent ? null : `Update check failed: ${message}`,
    });
  }
}

/**
 * Applies a manifest check to the store and downloads the version it names unless that
 * package is already staged. A newer release replaces a staged one, so only the latest
 * version is ever installed.
 */
async function adoptCheckResult(update: UpdateCheckResult, silent: boolean, previous: UpdateStoreState): Promise<void> {
  const lastCheckTime = Date.now();
  if (update.available_version) {
    // Another flow is already downloading. Starting a second download would only repeat it.
    if (!update.is_staged && state.status === "installing") return;
    commit({
      ...state,
      currentVersion: update.current_version,
      lastCheckTime,
      lastCheckSucceeded: true,
      status: update.is_staged ? "ready_to_restart" : "installing",
      downloadProgress: update.is_staged ? 100 : null,
      downloadedBytes: 0,
      totalBytes: null,
      noticeDismissed:
        silent && previous.availableVersion === update.available_version ? previous.noticeDismissed : false,
      availableVersion: update.available_version,
      releaseNotes: update.release_notes,
      errorMessage: null,
    });
    if (!update.is_staged) await downloadUpdate();
    return;
  }
  commit({
    ...state,
    currentVersion: update.current_version,
    lastCheckTime,
    lastCheckSucceeded: true,
    status: silent ? "idle" : "up_to_date",
    availableVersion: null,
    releaseNotes: null,
    errorMessage: null,
  });
}

/** Download and verify without interrupting work. Applying requires a separate action. */
async function downloadUpdate(): Promise<void> {
  try {
    await updateInstall();
    commit({ ...state, status: "ready_to_restart", downloadProgress: 100, noticeDismissed: false });
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    commit({ ...state, status: "error", errorMessage: `Update download failed: ${message}` });
  }
}

/**
 * Apply the verified update and relaunch only when explicitly requested.
 *
 * A newer release may have been published after this one was staged, and a background
 * check may not have seen it yet. The manifest is read again first. If it names another
 * version, that version is downloaded instead and nothing is installed, so one restart
 * always installs the latest release.
 */
export async function restartApp(): Promise<boolean> {
  if (state.status !== "ready_to_restart" || state.restarting) return false;
  if (updateWorkIsActive()) {
    commit({ ...state, errorMessage: "Finish the active run or transfer before restarting to update." });
    return false;
  }
  const offered = state;
  commit({ ...state, restarting: true, errorMessage: null });
  try {
    const latest = await readLatestUpdate();
    if (latest && !(latest.is_staged && latest.available_version === offered.availableVersion)) {
      commit({ ...state, restarting: false });
      await adoptCheckResult(latest, false, offered);
      return false;
    }
    await updateRestart();
    return true;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    commit({ ...state, restarting: false, errorMessage: `Failed to restart application: ${message}` });
    return false;
  }
}

/** Null when the manifest cannot be reached. The staged package is already verified, so it may still be applied. */
async function readLatestUpdate(): Promise<UpdateCheckResult | null> {
  try {
    return await updateCheck();
  } catch {
    return null;
  }
}
