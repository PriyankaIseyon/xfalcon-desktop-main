import { describe, expect, it } from "vitest";
import type { ReportDatasetFreshness, ReportProjectOrigin, ReportView } from "../../lib/generated/contracts";
import {
  formatRelativeUpdated,
  formatDuration,
  freshnessSummary,
  isFileReport,
  isListed,
  isNameTaken,
  matchesProject,
  originOf,
  projectChoices,
  reportAudience,
  reportFormatLabel,
  reportKindLabel,
  reportSharing,
} from "./reportPresentation";

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

describe("reportSharing", () => {
  it("is null for a private report", () => {
    expect(reportSharing(report({ id: "a", scope: "user", is_owner: true }))).toBeNull();
  });

  it("names the organization for a global report the viewer owns", () => {
    expect(reportSharing(report({ id: "a", scope: "global", is_owner: true }))).toBe("Shared with your organization");
  });

  it("names the owner for a global report someone else owns", () => {
    expect(reportSharing(report({ id: "a", scope: "global", is_owner: false, owner_display_name: "Priya Shah" }))).toBe(
      "Shared by Priya Shah",
    );
  });

  it("falls back to a generic owner when DA gives no name", () => {
    expect(reportSharing(report({ id: "a", scope: "global", is_owner: false, owner_display_name: null }))).toBe(
      "Shared by a teammate",
    );
  });
});

describe("isNameTaken", () => {
  const reports = [report({ id: "a", title: "Sales Snapshot" }), report({ id: "b", title: "  Stock  " })];

  it("matches case-insensitively and ignores surrounding whitespace", () => {
    expect(isNameTaken(reports, "sales snapshot", null)).toBe(true);
    expect(isNameTaken(reports, "  STOCK ", null)).toBe(true);
  });

  it("does not match a different name", () => {
    expect(isNameTaken(reports, "Sales", null)).toBe(false);
  });

  it("ignores the report being renamed", () => {
    expect(isNameTaken(reports, "Sales Snapshot", "a")).toBe(false);
    expect(isNameTaken(reports, "Sales Snapshot", "b")).toBe(true);
  });

  it("never reports a blank name as taken", () => {
    const blank = [report({ id: "c", title: "   " })];
    expect(isNameTaken(blank, "  ", null)).toBe(false);
  });
});

function dataset(overrides: Partial<ReportDatasetFreshness> & { dataset_id: string }): ReportDatasetFreshness {
  return { version: 1, executed_at_ms: null, row_count: null, ...overrides };
}

describe("report kinds and audience", () => {
  it("names each kind and its file format, and treats only a dashboard as a page", () => {
    expect(reportKindLabel("dashboard")).toBe("Dashboard");
    expect(reportKindLabel("workbook")).toBe("Excel workbook");
    expect(reportKindLabel("presentation")).toBe("PowerPoint deck");
    expect(reportFormatLabel("dashboard")).toBe("HTML");
    expect(reportFormatLabel("workbook")).toBe("Excel");
    expect(reportFormatLabel("presentation")).toBe("PowerPoint");
    expect(isFileReport({ kind: "dashboard" })).toBe(false);
    expect(isFileReport({ kind: "workbook" })).toBe(true);
    expect(isFileReport({ kind: "presentation" })).toBe(true);
  });

  it("says who can open a report as one word, and lists what the reader has not hidden", () => {
    expect(reportAudience({ scope: "user" })).toBe("Private");
    expect(reportAudience({ scope: "global" })).toBe("Organization");
    expect(isListed({ hidden: false })).toBe(true);
    expect(isListed({ hidden: true })).toBe(false);
  });
});

describe("freshnessSummary", () => {
  const refreshable = { refreshable: true };

  it("states nothing for a report with no stored queries", () => {
    expect(
      freshnessSummary({ refreshable: false }, [dataset({ dataset_id: "a", executed_at_ms: 1_790_848_800_000n })]),
    ).toBeNull();
  });

  it("says the saved rows are what the page shows when nothing was ever stored", () => {
    for (const datasets of [[], [dataset({ dataset_id: "a" }), dataset({ dataset_id: "b" })]]) {
      expect(freshnessSummary(refreshable, datasets)).toBe(
        "Saved data has not been refreshed yet. The page shows the rows it was saved with.",
      );
    }
  });

  it("states the OLDEST stored time, because a report is as fresh as its stalest dataset", () => {
    const older = new Date(1_790_848_800_000).toLocaleString(undefined, {
      day: "numeric",
      month: "short",
      year: "numeric",
      hour: "numeric",
      minute: "2-digit",
    });
    const text = freshnessSummary(refreshable, [
      dataset({ dataset_id: "a", executed_at_ms: 1_790_935_200_000n }),
      dataset({ dataset_id: "b", executed_at_ms: 1_790_848_800_000n }),
    ]);
    expect(text).toBe(`Data stored ${older}.`);
  });

  it("counts the datasets that were never refreshed, in the singular and the plural", () => {
    const stored = dataset({ dataset_id: "a", executed_at_ms: 1_790_848_800_000n });
    expect(freshnessSummary(refreshable, [stored, dataset({ dataset_id: "b" })])).toMatch(
      /1 dataset has not been refreshed yet\.$/,
    );
    expect(freshnessSummary(refreshable, [stored, dataset({ dataset_id: "b" }), dataset({ dataset_id: "c" })])).toMatch(
      /2 datasets have not been refreshed yet\.$/,
    );
  });

  it("never says the data is current", () => {
    const text = freshnessSummary(refreshable, [dataset({ dataset_id: "a", executed_at_ms: 1_790_848_800_000n })]);
    expect(text).not.toMatch(/current|up to date|live|fresh/i);
  });
});

