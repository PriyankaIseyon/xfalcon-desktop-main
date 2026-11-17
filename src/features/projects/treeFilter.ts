import { indexBranches } from "./treeBranches";
import type { TreeRow } from "./useProjectTree";

interface SearchableRows {
  names: string[];
  paths: string[];
  /** Index of each row's parent row, or -1 at the top level. */
  parents: Int32Array;
}

// Lower-casing every name and path costs as much as the search itself, and the
// filter runs on every keystroke over the same rows. Keyed by the array itself,
// so a new tree (a new array) never reads another tree's entries.
const searchableByRows = new WeakMap<readonly TreeRow[], SearchableRows>();

function searchableRows(rows: readonly TreeRow[]): SearchableRows {
  let searchable = searchableByRows.get(rows);
  if (searchable === undefined) {
    searchable = {
      names: rows.map((row) => row.name.toLocaleLowerCase()),
      paths: rows.map((row) => row.path.toLocaleLowerCase()),
      parents: indexBranches(rows).parents,
    };
    searchableByRows.set(rows, searchable);
  }
  return searchable;
}

/** Keeps each matching row's visible ancestors so filtering never creates an
 * invalid tree level with no parent. Only already-expanded rows are searched;
 * filtering must not crawl the filesystem or expand folders behind the user. */
export function filterProjectRows(rows: readonly TreeRow[], query: string): TreeRow[] {
  if (query === "") return [...rows];

  const { names, paths, parents } = searchableRows(rows);
  // A kept row's ancestors are all kept already, so each walk up stops at the
  // first kept row it reaches.
  const kept = new Uint8Array(rows.length);
  for (let index = 0; index < rows.length; index += 1) {
    if (!names[index].includes(query) && !paths[index].includes(query)) continue;
    for (let current = index; current >= 0 && kept[current] === 0; current = parents[current]) {
      kept[current] = 1;
    }
  }

  return rows.filter((_row, index) => kept[index] === 1);
}
