import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { ReportScope, ReportView } from "../../lib/generated/contracts";
import {
  reportDelete,
  reportList,
  reportRename,
  reportSetHidden,
  reportSetPinned,
  reportSetScope,
} from "../../lib/native";
import { toast } from "../../ui/toastStore";
import {
  applyReportList,
  beginReportListRequest,
  cancelReportListRequest,
  getReportsSnapshot,
  invalidateReportListRequests,
  isCurrentReportListRequest,
  setReportsError,
  setReportsLoading,
  subscribeReports,
} from "./reportsStore";
import { captureReportPreview } from "./reportPreviewCapture";

export type ReportMutation = "rename" | "pin" | "delete" | "share" | "hide" | "preview";
export type ReportMutationResult = "succeeded" | "failed" | "forbidden" | "busy" | "stale";

/**
 * Whether a rejected native call is DA refusing the account. The Rust side
 * sends `{ version, code }` (`ReportOperationError`), and `forbidden` is the
 * wire spelling of `ReportErrorCode`. Surfaces use it to explain why an action
 * is not theirs to take, rather than showing a generic failure.
 */
export function isForbidden(cause: unknown): boolean {
  return typeof cause === "object" && cause !== null && "code" in cause && cause.code === "forbidden";
}

/**
 * Whether a list read that has come back should be dropped.
 *
 * A read is stale when something newer owns the catalog AND the store already
 * holds a result. While the store is still loading, no newer result exists to
 * protect: the palette's prefetch, which takes over the catalog, may be closed
 * or fail without ever applying one, and dropping this read would leave the
 * destination on its skeleton with no way out.
 */
function isSupersededRead(request: symbol): boolean {
  return !isCurrentReportListRequest(request) && getReportsSnapshot().status !== "loading";
}

/**
 * The Reports destination's data. Mutations commit only after the native store
 * confirms them. This keeps the complete visible snapshot unchanged on failure
 * and prevents one failed action from rolling a newer action back.
 */
