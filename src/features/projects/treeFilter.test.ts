import { describe, expect, it } from "vitest";
import { filterProjectRows } from "./treeFilter";
import type { TreeRow } from "./useProjectTree";

/** The filter as it was before, kept verbatim as the reference for the new one. */
function referenceFilter(rows: readonly TreeRow[], query: string): TreeRow[] {
  if (query === "") return [...rows];

  const included = new Set<number>();
  rows.forEach((row, index) => {
    if (!row.name.toLocaleLowerCase().includes(query) && !row.path.toLocaleLowerCase().includes(query)) return;
    included.add(index);
    let depth = row.depth;
    for (let candidate = index - 1; candidate >= 0 && depth > 0; candidate -= 1) {
      if (rows[candidate].depth < depth) {
        included.add(candidate);
        depth = rows[candidate].depth;
      }
    }
  });

  return rows.filter((_row, index) => included.has(index));
}

function row(path: string, depth: number, kind: TreeRow["kind"] = "file"): TreeRow {
  return {
    path,
    name: path.split("/").pop() ?? path,
    kind,
    sizeBytes: 0,
    isSensitive: false,
    depth,
    expanded: false,
    loading: false,
  };
}

function dir(path: string, depth: number): TreeRow {
  return row(path, depth, "directory");
}

/** Rows in the order the tree flattens them: a folder, then its children. */
const PROJECT: TreeRow[] = [
  dir("src", 0),
  dir("src/app", 1),
  dir("src/app/ui", 2),
  row("src/app/ui/Button.tsx", 3),
  row("src/app/main.ts", 2),
  dir("src/lib", 1),
  row("src/lib/util.ts", 2),
  row("README.md", 0),
  dir("docs", 0),
  row("docs/Guide.md", 1),
];

