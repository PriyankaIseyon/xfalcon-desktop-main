import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CanvasPanel } from "./CanvasPanel";
import { getCanvasState, openCanvasArtifact, resetCanvasStore, type TableArtifact } from "./canvasStore";

vi.mock("../../lib/native", () => ({
  documentExportSave: vi.fn(),
  previewDocumentRegister: vi.fn(async () => "doc-1"),
  previewDocumentSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

function table(id: string, title: string, columns: string[], rows: string[][]): TableArtifact {
  return { id, type: "table", title, columns, rows, numberFormats: columns.map(() => null) };
}

const sales = table("t-1", "Sales", ["Product", "Units"], [["Widget", "1"]]);
/** Deliberately narrower than `sales`: a sort column carried over from the wider
 * table would point past this one's last column. */
const regions = table("t-2", "Regions", ["Region"], [["North"], ["South"]]);

afterEach(cleanup);

beforeEach(() => {
  resetCanvasStore();
});

describe("what the panel shows", () => {
  it("renders nothing at all with no artifacts", () => {
    const { container } = render(<CanvasPanel />);
    expect(container.firstChild).toBeNull();
  });

  it("puts one tab per artifact and renders the active one", () => {
    openCanvasArtifact(sales);
    openCanvasArtifact(regions);
    render(<CanvasPanel />);

    expect(screen.getByRole("tab", { name: /^Sales$/ })).toBeTruthy();
    expect(screen.getByRole("tab", { name: /^Regions$/ })).toBeTruthy();
    // The newest is active, so its columns are the ones on screen.
    expect(screen.getByRole("columnheader", { name: /sort by region/i })).toBeTruthy();
    expect(screen.queryByRole("columnheader", { name: /sort by product/i })).toBeNull();
  });

  it("falls back to the first tab when the active id names none of them", () => {
    openCanvasArtifact(sales);
    render(<CanvasPanel />);
    expect(screen.getByRole("columnheader", { name: /sort by product/i })).toBeTruthy();
  });
});

describe("switching tabs", () => {
  it("does not hand the next table the previous one's filter and sort", async () => {
    const user = userEvent.setup();
    openCanvasArtifact(sales);
    openCanvasArtifact(regions);
    render(<CanvasPanel />);

    // Filter and sort the narrow table.
    await user.type(screen.getByLabelText("Filter rows"), "North");
    await user.click(screen.getByRole("columnheader", { name: /sort by region/i }));
    expect(screen.getByText("1 row · 1 column")).toBeTruthy();

    await user.click(screen.getByRole("tab", { name: /^Sales$/ }));

    // A fresh renderer: no filter text, no sorted column, full row count.
    expect((screen.getByLabelText("Filter rows") as HTMLInputElement).value).toBe("");
    expect(screen.getByRole("columnheader", { name: /sort by product/i })).toHaveAttribute("aria-sort", "none");
    expect(screen.getByText("1 row · 2 columns")).toBeTruthy();
  });

  it("closes one tab without disturbing the rest", async () => {
    const user = userEvent.setup();
    openCanvasArtifact(sales);
    openCanvasArtifact(regions);
    render(<CanvasPanel />);

    await user.click(screen.getByRole("button", { name: "Close Regions" }));

    expect(screen.queryByRole("tab", { name: /^Regions$/ })).toBeNull();
    expect(screen.getByRole("columnheader", { name: /sort by product/i })).toBeTruthy();
  });

  it("collapses the panel without dropping what is in it", async () => {
    const user = userEvent.setup();
    openCanvasArtifact(sales);
    render(<CanvasPanel />);

    await user.click(screen.getByRole("button", { name: "Close side panel" }));

    const state = getCanvasState();
    expect(state.isOpen).toBe(false);
    // The tab survives, so re-opening shows the same artifact rather than
    // needing it produced again.
    expect(state.tabs).toHaveLength(1);
  });
});

describe("reaching the tabs from the keyboard", () => {
  it("puts only the selected tab in the tab order", () => {
    openCanvasArtifact(sales);
    openCanvasArtifact(regions);
    render(<CanvasPanel />);

    // A strip of focus stops between the transcript and the artifact is a
    // thicket a keyboard user has to Tab out of.
    expect(screen.getByRole("tab", { name: /^Regions$/ })).toHaveAttribute("tabindex", "0");
    expect(screen.getByRole("tab", { name: /^Sales$/ })).toHaveAttribute("tabindex", "-1");
  });

  it("moves the selection with the arrow keys, and wraps", async () => {
    const user = userEvent.setup();
    openCanvasArtifact(sales);
    openCanvasArtifact(regions);
    render(<CanvasPanel />);

    screen.getByRole("tab", { name: /^Regions$/ }).focus();
    await user.keyboard("{ArrowLeft}");
    expect(screen.getByRole("tab", { name: /^Sales$/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: /^Sales$/ })).toHaveFocus();

    await user.keyboard("{ArrowLeft}");
    expect(screen.getByRole("tab", { name: /^Regions$/ })).toHaveAttribute("aria-selected", "true");
  });

  it("jumps to the ends with Home and End", async () => {
    const user = userEvent.setup();
    openCanvasArtifact(sales);
    openCanvasArtifact(regions);
    render(<CanvasPanel />);

    screen.getByRole("tab", { name: /^Regions$/ }).focus();
    await user.keyboard("{Home}");
    expect(screen.getByRole("tab", { name: /^Sales$/ })).toHaveAttribute("aria-selected", "true");

    await user.keyboard("{End}");
    expect(screen.getByRole("tab", { name: /^Regions$/ })).toHaveAttribute("aria-selected", "true");
  });

  it("closes the focused tab with Delete, so the mouse is not the only way", async () => {
    const user = userEvent.setup();
    openCanvasArtifact(sales);
    openCanvasArtifact(regions);
    render(<CanvasPanel />);

    screen.getByRole("tab", { name: /^Regions$/ }).focus();
    await user.keyboard("{Delete}");

    expect(screen.queryByRole("tab", { name: /^Regions$/ })).toBeNull();
    expect(screen.getByRole("tab", { name: /^Sales$/ })).toHaveAttribute("aria-selected", "true");
    expect(screen.getByRole("tab", { name: /^Sales$/ })).toHaveFocus();
  });

  it("names the panel its tabs control", () => {
    openCanvasArtifact(sales);
    render(<CanvasPanel />);
    const tab = screen.getByRole("tab", { name: /^Sales$/ });
    const panel = screen.getByRole("tabpanel");
    expect(tab.getAttribute("aria-controls")).toBe(panel.getAttribute("id"));
    expect(panel.getAttribute("aria-labelledby")).toBe(tab.getAttribute("id"));
  });
});

describe("how much room the panel takes", () => {
  it("toggles expansion, and says which state it is in", async () => {
    const user = userEvent.setup();
    openCanvasArtifact(sales);
    render(<CanvasPanel />);

    const toggle = screen.getByRole("button", { name: "Expand panel" });
    expect(toggle).toHaveAttribute("aria-pressed", "false");

    await user.click(toggle);

    expect(getCanvasState().isExpanded).toBe(true);
    const pressed = screen.getByRole("button", { name: "Restore panel width" });
    expect(pressed).toHaveAttribute("aria-pressed", "true");
  });

  it("restores the split on Escape, so the one button is not the only exit", async () => {
    const user = userEvent.setup();
    openCanvasArtifact(sales);
    render(<CanvasPanel />);
    await user.click(screen.getByRole("button", { name: "Expand panel" }));
    expect(getCanvasState().isExpanded).toBe(true);

    await user.keyboard("{Escape}");

    expect(getCanvasState().isExpanded).toBe(false);
  });

  it("does not listen for Escape while sharing the width", async () => {
    const user = userEvent.setup();
    openCanvasArtifact(sales);
    render(<CanvasPanel />);

    await user.keyboard("{Escape}");

    // Nothing to exit, and no listener to swallow an Escape a dialog wanted.
    expect(getCanvasState().isExpanded).toBe(false);
    expect(getCanvasState().isOpen).toBe(true);
  });

  it("offers no device simulation, which is a builder's tool not a reader's", () => {
    openCanvasArtifact(sales);
    render(<CanvasPanel />);
    expect(screen.queryByRole("radio", { name: /desktop|tablet|mobile/i })).toBeNull();
  });
});
