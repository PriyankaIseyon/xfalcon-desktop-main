import { useSyncExternalStore } from "react";
import type { NumberFormat } from "../../lib/readNumber";

/**
 * The Canvas's tab state (spec: dynamic-canvas-artifacts).
 *
 * A module store rather than React state because the openers are chat message
 * cards deep inside a virtualized transcript, and the panel is a sibling of the
 * whole workspace — threading a callback between them would mean passing it
 * through every layer that renders an artifact.
 */

export type TableArtifact = {
  id: string;
  type: "table";
  title: string;
  columns: string[];
  rows: (string | number | boolean | null)[][];
  /**
   * How each column reads, resolved by the card that produced the table from
   * the WHOLE column rather than cell by cell. A null entry is a non-measure
   * (a label column).
   *
   * Carried rather than re-derived here: a chat table's cells are all strings
   * (`richContent/detect.ts` yields `string[][]`), so `typeof cell === "number"`
   * is false for every one of them. The grid needs the same resolution the card
   * used, or a column of `100` and `50` sorts as text and puts "100" first, and
   * `1.234` sorts as text too. Absent means nothing is known, which sorts
   * everything as text.
   */
  numberFormats?: (NumberFormat | null)[];
  sourceMessageId?: string;
};

export type ChartType = "bar" | "line" | "scatter" | "pie" | "area";

export type ChartArtifact = {
  id: string;
  type: "chart";
  title: string;
  chartType: ChartType;
  /**
   * x labels are strings; y values are numbers, or `null` for a gap. Never
   * numeric text: the chat boundary reads each cell once, so nothing downstream
   * parses a string to draw a bar.
   */
  data: Record<string, string | number | null>[];
  xKey: string;
  yKeys: string[];
  seriesLabels?: Record<string, string>;
  sourceMessageId?: string;
};

export type HtmlArtifact = {
  id: string;
  type: "html";
  title: string;
  /**
   * Which preview kind the document is served as.
   *
   * Carried because the card that opens it serves BOTH an HTML document and a
   * standalone SVG through the same frame, and the scheme answers each under a
   * different content type and policy — re-registering an SVG as HTML would
   * hand it the wrong one.
   */
  kind: "html" | "svg";
  /** The preview-store id the frame loads from, refreshed on every mount. */
  documentId: string;
  /**
   * The document's own bytes.
   *
   * Held because a preview id is NOT durable: the store behind the `xf-preview`
   * scheme is bounded with least-recently-served eviction, so another document
   * can evict this one and the id is gone after a restart. The frame cannot
   * report that — the scheme answers an unknown id with a 404 whose body the
   * frame then LOADS, so `onError` never fires. Keeping the source is what lets
   * the canvas re-register and stay correct instead of silently blank.
   */
  content: string;
  sourceMessageId?: string;
};

export type CanvasArtifact = TableArtifact | ChartArtifact | HtmlArtifact;

export interface CanvasState {
  isOpen: boolean;
  activeArtifactId: string | null;
  tabs: CanvasArtifact[];
  /** Expands to 70% while retaining chat and navigation. */
  isExpanded: boolean;
}

/**
 * Most tabs the canvas will hold at once.
 *
 * A cap, not a preference: every table tab retains its full row set, so an
 * unbounded strip would let one long conversation pin an arbitrary amount of
 * data in memory. Opening past the cap drops the OLDEST tab that is not the one
 * being opened, which matches how the rest of the app bounds retained
 * artefacts (the preview store's own least-recently-served eviction).
 */
export const MAX_CANVAS_TABS = 8;

const initialState: CanvasState = {
  isOpen: false,
  activeArtifactId: null,
  tabs: [],
  isExpanded: false,
};

let currentState: CanvasState = { ...initialState };
const listeners = new Set<() => void>();

function notify() {
  for (const listener of listeners) {
    listener();
  }
}

