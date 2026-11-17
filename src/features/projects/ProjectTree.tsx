import React from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { AlertCircle, ChevronDown, ChevronRight, File, Folder, Link2, ShieldAlert, Slash } from "lucide-react";
import { cn } from "../../lib/cn";
import { observeScrollRect } from "../../lib/virtualScrollRect";
import { Spinner } from "../../ui/Spinner";
import { TruncatedText } from "../../ui/TruncatedText";
import { cappedName, fileTypeLabel } from "./fileTypeLabel";
import { UNOPENABLE_KIND_NOTE } from "./projectFileText";
import { indexBranches, reachSteps, type RowBranches, rowBranches } from "./treeBranches";
import type { TreeRow } from "./useProjectTree";

const ROW_HEIGHT_PX = 28;
const INDENT_PX = 16;
/** Centre of a row's disclosure column, where its children's trunk runs. */
const GUIDE_OFFSET_PX = 8;
const BASE_INDENT_PX = 8;
/** Corner radius of the elbow that turns a trunk toward a row. */
const ELBOW_RADIUS_PX = 6;
/** How far past its trunk an elbow reaches: to the folder's chevron, or, for a
 * row with none, through the empty disclosure column toward its icon. */
const FOLDER_REACH_PX = 7;
const LEAF_REACH_PX = 19;

export interface ProjectTreeProps {
  rows: TreeRow[];
  /** Listing failures by directory path, rendered under the directory that failed. */
  errors: ReadonlyMap<string, string>;
  selectedPath: string | null;
  onToggle: (row: TreeRow) => void;
  onOpenFile: (row: TreeRow) => void;
}

function isOpenable(row: TreeRow): boolean {
  return row.kind === "directory" || (row.kind === "file" && !row.isSensitive);
}

/** Index of the row that owns `rows[index]`, or -1 at the top level. */
function parentIndexOf(rows: readonly TreeRow[], index: number): number {
  const depth = rows[index]?.depth ?? 0;
  for (let candidate = index - 1; candidate >= 0; candidate -= 1) {
    if (rows[candidate].depth < depth) return candidate;
  }
  return -1;
}

function rowDomId(treeId: string, path: string): string {
  // Project-relative paths may contain spaces and other characters that are
  // not valid in an HTML id. The encoded path remains stable and unique.
  return `${treeId}-${encodeURIComponent(path)}`;
}

/**
 * Keyboard-navigable, virtualized file tree.
 *
 * Focus stays on the tree container and the active row is named by
 * `aria-activedescendant` -- the same pattern `CommandPalette` uses for its
 * listbox. Per-row focus would fight virtualization, because the element a
 * roving `tabIndex` points at may not be mounted.
 */
