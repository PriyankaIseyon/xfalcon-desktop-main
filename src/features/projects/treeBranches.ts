import type { TreeRow } from "./useProjectTree";

/**
 * Where the tree's connector lines run.
 *
 * Rows arrive flattened and depth-tagged for the virtualizer, so a row cannot
 * see its siblings. This reads that list once and answers, per row, the two
 * things a connector needs: whether a row has a later sibling (its folder's
 * line continues past it) and which row owns it.
 */
export interface BranchIndex {
  /** Index of each row's parent row, or -1 at the top level. */
  parents: Int32Array;
  /** Whether a later row shares the row's parent. */
  hasLater: boolean[];
}

export function indexBranches(rows: readonly TreeRow[]): BranchIndex {
  const parents = new Int32Array(rows.length).fill(-1);
  const stack: number[] = [];
  rows.forEach((row, index) => {
    parents[index] = row.depth > 0 ? (stack[row.depth - 1] ?? -1) : -1;
    stack[row.depth] = index;
    stack.length = row.depth + 1;
  });

  // Backwards, "seen at this depth" means a later sibling, until a shallower
  // row is passed: whatever lies deeper then belongs to another parent.
  const hasLater = new Array<boolean>(rows.length).fill(false);
  const seen: boolean[] = [];
  for (let index = rows.length - 1; index >= 0; index -= 1) {
    const { depth } = rows[index];
    hasLater[index] = seen[depth] === true;
    seen[depth] = true;
    seen.length = depth + 1;
  }
  return { parents, hasLater };
}

/** One step of the way from the top of the tree to the chosen row: a row, and
 * the row that owns it. The step's line sits in column `depth - 1`. */
export interface ReachStep {
  index: number;
  parentIndex: number;
}

/** The chosen row and each of its ancestors below the top level, indexed by the
 * column their elbow sits in. Empty when the row is not shown. */
export function reachSteps(rows: readonly TreeRow[], index: BranchIndex, selectedPath: string | null): ReachStep[] {
  const steps: ReachStep[] = [];
  if (selectedPath === null) return steps;
  let at = rows.findIndex((row) => row.path === selectedPath);
  while (at >= 0 && rows[at].depth > 0) {
    steps[rows[at].depth - 1] = { index: at, parentIndex: index.parents[at] };
    at = index.parents[at];
  }
  return steps;
}

export interface BranchColumn {
  /** A vertical line through the whole row: this column's folder has more to come. */
  continues: boolean;
  /** The accent line over it, on the way to the chosen row. */
  reach: boolean;
}

export interface RowBranches {
  /** One per ancestor level, outermost first. The last holds the row's own elbow. */
  columns: BranchColumn[];
  /** Whether the row's own elbow is on the way to the chosen row. */
  elbowReach: boolean;
}

export function rowBranches(
  rows: readonly TreeRow[],
  index: BranchIndex,
  steps: readonly ReachStep[],
  at: number,
): RowBranches {
  const depth = rows[at].depth;
  const columns: BranchColumn[] = new Array(depth);
  let owner = at;
  for (let column = depth - 1; column >= 0; column -= 1) {
    const step = steps[column];
    columns[column] = {
      continues: owner >= 0 && index.hasLater[owner],
      reach: step !== undefined && step.parentIndex < at && at < step.index,
    };
    owner = owner >= 0 ? index.parents[owner] : -1;
  }
  return { columns, elbowReach: depth > 0 && steps[depth - 1]?.index === at };
}
