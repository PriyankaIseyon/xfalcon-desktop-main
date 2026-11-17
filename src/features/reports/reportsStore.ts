import type { ReportProjectOrigin, ReportView } from "../../lib/generated/contracts";
import { clearChatReportDocuments } from "./chatReportDocuments";
import { clearStoredThumbnails } from "./storedThumbnails";

/**
 * Module store behind the Reports destination, split from the hook the same way
 * `chatStore` is split from `useChat`: the mutators are importable from plain
 * modules and tests without dragging a component in.
 *
 * The store holds DA's list, not a copy the device owns. A failed read is an
 * error state with no reports, never a previous list shown as current.
 *
 * The list's order is maintained HERE rather than refetched after every action,
 * so an optimistic pin lands where the next list would put it.
 */
export type ReportsStoreSnapshot = {
  status: "loading" | "ready" | "error";
  reports: ReportView[];
  /** Which project each linked report was created in, from the last list. Held
   * beside the reports, not on them: a pin, rename or refresh returns a new
   * report row and changes no origin. */
  projectOrigins: ReportProjectOrigin[];
};

type Listener = () => void;

const initialState: ReportsStoreSnapshot = {
  status: "loading",
  reports: [],
  projectOrigins: [],
};

let state: ReportsStoreSnapshot = { ...initialState };
const listeners = new Set<Listener>();
let currentListRequest: symbol | null = null;
/** The owner `currentListRequest` displaced when it began, kept so that cancelling
 * the current request can give the catalog back to it. */
let displacedListRequest: symbol | null = null;

function commit(next: ReportsStoreSnapshot) {
  state = next;
  for (const listener of listeners) listener();
}

/** Pinned first, then most recently updated, then by id ascending. This is the
 * single ordering rule for the list. The pin time is not known per reader, so
 * "most recently pinned leads" no longer exists. Rust returns DA's order, and
 * this function replaces it on every write, so an optimistic update lands where
 * the next list would put it. Equal keys keep their input order. */
export function sortReports(reports: ReportView[]): ReportView[] {
  return [...reports].sort((left, right) => {
    const leftPinned = left.pinned === true;
    const rightPinned = right.pinned === true;
    if (leftPinned !== rightPinned) return leftPinned ? -1 : 1;
    if (left.updated_at_ms !== right.updated_at_ms) {
      return left.updated_at_ms > right.updated_at_ms ? -1 : 1;
    }
    return left.id.localeCompare(right.id);
  });
}

export function subscribeReports(listener: Listener): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getReportsSnapshot(): ReportsStoreSnapshot {
  return state;
}

export function resetReportsStore() {
  invalidateReportListRequests();
  clearStoredThumbnails();
  clearChatReportDocuments();
  commit({ ...initialState });
  commitPendingOpen(null);
}

/** One revision owner for every producer of the shared report catalog.
 *
 * A request that only prefetched and left must not strand the read it displaced:
 * cancelling the current request hands the catalog back to that read. An
 * invalidation (a mutation) must not, so it clears the displaced owner too.
 * A request that applied its list has already superseded the read it displaced,
 * so `applyReportList` clears that owner as well. */
export function beginReportListRequest(): symbol {
  const request = Symbol("report-list");
  displacedListRequest = currentListRequest;
  currentListRequest = request;
  return request;
}

export function isCurrentReportListRequest(request: symbol): boolean {
  return currentListRequest === request;
}

export function cancelReportListRequest(request: symbol): void {
  if (currentListRequest !== request) return;
  currentListRequest = displacedListRequest;
  displacedListRequest = null;
}

export function invalidateReportListRequests(): void {
  currentListRequest = Symbol("report-list-invalidated");
  displacedListRequest = null;
}

export function setReportsLoading() {
  commit({ ...state, status: "loading" });
}

/** `projectOrigins` is given by a list read; a mutation passes none and keeps the
 * origins it holds. Either way an origin whose report is no longer listed is
 * dropped, so a deleted report leaves nothing behind. */
export function applyReportList(reports: ReportView[], projectOrigins?: ReportProjectOrigin[]) {
  // What this list replaces is older than it: a read that was displaced by the
  // request that just applied must not be restored over it when that request ends.
  displacedListRequest = null;
  const present = new Set(reports.map((report) => report.id));
  commit({
    status: "ready",
    reports: sortReports(reports),
    projectOrigins: (projectOrigins ?? state.projectOrigins).filter((origin) => present.has(origin.report_id)),
  });
}

/** A failed read shows no reports: the last list is not current any more, and
 * the surface, the palette, and the counts must not present it as though it were. */
export function setReportsError() {
  commit({ status: "error", reports: [], projectOrigins: [] });
}

/** Puts the version of a report DA just returned (a refresh changes its size
 * and timestamps) in place of the one the list holds, or adds it when the list
 * does not have it yet. */
export function applyReportUpdated(report: ReportView) {
  // A list read already on its way predates this report, so it must not land
  // over it.
  invalidateReportListRequests();
  const without = state.reports.filter((existing) => existing.id !== report.id);
  commit({ ...state, reports: sortReports([...without, report]) });
}

/**
 * "Open this report" as an intent, in its own tiny store.
 *
 * The command palette has to be able to say "open the report that
 * already holds these bytes", and that report lives in a different destination.
 * Threading a callback from the shell down through `ChatSurface`, `MessageList`,
 * `MessageRow`, `AssistantMessage` and into a fence component would add a prop
 * to five components that have no interest in reports. An intent both ends read
 * keeps the plumbing out of the chat tree: the shell switches destination on it,
 * and the destination opens the report and clears it.
 *
 * Separate listener set from the list above so switching destination does not
 * re-render the shell on every list change.
 */
let pendingOpenId: string | null = null;
const pendingOpenListeners = new Set<Listener>();

export function subscribePendingReport(listener: Listener): () => void {
  pendingOpenListeners.add(listener);
  return () => pendingOpenListeners.delete(listener);
}

export function getPendingReportId(): string | null {
  return pendingOpenId;
}

function commitPendingOpen(next: string | null) {
  pendingOpenId = next;
  for (const listener of pendingOpenListeners) listener();
}

export function requestOpenReport(reportId: string) {
  commitPendingOpen(reportId);
}

export function clearPendingReport() {
  commitPendingOpen(null);
}
