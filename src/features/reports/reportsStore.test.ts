import { beforeEach, describe, expect, it } from "vitest";
import type { ReportView } from "../../lib/generated/contracts";
import {
  applyReportList,
  applyReportUpdated,
  beginReportListRequest,
  cancelReportListRequest,
  getReportsSnapshot,
  invalidateReportListRequests,
  isCurrentReportListRequest,
  resetReportsStore,
  setReportsError,
  sortReports,
} from "./reportsStore";

function report(overrides: Partial<ReportView> & { id: string }): ReportView {
  return {
    version: 1,
    kind: "dashboard",
    title: `Report ${overrides.id}`,
    description: null,
    created_at_ms: 1n,
    updated_at_ms: 1n,
    thumbnail_updated_at_ms: null,
    data_sources: [],
    pinned: false,
    hidden: false,
    byte_size: 10n,
    scope: "user",
    is_owner: true,
    owner_display_name: null,
    refreshable: true,
    can_refresh: true,
    can_manage: true,
    can_delete: true,
    page_count: 1,
    dataset_count: 1,
    source_conversation_id: null,
    ...overrides,
  };
}

const ids = (reports: ReportView[]) => reports.map((entry) => entry.id);

describe("sortReports", () => {
  it("puts pinned reports first, then newest, then id", () => {
    const sorted = sortReports([
      report({ id: "c", updated_at_ms: 300n }),
      report({ id: "b", updated_at_ms: 100n, pinned: true }),
      report({ id: "a", updated_at_ms: 200n }),
      report({ id: "d", updated_at_ms: 400n, pinned: true }),
    ]);
    expect(ids(sorted)).toEqual(["d", "b", "c", "a"]);
  });

  it("orders ties on update time by id ascending", () => {
    const sorted = sortReports([
      report({ id: "m", updated_at_ms: 5n }),
      report({ id: "a", updated_at_ms: 5n }),
      report({ id: "z", updated_at_ms: 5n, pinned: true }),
      report({ id: "k", updated_at_ms: 5n, pinned: true }),
    ]);
    expect(ids(sorted)).toEqual(["k", "z", "a", "m"]);
  });

  it("does not reorder reports that tie on every key", () => {
    const first = report({ id: "same", title: "First", updated_at_ms: 9n });
    const second = report({ id: "same", title: "Second", updated_at_ms: 9n });
    const sorted = sortReports([first, second]);
    expect(sorted.map((entry) => entry.title)).toEqual(["First", "Second"]);
  });

  it("returns a new array and leaves its input unchanged", () => {
    const input = [report({ id: "b", updated_at_ms: 1n }), report({ id: "a", updated_at_ms: 2n })];
    const sorted = sortReports(input);
    expect(sorted).not.toBe(input);
    expect(ids(input)).toEqual(["b", "a"]);
  });
});