export function ProjectTree({ rows, errors, selectedPath, onToggle, onOpenFile }: ProjectTreeProps) {
  const treeId = React.useId();
  const treeRef = React.useRef<HTMLDivElement | null>(null);
  const [scrollElement, setScrollElement] = React.useState<HTMLDivElement | null>(null);
  // Keep the active path, not its array position. Filtering and collapsing can
  // move every later row without changing the item the reader was on.
  const [activePath, setActivePath] = React.useState<string | null>(null);
  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollElement,
    estimateSize: () => ROW_HEIGHT_PX,
    getItemKey: (index) => rows[index]?.path ?? index,
    overscan: 12,
    observeElementRect: observeScrollRect,
  });

  const branchIndex = React.useMemo(() => indexBranches(rows), [rows]);
  const reach = React.useMemo(() => reachSteps(rows, branchIndex, selectedPath), [rows, branchIndex, selectedPath]);

  const storedIndex = activePath === null ? -1 : rows.findIndex((row) => row.path === activePath);
  const activeIndex = storedIndex >= 0 ? storedIndex : rows.length > 0 ? 0 : -1;
  const activeRow = activeIndex === -1 ? undefined : rows[activeIndex];

  React.useEffect(() => {
    if (activeIndex === -1) return;
    virtualizer.scrollToIndex(activeIndex);
  }, [activeIndex, virtualizer]);

  const activate = (row: TreeRow | undefined) => {
    if (!row) return;
    if (row.kind === "directory") onToggle(row);
    else if (row.kind === "file" && !row.isSensitive) onOpenFile(row);
  };

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (rows.length === 0) return;
    const moveTo = (index: number) => {
      event.preventDefault();
      setActivePath(rows[Math.max(0, Math.min(index, rows.length - 1))]?.path ?? null);
    };

    switch (event.key) {
      case "ArrowDown":
        moveTo(activeIndex + 1);
        return;
      case "ArrowUp":
        moveTo(activeIndex - 1);
        return;
      case "Home":
        moveTo(0);
        return;
      case "End":
        moveTo(rows.length - 1);
        return;
      case "ArrowRight":
        if (activeRow?.kind === "directory" && !activeRow.expanded) {
          event.preventDefault();
          onToggle(activeRow);
        } else if (rows[activeIndex + 1]?.depth > (activeRow?.depth ?? 0)) {
          moveTo(activeIndex + 1);
        }
        return;
      case "ArrowLeft": {
        if (activeRow?.kind === "directory" && activeRow.expanded) {
          event.preventDefault();
          onToggle(activeRow);
          return;
        }
        const parentIndex = parentIndexOf(rows, activeIndex);
        if (parentIndex >= 0) moveTo(parentIndex);
        return;
      }
      case "Enter":
      case " ":
        event.preventDefault();
        activate(activeRow);
        return;
      default:
        return;
    }
  };

  return (
    <div className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden" ref={setScrollElement}>
      <div
        aria-activedescendant={activeRow ? rowDomId(treeId, activeRow.path) : undefined}
        aria-label="Project files"
        className="relative min-w-0"
        onKeyDown={handleKeyDown}
        ref={treeRef}
        role="tree"
        tabIndex={0}
        style={{ height: `${virtualizer.getTotalSize()}px` }}
      >
        {virtualizer.getVirtualItems().map((item) => {
          const row = rows[item.index];
          const openable = isOpenable(row);
          const selected = row.path === selectedPath;
          const error = errors.get(row.path);
          return (
            <div
              aria-expanded={row.kind === "directory" ? row.expanded : undefined}
              aria-busy={row.loading || undefined}
              aria-disabled={!openable || undefined}
              aria-level={row.depth + 1}
              aria-selected={openable ? selected : undefined}
              className={cn(
                "project-tree__row type-body absolute left-0 top-0 flex w-full items-center gap-1.5 pr-3",
                openable ? "cursor-pointer" : "cursor-default text-muted",
                selected && "text-foreground",
              )}
              data-cursor={row === activeRow ? "" : undefined}
              data-openable={openable ? "" : undefined}
              id={rowDomId(treeId, row.path)}
              key={row.path}
              onClick={() => {
                setActivePath(row.path);
                // Focus lives on the tree, not the row, so a mouse click has to
                // move it there or the arrow keys would go nowhere afterwards.
                treeRef.current?.focus();
                activate(row);
              }}
              role="treeitem"
              style={{
                height: `${ROW_HEIGHT_PX}px`,
                paddingLeft: `${BASE_INDENT_PX + row.depth * INDENT_PX}px`,
                // Start after the elbow's endpoint, including its leaf reach.
                ["--tree-row-indent" as string]: `${
                  row.depth === 0
                    ? BASE_INDENT_PX - 2
                    : BASE_INDENT_PX +
                      (row.depth - 1) * INDENT_PX +
                      GUIDE_OFFSET_PX +
                      (row.kind === "directory" ? FOLDER_REACH_PX : LEAF_REACH_PX) +
                      2
                }px`,
                transform: `translateY(${item.start}px)`,
              }}
            >
              {/* Branches down from each ancestor folder to its rows, so a
                  folder's rows read as its contents, not merely as rows that
                  happen to be indented. Rows are contiguous, so the per-row
                  segments join into one line. */}
              {row.depth > 0 ? (
                <TreeBranches
                  branches={rowBranches(rows, branchIndex, reach, item.index)}
                  reaches={row.kind === "directory" ? FOLDER_REACH_PX : LEAF_REACH_PX}
                />
              ) : null}
              <span className="flex w-4 shrink-0 items-center justify-center text-muted">
                {row.kind === "directory" ? (
                  row.loading ? (
                    <Spinner decorative size={12} />
                  ) : row.expanded ? (
                    <ChevronDown aria-hidden size={14} strokeWidth={1.9} />
                  ) : (
                    <ChevronRight aria-hidden size={14} strokeWidth={1.9} />
                  )
                ) : null}
              </span>
              <RowIcon kind={row.kind} path={row.path} />
              <RowName error={error !== undefined} name={row.name} />
              {row.isSensitive ? (
                <span className="flex shrink-0 items-center gap-1 text-warn">
                  <ShieldAlert aria-hidden size={13} strokeWidth={1.9} />
                  <span className="sr-only">Classified as sensitive — not read</span>
                </span>
              ) : null}
              {row.kind === "symlink" || row.kind === "other" ? (
                <span className="type-micro shrink-0 text-faint">{UNOPENABLE_KIND_NOTE[row.kind]}</span>
              ) : null}
              {error ? (
                <span className="flex min-w-0 max-w-40 shrink items-center gap-1 text-danger">
                  <AlertCircle aria-hidden className="shrink-0" size={12} strokeWidth={1.9} />
                  <TruncatedText className="type-micro" text={error}>
                    {error}
                  </TruncatedText>
                </span>
              ) : null}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/** The connector lines one row draws: a trunk through each ancestor level that
 * still has rows to come, and, in the row's own level, an elbow turning toward
 * it. Lines on the way to the chosen row are drawn again over them in the
 * accent. */
function TreeBranches({ branches, reaches }: { branches: RowBranches; reaches: number }) {
  const { columns, elbowReach } = branches;
  const middle = ROW_HEIGHT_PX / 2;
  const radius = ELBOW_RADIUS_PX;
  const trunkX = (column: number) => BASE_INDENT_PX + column * INDENT_PX + GUIDE_OFFSET_PX;
  const own = columns.length - 1;
  const turn = (x: number) => `A ${radius} ${radius} 0 0 0 ${x + radius} ${middle} H ${x + reaches}`;
  const ownX = trunkX(own);
  return (
    <svg aria-hidden className="project-tree__guide" height={ROW_HEIGHT_PX} width={ownX + reaches}>
      {columns.map((column, level) =>
        column.continues ? (
          <path className="project-tree__branch" d={`M ${trunkX(level)} 0 V ${ROW_HEIGHT_PX}`} key={`trunk-${level}`} />
        ) : null,
      )}
      {/* Where the trunk goes on past the row it is already drawn, the elbow
          starts at the bend; at a folder's last row it brings the trunk down. */}
      <path
        className="project-tree__branch"
        d={`${columns[own].continues ? `M ${ownX} ${middle - radius}` : `M ${ownX} 0 V ${middle - radius}`} ${turn(ownX)}`}
      />
      {columns.map((column, level) =>
        column.reach ? (
          <path className="project-tree__reach" d={`M ${trunkX(level)} 0 V ${ROW_HEIGHT_PX}`} key={`reach-${level}`} />
        ) : null,
      )}
      {elbowReach ? (
        <path
          className="project-tree__reach"
          d={`M ${ownX} 0 V ${middle - radius} ${turn(ownX)}`}
          data-draw=""
          pathLength={1}
        />
      ) : null}
    </svg>
  );
}

/** A folder, link, or odd entry by icon; a file by its short, coloured type
 * tag (`CSV`, `HTML`) where it has one. One fixed slot, so names line up. */
function RowIcon({ kind, path }: { kind: TreeRow["kind"]; path: string }) {
  const label = kind === "file" ? fileTypeLabel(path) : null;
  const icons: Record<TreeRow["kind"], React.ReactNode> = {
    directory: <Folder aria-hidden size={14} strokeWidth={1.8} />,
    file: <File aria-hidden size={14} strokeWidth={1.8} />,
    symlink: <Link2 aria-hidden size={14} strokeWidth={1.8} />,
    other: <Slash aria-hidden size={14} strokeWidth={1.8} />,
  };
  return (
    // A tag starts at one offset inside the slot, so `HTML` and `CSV` begin on
    // the same line, a little in from the branch that leads to them; ending
    // them on one line instead left the longer tag poking out toward it. The
    // slot holds the longest tag (`MAX_TAG_CHARS`) after that offset, so names
    // still start on one column. A bare icon keeps to the name's side.
    <span className={cn("flex w-11 shrink-0 items-center text-muted", label ? "pl-2.5" : "justify-end")}>
      {label ? (
        // The name already carries the extension; the tag is for the eye.
        <span aria-hidden className={cn("file-type-label", `file-type-label--${label.tone}`)}>
          {label.text}
        </span>
      ) : (
        icons[kind]
      )}
    </span>
  );
}

/** A long name is cut in the middle, keeping its extension, and the whole
 * name is revealed on hover; a short one reveals itself only if clipped. */
function RowName({ name, error }: { name: string; error: boolean }) {
  const shown = cappedName(name);
  const className = cn("min-w-0 flex-1", error && "text-danger");
  if (shown === name) {
    return (
      <TruncatedText className={className} text={name}>
        {name}
      </TruncatedText>
    );
  }
  // The row is named by its content, and a hover title reaches neither the
  // keyboard nor a screen reader: the whole name is spoken, the cut one drawn.
  return (
    <TruncatedText abbreviated className={className} text={name}>
      <span aria-hidden>{shown}</span>
      <span className="sr-only">{name}</span>
    </TruncatedText>
  );
}
