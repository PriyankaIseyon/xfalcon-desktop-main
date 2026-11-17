import React from "react";
import type { ProjectDirectoryEntry, ProjectEntryKind } from "../../lib/generated/contracts";
import { projectListDirectory } from "../../lib/native";
import { describeProjectError } from "./projectFileText";

/** One visible line of the tree, already flattened for a virtualized list. */
export interface TreeRow {
  /** Project-relative, `/`-separated. Unique, so it doubles as the row key. */
  path: string;
  name: string;
  kind: ProjectEntryKind;
  sizeBytes: number;
  isSensitive: boolean;
  /** 0 for a child of the trusted root. */
  depth: number;
  expanded: boolean;
  loading: boolean;
}

export interface ProjectTreeState {
  rows: TreeRow[];
  /** Status of the trusted root's own listing. */
  rootStatus: "loading" | "ready" | "error";
  /** Listing failures, keyed by directory path (`""` is the root). */
  errors: ReadonlyMap<string, string>;
  toggle: (row: TreeRow) => void;
  /** Closes every folder, and stops opening new ones by default until reload. */
  collapseAll: () => void;
  /** Opens every folder that opens by default, reading those not yet read,
   * within the same budget as the first load. */
  expandAll: () => void;
  /** Whether any folder is open, so there is something to collapse. */
  anyExpanded: boolean;
  reload: () => void;
}

const ROOT = "";

/** How many folders one tree opens on its own. A person opens the rest. */
export const MAX_AUTO_EXPANDED_DIRECTORIES = 100;

/** Folders a tool regenerates and a person rarely reads, which can hold far
 * more entries than everything else together. Dot-folders (`.git`, `.venv`)
 * are skipped by their leading dot. */
const NOT_AUTO_EXPANDED = new Set(["node_modules", "bower_components", "__pycache__", "target", "venv"]);

function opensByDefault(entry: ProjectDirectoryEntry): boolean {
  return (
    entry.kind === "directory" &&
    !entry.is_sensitive &&
    !entry.name.startsWith(".") &&
    !NOT_AUTO_EXPANDED.has(entry.name.toLowerCase())
  );
}

// Built once: `localeCompare` with options builds a collator on every call, which
// dominates the sort of a large listing.
const NAME_COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

function sortEntries(entries: readonly ProjectDirectoryEntry[]): ProjectDirectoryEntry[] {
  return [...entries].sort((left, right) => {
    const leftIsDirectory = left.kind === "directory";
    if (leftIsDirectory !== (right.kind === "directory")) return leftIsDirectory ? -1 : 1;
    return NAME_COLLATOR.compare(left.name, right.name);
  });
}

/**
 * Lazy directory tree over `project_list_directory`.
 *
 * Folders open by default, one level per call as each listing lands, up to
 * {@link MAX_AUTO_EXPANDED_DIRECTORIES} and never into a dot-folder or a
 * dependency or build folder — so a project with a deep `node_modules` costs a
 * bounded number of listings, not a crawl. A folder the person closes stays
 * closed, and `collapseAll` stops the default opening until `reload`. Children
 * are cached per directory, so collapsing and re-expanding does not re-read the
 * disk; `reload` is the deliberate way to see changes made outside the app.
 */
