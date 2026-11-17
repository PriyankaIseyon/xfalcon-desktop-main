import { describe, expect, it } from "vitest";
import type { DatasetErrorCode, DatasetFileView, DatasetView } from "../../lib/generated/contracts";
import {
  describeError,
  describeStep,
  describeStepError,
  formatBytes,
  formatCount,
  sortDatasets,
  totals,
} from "./datasetPresentation";

function dataset(overrides: Partial<DatasetView> & { id: string; display_name: string }): DatasetView {
  return {
    version: 1,
    status: "ready",
    created_at_ms: 1_783_500_000_000n,
    updated_at_ms: 1_783_500_000_000n,
    ...overrides,
  };
}

function file(overrides: Partial<DatasetFileView>): DatasetFileView {
  return {
    file_id: "f-1",
    table_name: "sales",
    row_count: null,
    column_count: null,
    size_bytes: null,
    original_filename: null,
    columns: [],
    created_at_ms: 0n,
    updated_at_ms: 0n,
    ...overrides,
  };
}

describe("formatCount", () => {
  it("keeps an unrecorded count distinct from zero", () => {
    // A table of no rows and a table whose count was never written are
    // different facts. Rendering both as 0 tells someone their data is empty.
    expect(formatCount(null)).toBe("Unknown");
    expect(formatCount(0)).toBe("0");
  });

  it("groups a large count", () => {
    expect(formatCount(482_109)).toBe((482_109).toLocaleString());
  });
});

describe("formatBytes", () => {
  it("names the unit a person reads", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
    expect(formatBytes(38 * 1024 * 1024)).toBe("38 MB");
    expect(formatBytes(2 * 1024 * 1024 * 1024)).toBe("2.0 GB");
  });

  it("keeps an unknown size unknown", () => {
    expect(formatBytes(null)).toBe("Unknown");
  });
});

describe("sortDatasets", () => {
  const first = dataset({ id: "a", display_name: "Apex", created_at_ms: 100n });
  const second = dataset({ id: "b", display_name: "beta", created_at_ms: 300n });
  const third = dataset({ id: "c", display_name: "Cedar", created_at_ms: 200n });

  it("orders by upload time, newest first", () => {
    expect(sortDatasets([first, second, third], "uploaded", "desc").map((row) => row.id)).toEqual(["b", "c", "a"]);
  });

  it("orders by name without regard to case", () => {
    expect(sortDatasets([second, third, first], "name", "asc").map((row) => row.id)).toEqual(["a", "b", "c"]);
  });

  it("breaks a tie on upload time by name, so the order is stable", () => {
    const tied = dataset({ id: "d", display_name: "Aardvark", created_at_ms: 100n });
    expect(sortDatasets([first, tied], "uploaded", "desc").map((row) => row.id)).toEqual(["d", "a"]);
  });
});

describe("totals", () => {
  it("sums what the server recorded and leaves the rest unknown", () => {
    expect(totals([file({ row_count: 10 }), file({ row_count: 5 })])).toEqual({
      rows: 15,
      columns: null,
    });
    expect(totals([file({})])).toEqual({ rows: null, columns: null });
  });
});

describe("describeError", () => {
  const codes: DatasetErrorCode[] = [
    "auth_required",
    "not_configured",
    "uploads_disabled",
    "update_pending",
    "stage_not_found",
    "stage_limit_reached",
    "no_file_chosen",
    "unsupported_format",
    "content_mismatch",
    "encrypted_file",
    "empty_file",
    "file_too_large",
    "unreadable",
    "recipe_too_long",
    "recipe_invalid_step",
    "recipe_file_too_large",
    "not_found",
    "server_refused",
    "network",
    "storage",
  ];

  it("writes a sentence for every code the boundary can send", () => {
    // The backend sends a code and never a sentence — DA's own prose can name a
    // presigned URL or a bucket. So this mapping is the only place the words
    // exist, and a code with no words would render as nothing.
    for (const code of codes) {
      const message = describeError(code);
      expect(message.length, code).toBeGreaterThan(0);
      expect(message.endsWith("."), code).toBe(true);
    }
  });

  it("names the step a refused recipe points at", () => {
    expect(describeStepError("recipe_invalid_step", 2)).toContain("Step 3");
    // Without a position there is nothing to point at, so it falls back.
    expect(describeStepError("recipe_invalid_step", null)).toBe(describeError("recipe_invalid_step"));
  });
});

describe("describeStep", () => {
  it("reads as a line a person can check", () => {
    expect(describeStep({ kind: "filter", column: "region", op: "equals", value: "EMEA" })).toBe(
      'keep rows where region is "EMEA"',
    );
    expect(describeStep({ kind: "filter", column: "region", op: "is_null", value: null })).toBe(
      "keep rows where region is empty",
    );
    expect(describeStep({ kind: "sort", column: "revenue", descending: true })).toBe("sort by revenue, largest first");
    expect(describeStep({ kind: "dedup" })).toBe("drop duplicate rows");
    expect(describeStep({ kind: "split", column: "name", delimiter: " ", parts: 2 })).toBe('split name on " " into 2');
  });
});