describe("formatDuration", () => {
  it("states a duration in milliseconds under a second and in seconds above it", () => {
    expect(formatDuration(850n)).toBe("850 ms");
    expect(formatDuration(1_000n)).toBe("1 s");
    expect(formatDuration(2_000)).toBe("2 s");
  });
});

describe("project filtering", () => {
  const origin = (report_id: string, project_id: string, project_name: string): ReportProjectOrigin => ({
    version: 1,
    report_id,
    project_id,
    project_name,
  });
  const origins = [origin("a", "p-sales", "Sales"), origin("b", "p-sales", "Sales"), origin("c", "p-ops", "Ops")];

  it("finds a report's project, and none for a report with no link", () => {
    expect(originOf("a", origins)?.project_name).toBe("Sales");
    expect(originOf("z", origins)).toBeNull();
  });

  it("matches all, only the unlinked, or one project", () => {
    expect(matchesProject({ id: "z" }, "all", origins)).toBe(true);
    expect(matchesProject({ id: "z" }, "none", origins)).toBe(true);
    expect(matchesProject({ id: "a" }, "none", origins)).toBe(false);
    expect(matchesProject({ id: "a" }, "p-sales", origins)).toBe(true);
    expect(matchesProject({ id: "c" }, "p-sales", origins)).toBe(false);
    // A report with no link is in no project, whichever one is asked for.
    expect(matchesProject({ id: "z" }, "p-sales", origins)).toBe(false);
  });

  it("offers each project that has a listed report, with its count, A to Z", () => {
    const listed = [{ id: "a" }, { id: "b" }, { id: "c" }, { id: "z" }];
    expect(projectChoices(listed, origins)).toEqual([
      { id: "p-ops", name: "Ops", count: 1 },
      { id: "p-sales", name: "Sales", count: 2 },
    ]);
  });

  it("counts only what is listed, so a hidden report is not in the number", () => {
    expect(projectChoices([{ id: "a" }, { id: "c" }], origins)).toEqual([
      { id: "p-ops", name: "Ops", count: 1 },
      { id: "p-sales", name: "Sales", count: 1 },
    ]);
  });

  it("drops a project with nothing listed, unless it is the one chosen", () => {
    expect(projectChoices([{ id: "c" }], origins)).toEqual([{ id: "p-ops", name: "Ops", count: 1 }]);
    expect(projectChoices([{ id: "c" }], origins, "p-sales")).toEqual([
      { id: "p-ops", name: "Ops", count: 1 },
      { id: "p-sales", name: "Sales", count: 0 },
    ]);
    // A project no origin names any more cannot be kept on offer.
    expect(projectChoices([{ id: "c" }], origins, "p-gone")).toEqual([{ id: "p-ops", name: "Ops", count: 1 }]);
  });
});

describe("formatRelativeUpdated", () => {
  const now = Date.UTC(2026, 9, 12, 12, 0, 0);
  const ago = (seconds: number) => BigInt(now - seconds * 1000);

  it("says how long ago a recent change was, and the date past a week", () => {
    expect(formatRelativeUpdated(ago(10), now)).toBe("just now");
    expect(formatRelativeUpdated(ago(5 * 60), now)).toMatch(/5 minutes ago/);
    expect(formatRelativeUpdated(ago(3 * 3_600), now)).toMatch(/3 hours ago/);
    expect(formatRelativeUpdated(ago(26 * 3_600), now)).toMatch(/yesterday/i);
    expect(formatRelativeUpdated(ago(3 * 86_400), now)).toMatch(/3 days ago/);
    expect(formatRelativeUpdated(ago(30 * 86_400), now)).toBe(
      new Date(now - 30 * 86_400_000).toLocaleDateString(undefined, {
        day: "numeric",
        month: "short",
        year: "numeric",
      }),
    );
  });
});