export function useReports(active: boolean) {
  const snapshot = useSyncExternalStore(subscribeReports, getReportsSnapshot, getReportsSnapshot);
  const ownedListRequest = useRef<symbol | null>(null);
  const mounted = useRef(true);
  const mutations = useRef(new Map<string, { kind: ReportMutation; token: symbol }>());
  const [busyById, setBusyById] = useState<Readonly<Record<string, ReportMutation>>>({});

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (ownedListRequest.current !== null) cancelReportListRequest(ownedListRequest.current);
    };
  }, []);

  const beginMutation = useCallback((reportId: string, kind: ReportMutation): symbol | null => {
    if (mutations.current.has(reportId)) return null;
    // A list read that started before this write cannot be allowed to replace
    // the confirmed result after it arrives.
    invalidateReportListRequests();
    const token = Symbol(kind);
    mutations.current.set(reportId, { kind, token });
    setBusyById(Object.fromEntries([...mutations.current].map(([id, entry]) => [id, entry.kind])));
    return token;
  }, []);

  const ownsMutation = useCallback(
    (reportId: string, token: symbol) => mounted.current && mutations.current.get(reportId)?.token === token,
    [],
  );

  const finishMutation = useCallback((reportId: string, token: symbol) => {
    if (mutations.current.get(reportId)?.token !== token) return;
    mutations.current.delete(reportId);
    if (mounted.current) {
      setBusyById(Object.fromEntries([...mutations.current].map(([id, entry]) => [id, entry.kind])));
    }
  }, []);

  const refresh = useCallback(async () => {
    const request = beginReportListRequest();
    ownedListRequest.current = request;
    setReportsLoading();
    try {
      const result = await reportList();
      if (!mounted.current || isSupersededRead(request)) return;
      applyReportList(result.reports, result.project_origins);
    } catch {
      if (!mounted.current || isSupersededRead(request)) return;
      setReportsError();
    }
  }, []);

  /**
   * Re-reads the library without emptying the surface first, for a revisit with
   * the list still in the store. `refresh` blanks the surface instead: on a first
   * visit, or for an explicit retry. A failure is an error state, as for any
   * read: the store holds no list it can no longer vouch for.
   */
  const revalidate = useCallback(async () => {
    const request = beginReportListRequest();
    ownedListRequest.current = request;
    try {
      const result = await reportList();
      if (!mounted.current || isSupersededRead(request)) return;
      applyReportList(result.reports, result.project_origins);
    } catch {
      if (!mounted.current || isSupersededRead(request)) return;
      setReportsError();
    }
  }, []);

  // Loaded when the destination opens, not on shell mount: an account that
  // never opens Reports should not read its library from DA. A revisit with the
  // list still in the store shows that list while the read runs behind it.
  useEffect(() => {
    if (!active) return;
    if (getReportsSnapshot().status === "ready") void revalidate();
    else void refresh();
  }, [active, refresh, revalidate]);

  const rename = useCallback(
    async (report: ReportView, title: string, description: string | null): Promise<ReportMutationResult> => {
      const token = beginMutation(report.id, "rename");
      if (token === null) return "busy";
      try {
        const updated = await reportRename(report.id, title, description);
        if (!ownsMutation(report.id, token)) return "stale";
        if (updated.id !== report.id) {
          toast.error("Could not update report");
          return "failed";
        }
        invalidateReportListRequests();
        const current = getReportsSnapshot();
        applyReportList(current.reports.map((existing) => (existing.id === updated.id ? updated : existing)));
        toast.success("Report updated");
        return "succeeded";
      } catch (cause) {
        if (!ownsMutation(report.id, token)) return "stale";
        // A refusal for ownership is a different answer from a failure: the dialog
        // that asked says so, because trying again cannot help.
        if (isForbidden(cause)) return "forbidden";
        toast.error("Could not update report");
        return "failed";
      } finally {
        finishMutation(report.id, token);
      }
    },
    [beginMutation, finishMutation, ownsMutation],
  );

  const setPinned = useCallback(
    async (report: ReportView, pinned: boolean): Promise<ReportMutationResult> => {
      const token = beginMutation(report.id, "pin");
      if (token === null) return "busy";
      try {
        const updated = await reportSetPinned(report.id, pinned);
        if (!ownsMutation(report.id, token)) return "stale";
        if (updated.id !== report.id) {
          toast.error(pinned ? "Could not pin report" : "Could not unpin report");
          return "failed";
        }
        invalidateReportListRequests();
        const current = getReportsSnapshot();
        applyReportList(current.reports.map((existing) => (existing.id === updated.id ? updated : existing)));
        toast.success(pinned ? "Report pinned" : "Report unpinned");
        return "succeeded";
      } catch {
        if (!ownsMutation(report.id, token)) return "stale";
        toast.error(pinned ? "Could not pin report" : "Could not unpin report");
        return "failed";
      } finally {
        finishMutation(report.id, token);
      }
    },
    [beginMutation, finishMutation, ownsMutation],
  );

  const setScope = useCallback(
    async (report: ReportView, scope: ReportScope): Promise<ReportMutationResult> => {
      const token = beginMutation(report.id, "share");
      if (token === null) return "busy";
      try {
        const updated = await reportSetScope(report.id, scope);
        if (!ownsMutation(report.id, token)) return "stale";
        if (updated.id !== report.id) {
          toast.error("Could not change sharing");
          return "failed";
        }
        invalidateReportListRequests();
        const current = getReportsSnapshot();
        applyReportList(current.reports.map((existing) => (existing.id === updated.id ? updated : existing)));
        toast.success(scope === "global" ? "Report shared with your organization" : "Report is private again");
        return "succeeded";
      } catch (cause) {
        if (!ownsMutation(report.id, token)) return "stale";
        // Sharing is the owner's: a refusal is explained where it was asked for.
        if (isForbidden(cause)) return "forbidden";
        toast.error("Could not change sharing");
        return "failed";
      } finally {
        finishMutation(report.id, token);
      }
    },
    [beginMutation, finishMutation, ownsMutation],
  );

  const setHidden = useCallback(
    async (report: ReportView, hidden: boolean): Promise<ReportMutationResult> => {
      const token = beginMutation(report.id, "hide");
      if (token === null) return "busy";
      try {
        const updated = await reportSetHidden(report.id, hidden);
        if (!ownsMutation(report.id, token)) return "stale";
        if (updated.id !== report.id) {
          toast.error(hidden ? "Could not hide report" : "Could not show report");
          return "failed";
        }
        invalidateReportListRequests();
        const current = getReportsSnapshot();
        applyReportList(current.reports.map((existing) => (existing.id === updated.id ? updated : existing)));
        toast.success(hidden ? "Report hidden from your list" : "Report is back in your list");
        return "succeeded";
      } catch {
        if (!ownsMutation(report.id, token)) return "stale";
        toast.error(hidden ? "Could not hide report" : "Could not show report");
        return "failed";
      } finally {
        finishMutation(report.id, token);
      }
    },
    [beginMutation, finishMutation, ownsMutation],
  );

  const remove = useCallback(
    async (report: ReportView): Promise<ReportMutationResult> => {
      const token = beginMutation(report.id, "delete");
      if (token === null) return "busy";
      try {
        await reportDelete(report.id);
        if (!ownsMutation(report.id, token)) return "stale";
        invalidateReportListRequests();
        const current = getReportsSnapshot();
        applyReportList(current.reports.filter((existing) => existing.id !== report.id));
        toast.success("Report deleted");
        return "succeeded";
      } catch (cause) {
        if (!ownsMutation(report.id, token)) return "stale";
        if (isForbidden(cause)) return "forbidden";
        toast.error("Could not delete report");
        return "failed";
      } finally {
        finishMutation(report.id, token);
      }
    },
    [beginMutation, finishMutation, ownsMutation],
  );

  /** Makes the owner's dashboard image again from its current page, replacing
   * the one DA holds. The capture stores the report it returns itself. */
  const regeneratePreview = useCallback(
    async (report: ReportView): Promise<ReportMutationResult> => {
      const token = beginMutation(report.id, "preview");
      if (token === null) return "busy";
      try {
        await captureReportPreview(report.id);
        if (!ownsMutation(report.id, token)) return "stale";
        toast.success("Preview updated");
        return "succeeded";
      } catch {
        if (!ownsMutation(report.id, token)) return "stale";
        toast.error("Could not update the preview");
        return "failed";
      } finally {
        finishMutation(report.id, token);
      }
    },
    [beginMutation, finishMutation, ownsMutation],
  );

  return { snapshot, busyById, refresh, rename, setPinned, setScope, setHidden, remove, regeneratePreview };
}
