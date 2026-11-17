import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";
import { DataGrid } from "./DataGrid";
import type { TableArtifact } from "./canvasStore";

const saveMock = vi.hoisted(() => vi.fn());

vi.mock("../../lib/native", () => ({
  documentExportSave: saveMock,
}));

const clipboard = vi.hoisted(() => vi.fn(async (text: string) => text));
vi.mock("../chat/clipboard", () => ({ writeClipboardText: clipboard }));

/** Measures arrive as STRINGS from the chat pipeline — that is the whole reason
 * `numberFormats` is carried rather than inferred from `typeof`. */
const sales: TableArtifact = {
  id: "tbl-1",
  type: "table",
  title: "Sales Report",
  columns: ["Product", "Units", "Revenue"],
  rows: [
    ["Widget A", "100", "2,500"],
    ["Gadget B", "50", "1,500"],
    ["Alpha C", "9", "6,000"],
  ],
  numberFormats: [null, { decimal: "." }, { decimal: "." }],
};

function cellsOfColumn(column: number): string[] {
  return screen
    .getAllByRole("row")
    .slice(1)
    .map((row) => row.querySelectorAll('[role="cell"]')[column + 1]?.textContent ?? "");
}

afterEach(cleanup);

beforeEach(() => {
  saveMock.mockReset();
  clipboard.mockClear();
  resetToastsForTests();
});

describe("what the grid shows", () => {
  it("renders every column and counts the rows in words that agree with the number", () => {
    render(<DataGrid artifact={sales} />);
    expect(screen.getByRole("columnheader", { name: /sort by product/i })).toBeTruthy();
    expect(screen.getByText("3 rows · 3 columns")).toBeTruthy();
  });

  it("says one row in the singular", () => {
    render(<DataGrid artifact={{ ...sales, rows: [sales.rows[0]!] }} />);
    expect(screen.getByText("1 row · 3 columns")).toBeTruthy();
  });

  it("lines the header and the body up on one column template", () => {
    render(<DataGrid artifact={sales} />);
    const header = screen.getAllByRole("row")[0]!;
    const firstBodyRow = screen.getAllByRole("row")[1]!;
    // Same grid template on both, which is what makes the columns align — the
    // previous build measured a real <table> header against flexbox body rows.
    expect(firstBodyRow.style.gridTemplateColumns).toBe(header.style.gridTemplateColumns);
    expect(header.style.gridTemplateColumns).not.toBe("");
  });

  it("marks an empty cell rather than leaving a blank the reader must interpret", () => {
    render(<DataGrid artifact={{ ...sales, rows: [["Widget A", "", "2,500"]] }} />);
    expect(screen.getByLabelText("empty")).toBeTruthy();
  });

  it("keeps a ten-thousand-row table to a bounded mounted window", () => {
    render(
      <DataGrid
        artifact={{
          ...sales,
          rows: Array.from({ length: 10_000 }, (_, index) => [`Row ${index}`, String(index), String(index * 2)]),
        }}
      />,
    );

    expect(screen.getByText("10,000 rows · 3 columns")).toBeInTheDocument();
    expect(screen.getAllByRole("row").length).toBeLessThan(100);
  });
});

describe("filtering", () => {
  it("keeps the rows that match and announces the new count", async () => {
    const user = userEvent.setup();
    render(<DataGrid artifact={sales} />);

    await user.type(screen.getByLabelText("Filter rows"), "Widget");

    expect(screen.getByText("1 row · 3 columns")).toBeTruthy();
    expect(screen.getByRole("cell", { name: "Widget A" })).toBeTruthy();
    expect(screen.queryByRole("cell", { name: "Gadget B" })).toBeNull();
  });

  it("distinguishes an empty table from an over-filtered one", async () => {
    const user = userEvent.setup();
    render(<DataGrid artifact={sales} />);
    await user.type(screen.getByLabelText("Filter rows"), "zzz");
    expect(screen.getByText("No rows match that filter.")).toBeTruthy();

    cleanup();
    render(<DataGrid artifact={{ ...sales, rows: [] }} />);
    expect(screen.getByText("This table has no rows.")).toBeTruthy();
  });
});