export function useProjectTree(projectId: string | null): ProjectTreeState {
  const [children, setChildren] = React.useState<ReadonlyMap<string, ProjectDirectoryEntry[]>>(new Map());
  const [expanded, setExpanded] = React.useState<ReadonlySet<string>>(new Set());
  const [pending, setPending] = React.useState<ReadonlySet<string>>(new Set());
  const [errors, setErrors] = React.useState<ReadonlyMap<string, string>>(new Map());
  const [reloadToken, setReloadToken] = React.useState(0);
  // Bumped whenever the tree is thrown away. An in-flight listing compares
  // against it before applying, so a switch to another project can never have
  // the previous project's directory land in the new project's tree.
  const generationRef = React.useRef(0);
  // Expansion can be toggled again before its first listing settles. Keep one
  // request per directory and generation so two responses cannot race and the
  // first completion cannot hide the second request's loading state.
  const inFlightRef = React.useRef(new Set<string>());
  // The generation whose listings still open their folders by default, and how
  // many more folders it may open. `collapseAll` retires the former.
  const autoExpandRef = React.useRef({ generation: 0, remaining: MAX_AUTO_EXPANDED_DIRECTORIES });

  const fetchDirectory = React.useCallback(async (project: string, path: string, generation: number) => {
    const requestKey = `${generation}\u0000${path}`;
    if (inFlightRef.current.has(requestKey)) return;
    inFlightRef.current.add(requestKey);
    setPending((current) => new Set(current).add(path));
    try {
      const listing = await projectListDirectory(project, path === ROOT ? null : path);
      if (generationRef.current !== generation) return;
      const entries = sortEntries(listing.entries);
      setChildren((current) => new Map(current).set(path, entries));
      const auto = autoExpandRef.current;
      if (auto.generation === generation && auto.remaining > 0) {
        const opened = entries
          .filter(opensByDefault)
          .slice(0, auto.remaining)
          .map((entry) => (path === ROOT ? entry.name : `${path}/${entry.name}`));
        auto.remaining -= opened.length;
        if (opened.length > 0) {
          setExpanded((current) => {
            const next = new Set(current);
            for (const child of opened) next.add(child);
            return next;
          });
          for (const child of opened) void fetchDirectory(project, child, generation);
        }
      }
      setErrors((current) => {
        if (!current.has(path)) return current;
        const next = new Map(current);
        next.delete(path);
        return next;
      });
    } catch (cause: unknown) {
      if (generationRef.current !== generation) return;
      setErrors((current) =>
        new Map(current).set(path, describeProjectError(cause, "This folder could not be listed.")),
      );
    } finally {
      inFlightRef.current.delete(requestKey);
      if (generationRef.current === generation) {
        setPending((current) => {
          const next = new Set(current);
          next.delete(path);
          return next;
        });
      }
    }
  }, []);

  React.useEffect(() => {
    generationRef.current += 1;
    const generation = generationRef.current;
    setChildren(new Map());
    setExpanded(new Set());
    setPending(new Set());
    setErrors(new Map());
    autoExpandRef.current = { generation, remaining: MAX_AUTO_EXPANDED_DIRECTORIES };
    if (projectId == null) return;
    void fetchDirectory(projectId, ROOT, generation);
    return () => {
      // Retires this generation so anything still in flight is discarded on
      // unmount as well as on a project switch.
      generationRef.current += 1;
    };
  }, [projectId, reloadToken, fetchDirectory]);

  const rows = React.useMemo(() => {
    const flattened: TreeRow[] = [];
    const walk = (directory: string, depth: number) => {
      for (const entry of children.get(directory) ?? []) {
        const path = directory === ROOT ? entry.name : `${directory}/${entry.name}`;
        const isExpanded = expanded.has(path);
        flattened.push({
          path,
          name: entry.name,
          kind: entry.kind,
          sizeBytes: entry.size_bytes,
          isSensitive: entry.is_sensitive,
          depth,
          expanded: isExpanded,
          loading: pending.has(path),
        });
        if (entry.kind === "directory" && isExpanded) walk(path, depth + 1);
      }
    };
    walk(ROOT, 0);
    return flattened;
  }, [children, expanded, pending]);

  const toggle = React.useCallback(
    (row: TreeRow) => {
      if (projectId == null || row.kind !== "directory") return;
      const opening = !expanded.has(row.path);
      setExpanded((current) => {
        const next = new Set(current);
        if (opening) next.add(row.path);
        else next.delete(row.path);
        return next;
      });
      if (opening && !children.has(row.path)) {
        void fetchDirectory(projectId, row.path, generationRef.current);
      }
    },
    [children, expanded, fetchDirectory, projectId],
  );

  const collapseAll = React.useCallback(() => {
    autoExpandRef.current = { generation: 0, remaining: 0 };
    setExpanded(new Set());
  }, []);

  const expandAll = React.useCallback(() => {
    if (projectId == null) return;
    const generation = generationRef.current;
    const auto = { generation, remaining: MAX_AUTO_EXPANDED_DIRECTORIES };
    autoExpandRef.current = auto;
    const opened: string[] = [];
    const unread: string[] = [];
    for (const [directory, entries] of children) {
      for (const entry of entries) {
        if (!opensByDefault(entry)) continue;
        const path = directory === ROOT ? entry.name : `${directory}/${entry.name}`;
        if (!children.has(path)) {
          if (auto.remaining === 0) continue;
          auto.remaining -= 1;
          unread.push(path);
        }
        opened.push(path);
      }
    }
    setExpanded((current) => new Set([...current, ...opened]));
    // Each listing that lands opens its own children in turn, while the budget lasts.
    for (const path of unread) void fetchDirectory(projectId, path, generation);
  }, [children, fetchDirectory, projectId]);

  const reload = React.useCallback(() => setReloadToken((token) => token + 1), []);

  const rootStatus = errors.has(ROOT) ? "error" : children.has(ROOT) && !pending.has(ROOT) ? "ready" : "loading";

  return { rows, rootStatus, errors, toggle, collapseAll, expandAll, anyExpanded: expanded.size > 0, reload };
}
