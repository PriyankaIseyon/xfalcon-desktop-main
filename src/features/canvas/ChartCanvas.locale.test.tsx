import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { FractionDigits } from "../../lib/formatNumber";
import { ChartCanvas } from "./ChartCanvas";
import type { ChartArtifact } from "./canvasStore";

vi.mock("../../lib/native", () => ({ documentExportSave: vi.fn() }));

// The real module, with both helpers pinned to German. The axis and the tooltip
// must follow the same formatter, so the test does not depend on the host locale.
vi.mock("../../lib/formatNumber", async () => {
  const actual = await vi.importActual<typeof import("../../lib/formatNumber")>("../../lib/formatNumber");
  const german = actual.createNumberFormatter("de-DE");
  return {
    ...actual,
    formatNumber: (value: number, digits?: FractionDigits) => german.format(value, digits),
    formatTick: (value: number, step: number) => german.formatTick(value, step),
  };
});

function chart(overrides: Partial<ChartArtifact> = {}): ChartArtifact {
  return {
    id: "chart-1",
    type: "chart",
    title: "Revenue by Month",
    chartType: "bar",
    data: [
      { x: "Jan", y0: 1500 },
      { x: "Feb", y0: 0 },
    ],
    xKey: "x",
    yKeys: ["y0"],
    seriesLabels: { y0: "Revenue" },
    ...overrides,
  };
}

afterEach(cleanup);

describe("locale-aware axis and tooltip", () => {
  it("writes the axis ticks and the tooltip with the same German separators", () => {
    render(<ChartCanvas artifact={chart()} />);
    const svg = document.querySelector("svg[role='img']");
    if (svg === null) throw new Error("the chart did not render an svg");

    // A step of 500 over 0 to 2,000: the tick at 1,500 reads "1,5k", not "1.5k".
    const ticks = [...svg.querySelectorAll("text[text-anchor='end']")].map((node) => node.textContent);
    expect(ticks).toEqual(["0", "500", "1k", "1,5k", "2k"]);

    const tooltip = svg.querySelector("rect")?.querySelector("title")?.textContent ?? "";
    expect(tooltip).toContain("1.500");
  });
});
