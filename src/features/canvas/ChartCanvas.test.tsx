import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { getToasts, resetToastsForTests } from "../../ui/toastStore";
import { ChartCanvas } from "./ChartCanvas";
import type { ChartArtifact } from "./canvasStore";
import { resetChartPaletteForTests } from "./chartPalette";
import { resolveTimeAxis } from "./chartScale";

const saveMock = vi.hoisted(() => vi.fn());
vi.mock("../../lib/native", () => ({ documentExportSave: saveMock }));

function chart(overrides: Partial<ChartArtifact> = {}): ChartArtifact {
  return {
    id: "chart-1",
    type: "chart",
    title: "Revenue by Month",
    chartType: "bar",
    // Numbers: the chat boundary reads each cell once, before the chart sees it.
    data: [
      { x: "Jan", y0: 100 },
      { x: "Feb", y0: 150 },
      { x: "Mar", y0: 200 },
    ],
    xKey: "x",
    yKeys: ["y0"],
    seriesLabels: { y0: "Revenue" },
    ...overrides,
  };
}

function svg(): SVGSVGElement {
  const node = document.querySelector("svg[role='img']");
  if (node === null) throw new Error("the chart did not render an svg");
  return node as SVGSVGElement;
}

/** The value-axis tick labels, in the order drawn. Only they are end-anchored. */
function yTickLabels(): string[] {
  return [...svg().querySelectorAll("text[text-anchor='end']")].map((node) => node.textContent ?? "");
}

/** The x-axis labels: the 11px text under the plot. */
function xAxisLabels(): string[] {
  return [...svg().querySelectorAll("text[font-size='11']")].map((node) => node.textContent ?? "");
}

function circleCenters(): number[] {
  return [...svg().querySelectorAll("circle")].map((node) => Number(node.getAttribute("cx")));
}

afterEach(cleanup);

beforeEach(() => {
  saveMock.mockReset();
  resetToastsForTests();
  // The palette is module state, so a token one case sets must not follow it
  // into the next.
  document.documentElement.removeAttribute("style");
  resetChartPaletteForTests();
});

describe("what the chart draws", () => {
  it("labels each series by its column header, not by its internal key", () => {
    render(<ChartCanvas artifact={chart()} />);
    expect(screen.getByText("Revenue")).toBeTruthy();
    expect(screen.queryByText("y0")).toBeNull();
  });

  it("opens on the chart type the artifact asked for", () => {
    render(<ChartCanvas artifact={chart({ chartType: "line" })} />);
    expect(screen.getByRole("radio", { name: /line/i })).toHaveAttribute("aria-checked", "true");
  });

  it("switches type without touching the data", async () => {
    const user = userEvent.setup();
    render(<ChartCanvas artifact={chart()} />);

    await user.click(screen.getByRole("radio", { name: /pie/i }));
    expect(screen.getByRole("radio", { name: /pie/i })).toHaveAttribute("aria-checked", "true");
    expect(svg().querySelectorAll("path").length).toBeGreaterThan(0);
  });

  it("says so rather than drawing an empty frame when there is nothing to plot", () => {
    render(<ChartCanvas artifact={chart({ data: [], yKeys: [] })} />);
    expect(screen.getByText("This chart has no series to draw.")).toBeTruthy();
  });

  it("draws one bar for each numeric value", () => {
    render(<ChartCanvas artifact={chart()} />);
    // Three bars, one per value; a value that is not a number would draw nothing.
    expect(svg().querySelectorAll("rect")).toHaveLength(3);
  });
});

