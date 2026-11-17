import { render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { getChartPalette, resetChartPaletteForTests, subscribeChartPalette, useChartPalette } from "./chartPalette";
import { ChartCanvas } from "./ChartCanvas";
import type { ChartArtifact } from "./canvasStore";

const root = () => document.documentElement;

function setTokens(tokens: Record<string, string>) {
  for (const [name, value] of Object.entries(tokens)) root().style.setProperty(name, value);
  resetChartPaletteForTests();
}

beforeEach(() => {
  root().removeAttribute("style");
  root().removeAttribute("data-theme");
  resetChartPaletteForTests();
});

afterEach(() => {
  root().removeAttribute("style");
  root().removeAttribute("data-theme");
  resetChartPaletteForTests();
});

describe("chart palette", () => {
  it("falls back to the light palette when no stylesheet answers", () => {
    const palette = getChartPalette();
    expect(palette.series).toHaveLength(7);
    expect(palette.series[0]).toBe("#1596b0");
    expect(palette.axis).toBe("#606a79");
    expect(palette.surface).toBe("#ffffff");
  });

  it("reads what the document computes, so a var() chain arrives resolved", () => {
    setTokens({
      "--xf-chart-1": "#60a5fa",
      "--xf-chart-axis": "#98a2af",
      "--xf-chart-grid": "#303640",
      "--xf-chart-surface": "#181b21",
    });

    const palette = getChartPalette();
    expect(palette.series[0]).toBe("#60a5fa");
    expect(palette.axis).toBe("#98a2af");
    expect(palette.grid).toBe("#303640");
    expect(palette.surface).toBe("#181b21");
  });

  it("re-reads when the theme changes under it", async () => {
    setTokens({ "--xf-chart-1": "#2563eb" });
    let notified = 0;
    const unsubscribe = subscribeChartPalette(() => {
      notified += 1;
    });
    expect(getChartPalette().series[0]).toBe("#2563eb");

    root().style.setProperty("--xf-chart-1", "#60a5fa");
    root().dataset.theme = "dark";

    await waitFor(() => expect(notified).toBeGreaterThan(0));
    expect(getChartPalette().series[0]).toBe("#60a5fa");
    unsubscribe();
  });

  it("stops watching once the last chart goes away", async () => {
    const unsubscribe = subscribeChartPalette(() => {});
    unsubscribe();

    let notified = 0;
    const second = subscribeChartPalette(() => {
      notified += 1;
    });
    root().dataset.theme = "dark";
    await waitFor(() => expect(notified).toBe(1));
    second();

    // Nothing is listening now, so the observer must be gone with it.
    root().dataset.theme = "light";
    await Promise.resolve();
    expect(notified).toBe(1);
  });
});

const artifact: ChartArtifact = {
  id: "chart-1",
  type: "chart",
  title: "Revenue",
  chartType: "line",
  data: [
    { month: "Jan", revenue: 10 },
    { month: "Feb", revenue: 20 },
  ],
  xKey: "month",
  yKeys: ["revenue"],
};

describe("what the chart draws with", () => {
  it("writes the resolved tokens into the SVG, never a var() reference", () => {
    setTokens({
      "--xf-chart-1": "#60a5fa",
      "--xf-chart-axis": "#98a2af",
      "--xf-chart-grid": "#303640",
      "--xf-chart-surface": "#181b21",
    });

    const { container } = render(<ChartCanvas artifact={artifact} />);
    const svg = screen.getByRole("img", { name: /Revenue, line chart/ });

    // The export serializes this markup into a standalone file, so a `var()`
    // in any presentation attribute would render black in an outside viewer.
    expect(svg.outerHTML).not.toContain("var(--");
    expect(container.querySelector('path[stroke="#60a5fa"]')).not.toBeNull();
    expect(container.querySelector('line[stroke="#303640"]')).not.toBeNull();
    expect(container.querySelector('text[fill="#98a2af"]')).not.toBeNull();
    // The ring around a point is the card behind it, not white.
    expect(container.querySelector('circle[stroke="#181b21"]')).not.toBeNull();
  });

  it("redraws in the new palette when the theme flips", async () => {
    setTokens({ "--xf-chart-1": "#2563eb" });
    function Harness() {
      const palette = useChartPalette();
      return <span data-testid="first">{palette.series[0]}</span>;
    }
    render(<Harness />);
    expect(screen.getByTestId("first")).toHaveTextContent("#2563eb");

    root().style.setProperty("--xf-chart-1", "#60a5fa");
    root().dataset.theme = "dark";

    await waitFor(() => expect(screen.getByTestId("first")).toHaveTextContent("#60a5fa"));
  });
});