describe("reports store", () => {
  beforeEach(() => {
    resetReportsStore();
  });

  it("applyReportList sets ready and sorts, after an earlier error", () => {
    setReportsError();
    applyReportList([report({ id: "x", updated_at_ms: 1n }), report({ id: "y", updated_at_ms: 2n, pinned: true })]);

    const snapshot = getReportsSnapshot();
    expect(snapshot.status).toBe("ready");
    expect(ids(snapshot.reports)).toEqual(["y", "x"]);
  });

  it("applyReportList replaces the previous list rather than merging it", () => {
    applyReportList([report({ id: "old" })]);
    applyReportList([report({ id: "new" })]);
    expect(ids(getReportsSnapshot().reports)).toEqual(["new"]);
  });

  it("applyReportUpdated adds a report the list does not hold", () => {
    applyReportList([report({ id: "a", updated_at_ms: 1n })]);
    applyReportUpdated(report({ id: "b", updated_at_ms: 2n }));
    expect(ids(getReportsSnapshot().reports)).toEqual(["b", "a"]);
  });

  it("applyReportUpdated replaces a report that already carries its id", () => {
    applyReportList([report({ id: "a", title: "Old", updated_at_ms: 1n }), report({ id: "b", updated_at_ms: 2n })]);
    applyReportUpdated(report({ id: "a", title: "New", updated_at_ms: 3n }));

    const snapshot = getReportsSnapshot();
    expect(snapshot.reports).toHaveLength(2);
    expect(ids(snapshot.reports)).toEqual(["a", "b"]);
    expect(snapshot.reports[0].title).toBe("New");
  });

  it("applyReportUpdated drops a list read that started before it", () => {
    const earlier = beginReportListRequest();
    applyReportUpdated(report({ id: "a" }));
    expect(isCurrentReportListRequest(earlier)).toBe(false);
  });

  it("a cancelled prefetch hands the catalog back to the read it displaced", () => {
    const read = beginReportListRequest();
    const prefetch = beginReportListRequest();
    cancelReportListRequest(prefetch);
    expect(isCurrentReportListRequest(read)).toBe(true);
    expect(isCurrentReportListRequest(prefetch)).toBe(false);
  });

  it("a mutation during a prefetch still stops the read it displaced", () => {
    const read = beginReportListRequest();
    invalidateReportListRequests();
    const prefetch = beginReportListRequest();
    cancelReportListRequest(prefetch);
    expect(isCurrentReportListRequest(read)).toBe(false);
  });

  it("a prefetch that applied its list does not bring back the older read when it ends", () => {
    const read = beginReportListRequest();
    const prefetch = beginReportListRequest();
    applyReportList([report({ id: "fresh" })]);
    cancelReportListRequest(prefetch);
    expect(isCurrentReportListRequest(read)).toBe(false);
    expect(ids(getReportsSnapshot().reports)).toEqual(["fresh"]);
  });

  it("cancelling a refresh that unmounts leaves the catalog to what it displaced", () => {
    const refresh = beginReportListRequest();
    expect(() => cancelReportListRequest(refresh)).not.toThrow();
    expect(isCurrentReportListRequest(refresh)).toBe(false);
    const next = beginReportListRequest();
    expect(isCurrentReportListRequest(next)).toBe(true);
  });

  it("resetReportsStore returns to loading with no reports", () => {
    applyReportList([report({ id: "a" })]);
    setReportsError();

    resetReportsStore();

    expect(getReportsSnapshot()).toEqual({ status: "loading", reports: [], projectOrigins: [] });
  });
});

describe("project origins", () => {
  const origin = (report_id: string) => ({ version: 1 as const, report_id, project_id: "p-1", project_name: "Sales" });

  beforeEach(() => resetReportsStore());

  it("holds the origins a list read brought, beside the reports", () => {
    applyReportList([report({ id: "a" }), report({ id: "b" })], [origin("a")]);
    expect(getReportsSnapshot().projectOrigins).toEqual([origin("a")]);
  });

  it("keeps them through a write that returns a new row", () => {
    applyReportList([report({ id: "a" })], [origin("a")]);
    applyReportUpdated(report({ id: "a", pinned: true }));
    expect(getReportsSnapshot().projectOrigins).toEqual([origin("a")]);
    applyReportList([report({ id: "a", title: "Renamed" })]);
    expect(getReportsSnapshot().projectOrigins).toEqual([origin("a")]);
  });

  it("drops the origin of a report that is no longer listed", () => {
    applyReportList([report({ id: "a" }), report({ id: "b" })], [origin("a"), origin("b")]);
    applyReportList([report({ id: "b" })]);
    expect(getReportsSnapshot().projectOrigins).toEqual([origin("b")]);
  });

  it("replaces them on the next list read, and clears them on an error", () => {
    applyReportList([report({ id: "a" })], [origin("a")]);
    applyReportList([report({ id: "a" })], []);
    expect(getReportsSnapshot().projectOrigins).toEqual([]);
    applyReportList([report({ id: "a" })], [origin("a")]);
    setReportsError();
    expect(getReportsSnapshot().projectOrigins).toEqual([]);
  });
});