describe("values a naive chart gets wrong", () => {
  it("draws a negative value below the zero line instead of inverting its bar", () => {
    render(
      <ChartCanvas
        artifact={chart({
          data: [
            { x: "Jan", y0: 100 },
            { x: "Feb", y0: -50 },
          ],
        })}
      />,
    );
    const bars = [...svg().querySelectorAll("rect")];
    expect(bars).toHaveLength(2);
    const heights = bars.map((bar) => Number(bar.getAttribute("height")));
    // Both bars have real height; the loss is not collapsed to a 2px stub.
    expect(heights.every((height) => height > 4)).toBe(true);
    const tops = bars.map((bar) => Number(bar.getAttribute("y")));
    // The gain starts above the loss, which is what "below zero" means in SVG
    // coordinates where y grows downward.
    expect(tops[0]!).toBeLessThan(tops[1]!);
  });

  it("draws a loss of -1200 from the zero line, not as a gap or a stub", () => {
    render(
      <ChartCanvas
        artifact={chart({
          data: [
            { x: "Jan", y0: 100 },
            { x: "Feb", y0: -1200 },
          ],
        })}
      />,
    );
    const bars = [...svg().querySelectorAll("rect")];
    expect(bars).toHaveLength(2);
    // The gain ends at zero and the loss starts there, so the loss hangs below it.
    const gainBottom = Number(bars[0]!.getAttribute("y")) + Number(bars[0]!.getAttribute("height"));
    expect(Number(bars[1]!.getAttribute("y"))).toBeCloseTo(gainBottom, 1);
    expect(Number(bars[1]!.getAttribute("height"))).toBeGreaterThan(Number(bars[0]!.getAttribute("height")));
  });

  it("leaves a gap for a missing value rather than plotting it as zero", () => {
    render(
      <ChartCanvas
        artifact={chart({
          chartType: "bar",
          data: [
            { x: "Jan", y0: 100 },
            { x: "Feb", y0: null },
            { x: "Mar", y0: 150 },
          ],
        })}
      />,
    );
    // Two bars for three categories: the blank month draws nothing, and a bar
    // sitting at zero would be a claim the data does not make.
    expect(svg().querySelectorAll("rect")).toHaveLength(2);
  });

  it("breaks a line at its holes instead of drawing across them", () => {
    render(
      <ChartCanvas
        artifact={chart({
          chartType: "line",
          data: [
            { x: "Jan", y0: 10 },
            { x: "Feb", y0: null },
            { x: "Mar", y0: 30 },
          ],
        })}
      />,
    );
    // One path per contiguous run, so a single gap yields two strokes.
    const strokes = [...svg().querySelectorAll("path")].filter((path) => path.getAttribute("fill") === "none");
    expect(strokes).toHaveLength(2);
  });

  it("refuses to draw a pie of values that cannot be shares of a whole", () => {
    render(
      <ChartCanvas
        artifact={chart({
          chartType: "pie",
          data: [
            { x: "Jan", y0: -10 },
            { x: "Feb", y0: 0 },
          ],
        })}
      />,
    );
    expect(screen.getByText("No positive values to chart")).toBeTruthy();
  });

  it("draws a lone value as a full ring rather than collapsing the path", () => {
    render(<ChartCanvas artifact={chart({ chartType: "pie", data: [{ x: "Jan", y0: 10 }] })} />);
    const slices = [...svg().querySelectorAll("path")];
    expect(slices).toHaveLength(1);
    // Two arcs, because a single arc whose endpoints coincide renders nothing.
    expect((slices[0]!.getAttribute("d") ?? "").match(/A /g)?.length).toBeGreaterThanOrEqual(4);
  });
});

describe("saving the chart", () => {
  it("writes a standalone svg through the native dialog", async () => {
    const user = userEvent.setup();
    saveMock.mockResolvedValue({ version: 1, saved: true, file_name: "Revenue by Month.svg" });
    document.documentElement.style.setProperty("--xf-chart-1", "#60a5fa");
    resetChartPaletteForTests();
    render(<ChartCanvas artifact={chart()} />);

    await user.click(screen.getByRole("button", { name: "Save file as SVG" }));

    await waitFor(() => expect(saveMock).toHaveBeenCalledTimes(1));
    const [name, markup, format] = saveMock.mock.calls[0] as [string, string, string];
    expect(name).toBe("Revenue by Month");
    expect(format).toBe("svg");
    expect(markup).toContain('xmlns="http://www.w3.org/2000/svg"');
    // Self-contained: a file that referred to this app's CSS variables or
    // Tailwind classes would open colourless in any other viewer.
    expect(markup).not.toContain("var(--xf");
    expect(markup).not.toContain("fill-muted");
    // The colour the DARK theme computes, resolved on the way in. Both halves
    // matter: the token is what answers the theme, and the literal is what
    // makes the file readable somewhere else.
    expect(markup).toContain('fill="#60a5fa"');
    expect(getToasts()[0]?.text).toBe("Saved Revenue by Month.svg.");
  });

  it("says nothing when the save dialog is dismissed", async () => {
    const user = userEvent.setup();
    saveMock.mockResolvedValue({ version: 1, saved: false, file_name: null });
    render(<ChartCanvas artifact={chart()} />);

    await user.click(screen.getByRole("button", { name: "Save file as SVG" }));

    await waitFor(() => expect(saveMock).toHaveBeenCalled());
    expect(getToasts()).toHaveLength(0);
  });

  it("reports a refused save instead of claiming one", async () => {
    const user = userEvent.setup();
    saveMock.mockRejectedValue(new Error("That location cannot be written to."));
    render(<ChartCanvas artifact={chart()} />);

    await user.click(screen.getByRole("button", { name: "Save file as SVG" }));

    await waitFor(() => expect(getToasts()[0]?.tone).toBe("error"));
  });
});