function updateState(updater: (prev: CanvasState) => CanvasState) {
  const next = updater(currentState);
  // Referential equality means nothing changed, and `useSyncExternalStore`
  // would re-render every subscriber for a no-op without this.
  if (next === currentState) return;
  currentState = next;
  notify();
}

export function getCanvasState(): CanvasState {
  return currentState;
}

export function subscribeCanvas(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useCanvasStore(): CanvasState {
  return useSyncExternalStore(subscribeCanvas, getCanvasState, getCanvasState);
}

export function openCanvasArtifact(artifact: CanvasArtifact) {
  updateState((prev) => {
    const existingIndex = prev.tabs.findIndex((tab) => tab.id === artifact.id);
    let nextTabs =
      existingIndex >= 0
        ? prev.tabs.map((tab, index) => (index === existingIndex ? artifact : tab))
        : [...prev.tabs, artifact];

    if (nextTabs.length > MAX_CANVAS_TABS) {
      nextTabs = nextTabs.slice(nextTabs.length - MAX_CANVAS_TABS);
    }

    return {
      ...prev,
      isOpen: true,
      activeArtifactId: artifact.id,
      tabs: nextTabs,
    };
  });
}

export function closeCanvasArtifact(id: string) {
  updateState((prev) => {
    const closedIndex = prev.tabs.findIndex((tab) => tab.id === id);
    if (closedIndex < 0) return prev;
    const nextTabs = prev.tabs.filter((tab) => tab.id !== id);
    if (nextTabs.length === 0) {
      // Expansion goes with it: a shell still laid out for a panel that has
      // nothing in it leaves the chat squeezed against an empty column.
      return { ...prev, isOpen: false, isExpanded: false, activeArtifactId: null, tabs: [] };
    }

    let nextActiveId = prev.activeArtifactId;
    if (prev.activeArtifactId === id) {
      // The tab that took the closed one's place, or the new last one when the
      // rightmost tab closed — never an index past the end.
      const nextIndex = Math.min(closedIndex, nextTabs.length - 1);
      nextActiveId = nextTabs[nextIndex]?.id ?? null;
    }

    return { ...prev, tabs: nextTabs, activeArtifactId: nextActiveId };
  });
}

export function setActiveCanvasTab(id: string) {
  updateState((prev) => {
    if (prev.activeArtifactId === id && prev.isOpen) return prev;
    if (!prev.tabs.some((tab) => tab.id === id)) return prev;
    return { ...prev, activeArtifactId: id, isOpen: true };
  });
}

export function setCanvasExpanded(expanded: boolean) {
  updateState((prev) => (prev.isExpanded === expanded ? prev : { ...prev, isExpanded: expanded }));
}

export function toggleCanvasExpanded() {
  updateState((prev) => ({ ...prev, isExpanded: !prev.isExpanded }));
}

/** Shows the canvas when it has something to show, hides it otherwise. */
export function toggleCanvasOpen() {
  updateState((prev) => {
    if (prev.tabs.length === 0) {
      return prev.isOpen ? { ...prev, isOpen: false, isExpanded: false } : prev;
    }
    if (prev.isOpen) return { ...prev, isOpen: false, isExpanded: false };
    return { ...prev, isOpen: true };
  });
}

export function closeCanvas() {
  updateState((prev) => (prev.isOpen ? { ...prev, isOpen: false, isExpanded: false } : prev));
}

/**
 * Drops every tab.
 *
 * Called when the conversation changes, because an artifact belongs to the
 * conversation that produced it: leaving one open across a switch would show
 * one chat's data on another chat's screen, and after a sign-out it would show
 * the previous account's.
 */
export function clearCanvasArtifacts() {
  updateState((prev) => {
    if (prev.tabs.length === 0 && !prev.isOpen && !prev.isExpanded) return prev;
    return { ...prev, isOpen: false, isExpanded: false, activeArtifactId: null, tabs: [] };
  });
}

export function resetCanvasStore() {
  currentState = { ...initialState };
  notify();
}
