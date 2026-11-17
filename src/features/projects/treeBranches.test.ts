import { describe, expect, it } from "vitest";
import { indexBranches, reachSteps, rowBranches } from "./treeBranches";
import type { TreeRow } from "./useProjectTree";

function row(path: string, depth: number): TreeRow {
  return {
    path,
    name: path.split("/").pop() ?? path,
    kind: "file",
    sizeBytes: 0,
    isSensitive: false,
    depth,
    expanded: false,
    loading: false,
  };
}

// src/            0
//   a/            1
//     x           2
//     y           2
//   b             1
// README          0
const ROWS = [row("src", 0), row("src/a", 1), row("src/a/x", 2), row("src/a/y", 2), row("src/b", 1), row("README", 0)];

describe("tree branches", () => {
  it("finds each row's parent and whether it has a later sibling", () => {
    const index = indexBranches(ROWS);
    expect(Array.from(index.parents)).toEqual([-1, 0, 1, 1, 0, -1]);
    expect(index.hasLater).toEqual([true, true, true, false, false, false]);
  });

  it("keeps a folder's line running past a row that has later siblings, and ends it at the last", () => {
    const index = indexBranches(ROWS);
    // src/a/x sits under src/a, which has a later sibling (src/b): both columns run on.
    expect(rowBranches(ROWS, index, [], 2).columns.map((column) => column.continues)).toEqual([true, true]);
    // src/a/y is a's last row, but a itself still has src/b after it.
    expect(rowBranches(ROWS, index, [], 3).columns.map((column) => column.continues)).toEqual([true, false]);
    // src/b is the last of src: its trunk ends at the elbow.
    expect(rowBranches(ROWS, index, [], 4).columns.map((column) => column.continues)).toEqual([false]);
    expect(rowBranches(ROWS, index, [], 0).columns).toEqual([]);
  });

  it("marks the way from the top of the tree to the chosen row", () => {
    const index = indexBranches(ROWS);
    const steps = reachSteps(ROWS, index, "src/b");
    expect(steps).toEqual([{ index: 4, parentIndex: 0 }]);
    // Rows between src and src/b carry the accent down column 0; src/b turns off it.
    expect(rowBranches(ROWS, index, steps, 1).columns[0].reach).toBe(true);
    expect(rowBranches(ROWS, index, steps, 3).columns[0].reach).toBe(true);
    expect(rowBranches(ROWS, index, steps, 4)).toMatchObject({ elbowReach: true, columns: [{ reach: false }] });
  });

  it("follows every ancestor of a deep row, and nothing for a hidden or top-level one", () => {
    const index = indexBranches(ROWS);
    const steps = reachSteps(ROWS, index, "src/a/y");
    expect(steps).toEqual([
      { index: 1, parentIndex: 0 },
      { index: 3, parentIndex: 1 },
    ]);
    expect(rowBranches(ROWS, index, steps, 2).columns[1].reach).toBe(true);
    expect(rowBranches(ROWS, index, steps, 3).elbowReach).toBe(true);
    expect(reachSteps(ROWS, index, "gone/file")).toEqual([]);
    expect(reachSteps(ROWS, index, "README")).toEqual([]);
    expect(reachSteps(ROWS, index, null)).toEqual([]);
  });
});
