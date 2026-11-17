import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearCanvasArtifacts,
  closeCanvas,
  closeCanvasArtifact,
  getCanvasState,
  MAX_CANVAS_TABS,
  openCanvasArtifact,
  resetCanvasStore,
  setActiveCanvasTab,
  subscribeCanvas,
  toggleCanvasExpanded,
  toggleCanvasOpen,
  type ChartArtifact,
  type HtmlArtifact,
  type TableArtifact,
} from "./canvasStore";

function table(id: string): TableArtifact {
  return {
    id,
    type: "table",
    title: `Revenue ${id}`,
    columns: ["Region", "Q1"],
    rows: [["North", "120"]],
    numberFormats: [null, { decimal: "." }],
  };
}

const chart: ChartArtifact = {
  id: "art-2",
  type: "chart",
  title: "Growth Trend",
  chartType: "bar",
  data: [{ x: "Jan", y0: 10 }],
  xKey: "x",
  yKeys: ["y0"],
};

const html: HtmlArtifact = {
  id: "art-3",
  type: "html",
  title: "Dashboard",
  kind: "html",
  documentId: "doc-1",
  content: "<!DOCTYPE html><html><body>hi</body></html>",
};

beforeEach(() => {
  resetCanvasStore();
});

describe("opening artifacts", () => {
  it("opens the panel on the artifact it was handed", () => {
    openCanvasArtifact(table("art-1"));
    const state = getCanvasState();
    expect(state.isOpen).toBe(true);
    expect(state.activeArtifactId).toBe("art-1");
    expect(state.tabs).toHaveLength(1);
  });

  it("replaces a tab of the same id instead of stacking a duplicate", () => {
    openCanvasArtifact(table("art-1"));
    const updated = { ...table("art-1"), rows: [["South", "90"]] };
    openCanvasArtifact(updated);
    const state = getCanvasState();
    expect(state.tabs).toHaveLength(1);
    expect(state.tabs[0]).toEqual(updated);
  });

  it("keeps different artifact kinds side by side", () => {
    openCanvasArtifact(table("art-1"));
    openCanvasArtifact(chart);
    openCanvasArtifact(html);
    expect(getCanvasState().tabs.map((tab) => tab.type)).toEqual(["table", "chart", "html"]);
  });

  it("drops the oldest tab past the cap rather than growing without bound", () => {
    for (let index = 0; index < MAX_CANVAS_TABS + 3; index += 1) {
      openCanvasArtifact(table(`art-${index}`));
    }
    const state = getCanvasState();
    expect(state.tabs).toHaveLength(MAX_CANVAS_TABS);
    // The three earliest are gone; the newest is active.
    expect(state.tabs[0]?.id).toBe("art-3");
    expect(state.activeArtifactId).toBe(`art-${MAX_CANVAS_TABS + 2}`);
  });
});

describe("closing and switching", () => {
  it("hands focus to the tab that took the closed one's place", () => {
    openCanvasArtifact(table("a"));
    openCanvasArtifact(table("b"));
    openCanvasArtifact(table("c"));
    setActiveCanvasTab("b");

    closeCanvasArtifact("b");
    expect(getCanvasState().activeArtifactId).toBe("c");
  });

  it("falls back to the new last tab when the rightmost one closes", () => {
    openCanvasArtifact(table("a"));
    openCanvasArtifact(table("b"));
    closeCanvasArtifact("b");
    expect(getCanvasState().activeArtifactId).toBe("a");
  });

  it("leaves the active tab alone when some other tab closes", () => {
    openCanvasArtifact(table("a"));
    openCanvasArtifact(table("b"));
    setActiveCanvasTab("b");
    closeCanvasArtifact("a");
    expect(getCanvasState().activeArtifactId).toBe("b");
  });

  it("closes the panel when the last tab goes", () => {
    openCanvasArtifact(table("a"));
    closeCanvasArtifact("a");
    const state = getCanvasState();
    expect(state.tabs).toHaveLength(0);
    expect(state.isOpen).toBe(false);
    expect(state.activeArtifactId).toBeNull();
  });

  it("ignores a close or activate for an id it does not hold", () => {
    openCanvasArtifact(table("a"));
    const listener = vi.fn();
    const unsubscribe = subscribeCanvas(listener);

    closeCanvasArtifact("missing");
    setActiveCanvasTab("missing");

    expect(getCanvasState().tabs).toHaveLength(1);
    expect(getCanvasState().activeArtifactId).toBe("a");
    // No state changed, so no subscriber was woken for nothing.
    expect(listener).not.toHaveBeenCalled();
    unsubscribe();
  });
});

describe("showing and hiding", () => {
  it("toggles an open panel shut and back", () => {
    openCanvasArtifact(table("a"));
    expect(getCanvasState().isOpen).toBe(true);

    closeCanvas();
    expect(getCanvasState().isOpen).toBe(false);

    toggleCanvasOpen();
    expect(getCanvasState().isOpen).toBe(true);
  });

  it("cannot be toggled open with nothing to show", () => {
    toggleCanvasOpen();
    expect(getCanvasState().isOpen).toBe(false);
    expect(getCanvasState().tabs).toHaveLength(0);
  });

  it("expands and restores the split", () => {
    openCanvasArtifact(html);
    expect(getCanvasState().isExpanded).toBe(false);

    toggleCanvasExpanded();
    expect(getCanvasState().isExpanded).toBe(true);

    toggleCanvasExpanded();
    expect(getCanvasState().isExpanded).toBe(false);
  });

  it("gives the width back whenever the panel goes away", () => {
    // A shell still laid out for a expanded panel that is closed leaves the
    // chat hidden behind nothing.
    for (const hide of [
      () => closeCanvas(),
      () => toggleCanvasOpen(),
      () => closeCanvasArtifact(html.id),
      () => clearCanvasArtifacts(),
    ]) {
      resetCanvasStore();
      openCanvasArtifact(html);
      toggleCanvasExpanded();
      expect(getCanvasState().isExpanded).toBe(true);

      hide();
      expect(getCanvasState().isExpanded).toBe(false);
    }
  });
});

describe("clearing on a conversation change", () => {
  it("drops every artifact so one chat's data cannot show on another's screen", () => {
    openCanvasArtifact(table("a"));
    openCanvasArtifact(chart);

    clearCanvasArtifacts();

    const state = getCanvasState();
    expect(state.tabs).toHaveLength(0);
    expect(state.activeArtifactId).toBeNull();
    expect(state.isOpen).toBe(false);
  });

  it("is a no-op when there was nothing to clear", () => {
    const listener = vi.fn();
    const unsubscribe = subscribeCanvas(listener);
    clearCanvasArtifacts();
    expect(listener).not.toHaveBeenCalled();
    unsubscribe();
  });
});