describe("the value axis", () => {
  it("puts its ticks on round numbers that reach past the data", () => {
    // 13,800 used to draw ticks at 3.7k, 7.4k and so on; a round step reads at a glance.
    render(
      <ChartCanvas
        artifact={chart({
          data: [
            { x: "Jan", y0: 0 },
            { x: "Feb", y0: 13800 },
          ],
        })}
      />,
    );
    expect(yTickLabels()).toEqual(["0", "2k", "4k", "6k", "8k", "10k", "12k", "14k"]);
  });

  it("puts round fractional ticks without floating-point noise", () => {
    render(
      <ChartCanvas
        artifact={chart({
          data: [
            { x: "Jan", y0: 0 },
            { x: "Feb", y0: 1 },
          ],
        })}
      />,
    );
    expect(yTickLabels()).toEqual(["0", "0.2", "0.4", "0.6", "0.8", "1"]);
  });

  it("draws a zero tick and a solid zero line when the values cross zero", () => {
    render(
      <ChartCanvas
        artifact={chart({
          data: [
            { x: "Jan", y0: -300 },
            { x: "Feb", y0: 1200 },
          ],
        })}
      />,
    );
    expect(yTickLabels()).toContain("0");
    const zeroLine = svg().querySelector("line[stroke-width='1.5']");
    expect(zeroLine).not.toBeNull();
    const zero = Number(zeroLine!.getAttribute("y1"));
    const [loss, gain] = [...svg().querySelectorAll("rect")];
    // The loss hangs from the zero line downward; the gain stands up from it.
    expect(Number(loss!.getAttribute("y"))).toBeCloseTo(zero, 5);
    expect(Number(gain!.getAttribute("y")) + Number(gain!.getAttribute("height"))).toBeCloseTo(zero, 5);
  });
});

describe("a time axis", () => {
  const uneven = ["2024-01-01", "2024-01-02", "2024-01-30"];

  it("places points by their date, so uneven gaps draw in proportion", () => {
    render(
      <ChartCanvas artifact={chart({ chartType: "line", data: uneven.map((x, index) => ({ x, y0: 10 + index })) })} />,
    );
    const [first, second, third] = circleCenters();
    // One day, then 28 days: the second gap is 28 times the first.
    expect((second! - first!) * 28).toBeCloseTo(third! - second!, 6);
  });

  it("labels the axis with the time axis's own ticks", () => {
    const axis = resolveTimeAxis(uneven);
    expect(axis).not.toBeNull();
    const expected = axis!.ticks.map((tick) => tick.label);
    expect(expected.length).toBeGreaterThanOrEqual(2);
    render(
      <ChartCanvas artifact={chart({ chartType: "line", data: uneven.map((x, index) => ({ x, y0: 10 + index })) })} />,
    );
    expect(xAxisLabels()).toEqual(expected);
  });

  it("draws unsorted dates in calendar order, left to right", () => {
    render(
      <ChartCanvas
        artifact={chart({
          chartType: "line",
          data: [
            { x: "2024-03", y0: 30 },
            { x: "2024-01", y0: 10 },
            { x: "2024-02", y0: 20 },
          ],
        })}
      />,
    );
    const d = svg().querySelector("path[fill='none']")?.getAttribute("d") ?? "";
    const xs = [...d.matchAll(/[ML] (\S+) (\S+)/g)].map((match) => Number(match[1]));
    expect(xs).toHaveLength(3);
    expect(xs[0]!).toBeLessThan(xs[1]!);
    expect(xs[1]!).toBeLessThan(xs[2]!);
    // The first row is March, so its point is the rightmost and the path ends on it.
    const [march, january, february] = circleCenters();
    expect(march!).toBeGreaterThan(january!);
    expect(march!).toBeGreaterThan(february!);
    expect(xs[2]!).toBeCloseTo(march!, 6);
  });

  it("places scatter points by their date too", () => {
    render(
      <ChartCanvas
        artifact={chart({ chartType: "scatter", data: uneven.map((x, index) => ({ x, y0: 10 + index })) })}
      />,
    );
    const [first, second, third] = circleCenters();
    expect((second! - first!) * 28).toBeCloseTo(third! - second!, 6);
  });

  it("keeps a bar chart over date labels categorical, in row order", () => {
    render(
      <ChartCanvas artifact={chart({ chartType: "bar", data: uneven.map((x, index) => ({ x, y0: 10 + index })) })} />,
    );
    const xs = [...svg().querySelectorAll("rect")].map((bar) => Number(bar.getAttribute("x")));
    // Equal slots: a bar is a category, so the 28-day gap is not drawn wider.
    expect(xs[1]! - xs[0]!).toBeCloseTo(xs[2]! - xs[1]!, 6);
    expect(xAxisLabels()).toEqual(uneven);
  });

  it("leaves a line over category names exactly as before", () => {
    render(
      <ChartCanvas
        artifact={chart({
          chartType: "line",
          data: [
            { x: "North", y0: 10 },
            { x: "South", y0: 20 },
            { x: "East", y0: 30 },
          ],
        })}
      />,
    );
    expect(circleCenters()).toEqual([62, 327, 592]);
    expect(xAxisLabels()).toEqual(["North", "South", "East"]);
  });
});

describe("tooltips", () => {
  it("shows a pie slice's share to one decimal place", () => {
    render(
      <ChartCanvas
        artifact={chart({
          chartType: "pie",
          data: [
            { x: "Jan", y0: 40 },
            { x: "Feb", y0: 60 },
          ],
        })}
      />,
    );
    const [first] = [...svg().querySelectorAll("path > title")].map((node) => node.textContent ?? "");
    // The separator is the host locale's; the one-decimal share is the contract.
    expect(first).toMatch(/\(40[.,]0%\)$/);
  });
});