describe("filterProjectRows", () => {
  it("returns a copy of every row for an empty query", () => {
    const result = filterProjectRows(PROJECT, "");
    expect(result).toEqual(PROJECT);
    expect(result).not.toBe(PROJECT);
    expect(result).toEqual(referenceFilter(PROJECT, ""));
  });

  it("returns nothing when no row matches", () => {
    expect(filterProjectRows(PROJECT, "zzz-no-match")).toEqual([]);
    expect(filterProjectRows(PROJECT, "zzz-no-match")).toEqual(referenceFilter(PROJECT, "zzz-no-match"));
  });

  it("keeps the ancestors of matches at depth 0, 1 and 3, in original order", () => {
    // Depth 0 only: README.md has no ancestors.
    expect(filterProjectRows(PROJECT, "readme").map((r) => r.path)).toEqual(["README.md"]);
    // Depth 1: Guide.md keeps its folder docs.
    expect(filterProjectRows(PROJECT, "guide").map((r) => r.path)).toEqual(["docs", "docs/Guide.md"]);
    // Depth 3: Button.tsx keeps src, src/app and src/app/ui.
    expect(filterProjectRows(PROJECT, "button").map((r) => r.path)).toEqual([
      "src",
      "src/app",
      "src/app/ui",
      "src/app/ui/Button.tsx",
    ]);
    for (const query of ["readme", "guide", "button"]) {
      expect(filterProjectRows(PROJECT, query)).toEqual(referenceFilter(PROJECT, query));
    }
  });

  it("reaches a deep match whose ancestors sit far back in a wide folder", () => {
    const wide: TreeRow[] = [dir("wide", 0), dir("wide/inner", 1)];
    for (let index = 0; index < 300; index += 1) {
      wide.push(row(`wide/inner/a-${index}.txt`, 2));
    }
    wide.push(dir("wide/inner/deep", 2), row("wide/inner/deep/needle.md", 3));

    const result = filterProjectRows(wide, "needle");
    expect(result.map((r) => r.path)).toEqual(["wide", "wide/inner", "wide/inner/deep", "wide/inner/deep/needle.md"]);
    expect(result).toEqual(referenceFilter(wide, "needle"));
  });

  it("keeps a shared ancestor once for sibling matches", () => {
    const siblings: TreeRow[] = [
      row("other.ts", 0),
      dir("src", 0),
      row("src/a.test.ts", 1),
      row("src/b.test.ts", 1),
      row("src/c.ts", 1),
    ];
    expect(filterProjectRows(siblings, "test").map((r) => r.path)).toEqual(["src", "src/a.test.ts", "src/b.test.ts"]);
    expect(filterProjectRows(siblings, "test")).toEqual(referenceFilter(siblings, "test"));
  });

  it("matches case-insensitively when the caller passes a lower-cased query", () => {
    const mixed: TreeRow[] = [dir("Src", 0), row("Src/Main.TS", 1), row("README.MD", 0)];
    expect(filterProjectRows(mixed, "main").map((r) => r.path)).toEqual(["Src", "Src/Main.TS"]);
    expect(filterProjectRows(mixed, "readme").map((r) => r.path)).toEqual(["README.MD"]);
    expect(filterProjectRows(mixed, "main")).toEqual(referenceFilter(mixed, "main"));
  });

  it("matches a path through a folder name, so every descendant of the folder matches", () => {
    const folders: TreeRow[] = [
      dir("Components", 0),
      row("Components/Button.tsx", 1),
      dir("Components/Icon", 1),
      row("Components/Icon/Star.tsx", 2),
      dir("lib", 0),
      row("lib/other.ts", 1),
    ];
    const result = filterProjectRows(folders, "components");
    expect(result.map((r) => r.path)).toEqual([
      "Components",
      "Components/Button.tsx",
      "Components/Icon",
      "Components/Icon/Star.tsx",
    ]);
    expect(result).toEqual(referenceFilter(folders, "components"));
  });

  it("matches the reference on a deterministic pseudo-random forest over 25 queries", () => {
    const forest = pseudoRandomForest(2000);
    // The fixture must be a real nested tree, or the comparison proves little.
    expect(forest.some((r) => r.depth >= 3)).toBe(true);
    expect(forest.every((r, index) => index === 0 || r.depth <= forest[index - 1].depth + 1)).toBe(true);
    const queries = [
      "part-1",
      "readme",
      "lib",
      "x",
      "zzz",
      "data1",
      "main",
      "notes",
      "a",
      "config3",
      "src",
      "index",
      "test",
      "deep",
      "-2",
      "7.",
      "node",
      "util",
      "root",
      "c",
      "guide",
      "_",
      "ui",
      "e",
      "lib/x",
    ];
    expect(queries).toHaveLength(25);
    for (const query of queries) {
      expect(filterProjectRows(forest, query)).toEqual(referenceFilter(forest, query));
    }
  });

  it("does not leak cached names or paths from one rows array into another", () => {
    const first: TreeRow[] = [dir("alpha", 0), row("alpha/x-file.txt", 1), row("beta.txt", 0)];
    const second: TreeRow[] = [row("gamma.txt", 0), dir("delta", 0), row("delta/x-deep.md", 1)];

    expect(filterProjectRows(first, "x-").map((r) => r.path)).toEqual(["alpha", "alpha/x-file.txt"]);
    expect(filterProjectRows(second, "x-").map((r) => r.path)).toEqual(["delta", "delta/x-deep.md"]);
    // Back to the first array with another query: still its own rows.
    expect(filterProjectRows(first, "beta").map((r) => r.path)).toEqual(["beta.txt"]);
    expect(filterProjectRows(first, "x-")).toEqual(referenceFilter(first, "x-"));
    expect(filterProjectRows(second, "x-")).toEqual(referenceFilter(second, "x-"));
  });
});

/** A valid flattened forest from a fixed seed: each row is a child of the
 * latest folder one level up, as `useProjectTree` flattens its rows. */
function pseudoRandomForest(count: number): TreeRow[] {
  const words = ["src", "Lib", "test", "Data", "index", "README", "config", "Main", "x", "part", "Notes", "node"];
  let state = 20260519;
  const next = (limit: number) => {
    state = (state * 1103515245 + 12345) & 0x7fffffff;
    return state % limit;
  };

  const rows: TreeRow[] = [];
  // folders[depth] is the latest directory at that depth, while it is still open.
  const folders: (string | undefined)[] = [];
  for (let index = 0; index < count; index += 1) {
    let depth = next(5);
    while (depth > 0 && folders[depth - 1] === undefined) depth -= 1;
    const parent = depth === 0 ? undefined : folders[depth - 1];
    const name = `${words[next(words.length)]}-${next(40)}${next(3) === 0 ? ".md" : ""}`;
    const path = parent === undefined ? name : `${parent}/${name}`;
    const isDirectory = next(3) === 0;
    rows.push(row(path, depth, isDirectory ? "directory" : "file"));
    folders.length = depth + 1;
    folders[depth] = isDirectory ? path : undefined;
  }
  return rows;
}