describe("sorting", () => {
  it("orders a measure column as numbers, not as text", async () => {
    const user = userEvent.setup();
    render(<DataGrid artifact={sales} />);

    await user.click(screen.getByRole("columnheader", { name: /sort by units/i }));

    // Lexically "100" precedes "50" precedes "9"; numerically it is the reverse.
    expect(cellsOfColumn(1)).toEqual(["9", "50", "100"]);
  });

  it("cycles ascending, descending, then back to the table's own order", async () => {
    const user = userEvent.setup();
    render(<DataGrid artifact={sales} />);
    const header = screen.getByRole("columnheader", { name: /sort by units/i });

    await user.click(header);
    expect(header).toHaveAttribute("aria-sort", "ascending");
    expect(cellsOfColumn(1)).toEqual(["9", "50", "100"]);

    await user.click(header);
    expect(header).toHaveAttribute("aria-sort", "descending");
    expect(cellsOfColumn(1)).toEqual(["100", "50", "9"]);

    await user.click(header);
    expect(header).toHaveAttribute("aria-sort", "none");
    // Back to the order the model produced, which is the third state's point.
    expect(cellsOfColumn(0)).toEqual(["Widget A", "Gadget B", "Alpha C"]);
    expect(cellsOfColumn(1)).toEqual(["100", "50", "9"]);
  });

  it("sinks blanks in both directions, so a descending sort starts with a value", async () => {
    const user = userEvent.setup();
    render(
      <DataGrid
        artifact={{
          ...sales,
          rows: [
            ["Widget A", "", "1"],
            ["Gadget B", "50", "2"],
          ],
        }}
      />,
    );
    const header = screen.getByRole("columnheader", { name: /sort by units/i });

    await user.click(header);
    expect(cellsOfColumn(1)).toEqual(["50", "—"]);
    await user.click(header);
    expect(cellsOfColumn(1)).toEqual(["50", "—"]);
  });

  it("falls back to text order for a column with nothing known about it", async () => {
    const user = userEvent.setup();
    render(<DataGrid artifact={{ ...sales, numberFormats: undefined }} />);
    await user.click(screen.getByRole("columnheader", { name: /sort by product/i }));
    expect(cellsOfColumn(0)).toEqual(["Alpha C", "Gadget B", "Widget A"]);
  });

  it("sorts a numeric-looking column as text when no format is known for it", async () => {
    const user = userEvent.setup();
    // Text order reads `1,200` as "1" then "200", so it comes before `950`; a
    // numeric sort puts `950` first. The difference is what this case proves.
    render(
      <DataGrid
        artifact={{
          ...sales,
          rows: [
            ["Widget A", "950", "1"],
            ["Gadget B", "1,200", "2"],
          ],
          numberFormats: undefined,
        }}
      />,
    );
    await user.click(screen.getByRole("columnheader", { name: /sort by units/i }));
    expect(cellsOfColumn(1)).toEqual(["1,200", "950"]);
  });

  it("reads a signed currency column as numbers, so a loss sorts below zero", async () => {
    const user = userEvent.setup();
    render(
      <DataGrid
        artifact={{
          id: "tbl-signed",
          type: "table",
          title: "Movements",
          columns: ["Item", "Amount"],
          rows: [
            ["Refund", "$300"],
            ["Fee", "-$1,200"],
            ["Bonus", "+$5"],
          ],
          numberFormats: [null, { decimal: "." }],
        }}
      />,
    );

    await user.click(screen.getByRole("columnheader", { name: /sort by amount/i }));
    expect(cellsOfColumn(1)).toEqual(["-$1,200", "+$5", "$300"]);
  });

  it("reads a German-formatted column as numbers under its own decimal mark", async () => {
    const user = userEvent.setup();
    render(
      <DataGrid
        artifact={{
          id: "tbl-german",
          type: "table",
          title: "Mengen",
          columns: ["Artikel", "Menge"],
          rows: [
            ["A", "1.234,5"],
            ["B", "99,5"],
            ["C", "1.000,25"],
          ],
          numberFormats: [null, { decimal: "," }],
        }}
      />,
    );

    await user.click(screen.getByRole("columnheader", { name: /sort by menge/i }));
    expect(cellsOfColumn(1)).toEqual(["99,5", "1.000,25", "1.234,5"]);
  });
});

describe("handing the rows over as a file", () => {
  it("saves through the native dialog and never an anchor download", async () => {
    const user = userEvent.setup();
    saveMock.mockResolvedValue({ version: 1, saved: true, file_name: "Sales Report.csv" });
    render(<DataGrid artifact={sales} />);

    await user.click(screen.getByRole("button", { name: "Save file as CSV" }));

    await waitFor(() => expect(saveMock).toHaveBeenCalledTimes(1));
    const [name, body, format] = saveMock.mock.calls[0] as [string, string, string];
    expect(name).toBe("Sales Report");
    expect(format).toBe("csv");
    expect(body).toContain('"Product","Units","Revenue"');
    expect(getToasts()[0]?.text).toBe("Saved Sales Report.csv.");
  });

  it("exports what is on screen, not the whole table, once a filter is on", async () => {
    const user = userEvent.setup();
    saveMock.mockResolvedValue({ version: 1, saved: true, file_name: "out.csv" });
    render(<DataGrid artifact={sales} />);

    await user.type(screen.getByLabelText("Filter rows"), "Widget");
    await user.click(screen.getByRole("button", { name: "Save file as CSV" }));

    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    const [, body] = saveMock.mock.calls[0] as [string, string];
    expect(body).toContain("Widget A");
    expect(body).not.toContain("Gadget B");
  });

  it("says nothing when the save dialog is dismissed", async () => {
    const user = userEvent.setup();
    saveMock.mockResolvedValue({ version: 1, saved: false, file_name: null });
    render(<DataGrid artifact={sales} />);

    await user.click(screen.getByRole("button", { name: "Save file as CSV" }));

    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    expect(getToasts()).toHaveLength(0);
  });

  it("reports a refused save instead of claiming one", async () => {
    const user = userEvent.setup();
    saveMock.mockRejectedValue(new Error("This document is too large to export."));
    render(<DataGrid artifact={sales} />);

    await user.click(screen.getByRole("button", { name: "Save file as CSV" }));

    await waitFor(() => expect(getToasts()[0]?.tone).toBe("error"));
    expect(getToasts()[0]?.text).toContain("too large to export");
  });

  it("copies the same CSV to the clipboard", async () => {
    const user = userEvent.setup();
    render(<DataGrid artifact={sales} />);

    await user.click(screen.getByRole("button", { name: "Copy as CSV" }));

    await waitFor(() => expect(clipboard).toHaveBeenCalledTimes(1));
    expect(clipboard.mock.calls[0]?.[0]).toContain('"Widget A","100","2,500"');
    expect(getToasts()[0]?.text).toBe("Table copied as CSV.");
  });
});
