import { useMemo, useRef, useState } from "react";
import { AreaChart, BarChart3, Download, LineChart, PieChart, ScatterChart } from "lucide-react";
import { Button } from "../../ui/Button";
import { SegmentedControl } from "../../ui/SegmentedControl";
import { documentExportSave } from "../../lib/native";
import { formatNumber, formatTick } from "../../lib/formatNumber";
import { toast } from "../../ui/toastStore";
import { niceScale, resolveTimeAxis, type TimeAxis } from "./chartScale";
import { useChartPalette, type ChartPalette } from "./chartPalette";
import type { ChartArtifact, ChartType } from "./canvasStore";

export interface ChartCanvasProps {
  artifact: ChartArtifact;
}

/* Colours come from `--xf-chart-*`, resolved to literal values before they
   reach the SVG (see `chartPalette.ts`). The tokens answer theme and contrast;
   resolving them keeps the export self-contained, because a `.svg` carrying
   `var(--xf-chart-1)` renders black in a viewer that has none of this app's
   stylesheets. Geometry below is fixed: it is the drawing, not the theme. */
const PLOT_WIDTH = 620;
const PLOT_HEIGHT = 360;
const PADDING = { top: 28, right: 28, bottom: 44, left: 62 };
const INNER_WIDTH = PLOT_WIDTH - PADDING.left - PADDING.right;
const INNER_HEIGHT = PLOT_HEIGHT - PADDING.top - PADDING.bottom;
/** Longest x-axis label drawn before it is elided; past this the labels overlap
 * into an unreadable band. */
const MAX_AXIS_LABEL_CHARS = 12;
/** Pie shares are shown to one decimal place, the precision a reader can use. */
const SHARE_DIGITS = { minimumFractionDigits: 1, maximumFractionDigits: 1 };

const CHART_OPTIONS = [
  { value: "bar" as const, label: "Bar", icon: <BarChart3 aria-hidden size={14} strokeWidth={1.8} /> },
  { value: "line" as const, label: "Line", icon: <LineChart aria-hidden size={14} strokeWidth={1.8} /> },
  { value: "area" as const, label: "Area", icon: <AreaChart aria-hidden size={14} strokeWidth={1.8} /> },
  { value: "scatter" as const, label: "Scatter", icon: <ScatterChart aria-hidden size={14} strokeWidth={1.8} /> },
  { value: "pie" as const, label: "Pie", icon: <PieChart aria-hidden size={14} strokeWidth={1.8} /> },
];

/** Where a point sits, and whether it exists at all. */
type Point = { value: number | null };

/**
 * A chart over one chart artifact (spec: dynamic-canvas-artifacts).
 *
 * Hand-drawn SVG rather than a charting library: the artifact needs four chart
 * types over a handful of series, the app already carries 900 kB of JavaScript,
 * and the export has to be a standalone file — which rules out any renderer
 * whose output depends on its own runtime stylesheet.
 */
export function ChartCanvas({ artifact }: ChartCanvasProps) {
  const palette = useChartPalette();
  const [chartType, setChartType] = useState<ChartType>(artifact.chartType);
  /** Which x position is hovered, and which series — a point belongs to both,
   * and keying on the index alone lights up every series at that x. */
  const [hover, setHover] = useState<{ series: number; index: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const svgRef = useRef<SVGSVGElement>(null);

  const { data, xKey, yKeys } = artifact;

  /**
   * Every series as numbers, with a hole where a value is missing.
   *
   * `null` rather than `0`: a gap plotted at zero is a claim the data does not
   * make, and on a revenue chart it reads as a month with no sales rather than
   * a month with no figure.
   */
  const series = useMemo(
    () =>
      yKeys.map((key) => ({
        key,
        points: data.map((row): Point => {
          const raw = row[key];
          return { value: typeof raw === "number" && Number.isFinite(raw) ? raw : null };
        }),
      })),
    [data, yKeys],
  );

  /**
   * The value axis, which must contain zero.
   *
   * Taking only the maximum and flooring it at zero — the obvious version —
   * draws a bar of negative height for every loss in the data and puts it below
   * the axis at two pixels tall. A domain that spans the real minimum and
   * maximum is what makes a profit-and-loss chart honest.
   *
   * `niceScale` then widens that span out to round ticks, so a gridline reads as
   * a value (2k, 4k) rather than as a fraction of the data's range.
   */
  const scale = useMemo(() => {
    let min = 0;
    let max = 0;
    for (const line of series) {
      for (const point of line.points) {
        if (point.value === null) continue;
        if (point.value < min) min = point.value;
        if (point.value > max) max = point.value;
      }
    }
    return niceScale(min, max);
  }, [series]);

  /**
   * The date axis, when every label is a date of one granularity. Bars and pies
   * stay categorical: a bar's slot is its category.
   */
  const timeAxis = useMemo(
    () =>
      chartType === "line" || chartType === "area" || chartType === "scatter"
        ? resolveTimeAxis(data.map((row) => String(row[xKey] ?? "")))
        : null,
    [data, xKey, chartType],
  );

  const yFor = (value: number) => {
    const ratio = (value - scale.min) / (scale.max - scale.min);
    return PADDING.top + INNER_HEIGHT * (1 - ratio);
  };
  const baselineY = yFor(Math.max(scale.min, Math.min(0, scale.max)));

  const xForPosition = (axis: TimeAxis, position: number) =>
    PADDING.left + ((position - axis.min) / (axis.max - axis.min)) * INNER_WIDTH;

  const xFor = (index: number) => {
    if (timeAxis !== null) return xForPosition(timeAxis, timeAxis.positions[index]!);
    return PADDING.left + (data.length <= 1 ? INNER_WIDTH / 2 : (index / (data.length - 1)) * INNER_WIDTH);
  };

  /** The order line and area points are joined in: chronological on a time axis, row order otherwise. */
  const drawOrder = timeAxis?.order ?? data.map((_, index) => index);

  const axisLabel = (index: number) => {
    const raw = String(data[index]?.[xKey] ?? "");
    return raw.length > MAX_AXIS_LABEL_CHARS ? `${raw.slice(0, MAX_AXIS_LABEL_CHARS - 1)}…` : raw;
  };

  /** Draw at most this many x labels, evenly spaced, so a long series does not
   * print its labels on top of one another. */
  const labelStride = Math.max(1, Math.ceil(data.length / 12));

  const seriesLabel = (key: string) => artifact.seriesLabels?.[key] ?? key;
  const seriesColor = (index: number) => colorAt(palette, index);

  /**
   * Saves the chart through the native dialog.
   *
   * The serialized markup is self-contained by construction — every colour and
   * font size is a presentation attribute on the element, never a Tailwind class
   * or a CSS custom property — because the exported file opens in a viewer that
   * has none of this app's styles. An `xmlns` is added on the way out: the live
   * DOM does not need one, a standalone file does.
   */
  const saveSvg = async () => {
    if (busyRef.current) return;
    const node = svgRef.current;
    if (node === null) return;
    busyRef.current = true;
    setBusy(true);
    try {
      const clone = node.cloneNode(true) as SVGSVGElement;
      clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
      clone.setAttribute("width", String(PLOT_WIDTH));
      clone.setAttribute("height", String(PLOT_HEIGHT));
      const markup = `<?xml version="1.0" encoding="UTF-8"?>\n${new XMLSerializer().serializeToString(clone)}`;
      const result = await documentExportSave(artifact.title, markup, "svg");
      if (result.saved) toast.success(`Saved ${result.file_name ?? "the image"}.`);
    } catch (cause) {
      toast.error(cause instanceof Error ? cause.message : "The chart could not be saved.");
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  };

  const dimmed = (seriesIndex: number, index: number) =>
    hover !== null && !(hover.series === seriesIndex && hover.index === index) ? 0.35 : 0.9;

  return (
    <div className="flex h-full w-full flex-col overflow-hidden bg-card text-foreground">
      <div className="canvas-toolbar">
        <SegmentedControl<ChartType>
          aria-label="Chart type"
          className="canvas-segmented-control"
          onChange={setChartType}
          options={CHART_OPTIONS}
          value={chartType}
          variant="solid"
        />
        <Button
          aria-label="Save file as SVG"
          loading={busy}
          onClick={() => void saveSvg()}
          size="sm"
          variant="secondary"
        >
          <Download aria-hidden size={14} strokeWidth={1.8} />
          Save file
        </Button>
      </div>

      <div className="relative flex min-h-0 flex-1 flex-col items-center justify-center overflow-auto p-6">
        {data.length === 0 || yKeys.length === 0 ? (
          <p className="text-xs text-muted">This chart has no series to draw.</p>
        ) : (
          <svg
            ref={svgRef}
            aria-label={`${artifact.title}, ${chartType} chart`}
            className="h-auto w-full max-w-[680px]"
            role="img"
            viewBox={`0 0 ${PLOT_WIDTH} ${PLOT_HEIGHT}`}
          >
            {chartType === "pie" ? null : (
              <g>
                {scale.ticks.map((value) => {
                  const y = yFor(value);
                  return (
                    <g key={value}>
                      <line
                        stroke={palette.grid}
                        strokeDasharray="3 3"
                        strokeWidth="1"
                        x1={PADDING.left}
                        x2={PLOT_WIDTH - PADDING.right}
                        y1={y}
                        y2={y}
                      />
                      <text
                        fill={palette.axis}
                        fontFamily="inherit"
                        fontSize="10"
                        textAnchor="end"
                        x={PADDING.left - 8}
                        y={y + 3}
                      >
                        {formatTick(value, scale.step)}
                      </text>
                    </g>
                  );
                })}
                {/* The zero line, drawn solid, so a chart that crosses it says
                    where the crossing is. */}
                {scale.min < 0 ? (
                  <line
                    stroke={palette.axis}
                    strokeWidth="1.5"
                    x1={PADDING.left}
                    x2={PLOT_WIDTH - PADDING.right}
                    y1={baselineY}
                    y2={baselineY}
                  />
                ) : null}
              </g>
            )}

            {chartType === "bar" ? (
              <g>
                {data.map((_, index) => {
                  const slot = INNER_WIDTH / data.length;
                  const barWidth = Math.max(2, Math.min(26, (slot * 0.72) / yKeys.length));
                  const groupX = PADDING.left + index * slot;
                  const groupLeft = groupX + (slot - barWidth * yKeys.length) / 2;
                  return (
                    <g key={index}>
                      {series.map((line, seriesIndex) => {
                        const value = line.points[index]?.value;
                        if (value === null || value === undefined) return null;
                        const y = yFor(value);
                        // Height from the baseline, so a negative value draws
                        // downward from zero instead of inverting the bar.
                        const top = Math.min(y, baselineY);
                        const height = Math.max(1, Math.abs(baselineY - y));
                        return (
                          <rect
                            key={line.key}
                            fill={seriesColor(seriesIndex)}
                            height={height}
                            opacity={dimmed(seriesIndex, index)}
                            rx={2}
                            width={Math.max(1, barWidth - 2)}
                            x={groupLeft + seriesIndex * barWidth}
                            y={top}
                            onMouseEnter={() => setHover({ series: seriesIndex, index })}
                            onMouseLeave={() => setHover(null)}
                          >
                            <title>{`${seriesLabel(line.key)} · ${String(data[index]?.[xKey] ?? "")}: ${formatNumber(value)}`}</title>
                          </rect>
                        );
                      })}
                    </g>
                  );
                })}
              </g>
            ) : null}

            {chartType === "scatter" ? (
              <g>
                {series.map((line, seriesIndex) =>
                  line.points.map((point, index) =>
                    point.value === null ? null : (
                      <circle
                        key={`${line.key}-${index}`}
                        cx={xFor(index)}
                        cy={yFor(point.value)}
                        fill={seriesColor(seriesIndex)}
                        opacity={dimmed(seriesIndex, index)}
                        r={hover?.series === seriesIndex && hover.index === index ? 6 : 4}
                        onMouseEnter={() => setHover({ series: seriesIndex, index })}
                        onMouseLeave={() => setHover(null)}
                      >
                        <title>{`${seriesLabel(line.key)} · ${String(data[index]?.[xKey] ?? "")}: ${formatNumber(point.value)}`}</title>
                      </circle>
                    ),
                  ),
                )}
              </g>
            ) : null}

            {chartType === "line" || chartType === "area" ? (
              <g>
                {series.map((line, seriesIndex) => {
                  const color = seriesColor(seriesIndex);
                  // Split at every hole, so a missing value leaves a gap in the
                  // line rather than a straight segment across it.
                  const runs = contiguousRuns(line.points, drawOrder);
                  return (
                    <g key={line.key}>
                      {runs.map((run, runIndex) => {
                        const path = run
                          .map(
                            ({ index, value }, position) =>
                              `${position === 0 ? "M" : "L"} ${xFor(index)} ${yFor(value)}`,
                          )
                          .join(" ");
                        return (
                          <g key={runIndex}>
                            {chartType === "area" && run.length > 1 ? (
                              <path
                                d={`${path} L ${xFor(run[run.length - 1]!.index)} ${baselineY} L ${xFor(run[0]!.index)} ${baselineY} Z`}
                                fill={color}
                                opacity={0.14}
                              />
                            ) : null}
                            <path
                              d={path}
                              fill="none"
                              stroke={color}
                              strokeLinecap="round"
                              strokeLinejoin="round"
                              strokeWidth="2.5"
                            />
                          </g>
                        );
                      })}
                      {line.points.map((point, index) =>
                        point.value === null ? null : (
                          <circle
                            key={index}
                            cx={xFor(index)}
                            cy={yFor(point.value)}
                            fill={color}
                            opacity={dimmed(seriesIndex, index)}
                            r={hover?.series === seriesIndex && hover.index === index ? 5 : 3.5}
                            stroke={palette.surface}
                            strokeWidth="1.5"
                            onMouseEnter={() => setHover({ series: seriesIndex, index })}
                            onMouseLeave={() => setHover(null)}
                          >
                            <title>{`${seriesLabel(line.key)} · ${String(data[index]?.[xKey] ?? "")}: ${formatNumber(point.value)}`}</title>
                          </circle>
                        ),
                      )}
                    </g>
                  );
                })}
              </g>
            ) : null}

            {chartType === "pie" ? <PieSlices data={data} palette={palette} series={series} xKey={xKey} /> : null}

            {chartType === "pie" ? null : (
              <g>
                {timeAxis !== null
                  ? timeAxis.ticks.map((tick) => {
                      const x = xForPosition(timeAxis, tick.position);
                      return (
                        <text
                          key={tick.position}
                          fill={palette.axis}
                          fontSize="11"
                          textAnchor={tickAnchor(x)}
                          x={x}
                          y={PLOT_HEIGHT - PADDING.bottom + 18}
                        >
                          {tick.label}
                        </text>
                      );
                    })
                  : data.map((_, index) =>
                      index % labelStride === 0 ? (
                        <text
                          key={index}
                          fill={palette.axis}
                          fontSize="11"
                          textAnchor="middle"
                          x={chartType === "bar" ? barGroupCenter(index, data.length) : xFor(index)}
                          y={PLOT_HEIGHT - PADDING.bottom + 18}
                        >
                          {axisLabel(index)}
                        </text>
                      ) : null,
                    )}
              </g>
            )}
          </svg>
        )}

        <ul className="mt-4 flex list-none flex-wrap items-center justify-center gap-4 p-0">
          {yKeys.map((key, index) => (
            <li className="flex items-center gap-1.5 text-xs text-secondary" key={key}>
              <span aria-hidden className="size-2.5 rounded-full" style={{ backgroundColor: seriesColor(index) }} />
              <span className="font-medium">{seriesLabel(key)}</span>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}

function barGroupCenter(index: number, count: number): number {
  const slot = INNER_WIDTH / count;
  return PADDING.left + index * slot + slot / 2;
}

/**
 * Runs of consecutive points that have a value, so a line can skip its holes.
 *
 * Walks `order`, so a time-sorted line breaks at its neighbours in time rather
 * than in the order the rows arrived. Each point keeps its original index.
 */
function contiguousRuns(points: readonly Point[], order: readonly number[]): { index: number; value: number }[][] {
  const runs: { index: number; value: number }[][] = [];
  let current: { index: number; value: number }[] = [];
  for (const index of order) {
    const value = points[index]?.value ?? null;
    if (value === null) {
      if (current.length > 0) runs.push(current);
      current = [];
      continue;
    }
    current.push({ index, value });
  }
  if (current.length > 0) runs.push(current);
  return runs;
}

/** A label within 30px of either plot edge hangs inward, so the frame does not clip it. */
function tickAnchor(x: number): "start" | "middle" | "end" {
  if (x - PADDING.left < 30) return "start";
  if (PLOT_WIDTH - PADDING.right - x < 30) return "end";
  return "middle";
}

/**
 * Fixed order, never cycled (xfalcon-design DESIGN.md 7.6): the last palette
 * entry is the overflow grey, so a 7th series and beyond share it instead of
 * repainting series 1.
 */
function colorAt(palette: ChartPalette, index: number): string {
  return palette.series[Math.min(index, palette.series.length - 1)]!;
}

/**
 * A donut of the FIRST series.
 *
 * One series, because a pie of several would be several pies. Negative values
 * are dropped rather than drawn: a slice cannot be a negative share of a whole,
 * and rendering one as its absolute value would state the opposite of the data.
 */
function PieSlices({
  series,
  data,
  palette,
  xKey,
}: {
  series: readonly { key: string; points: readonly Point[] }[];
  data: readonly Record<string, string | number | null>[];
  palette: ChartPalette;
  xKey: string;
}) {
  const first = series[0];
  const slices = (first?.points ?? []).map((point, index) => ({
    index,
    value: point.value !== null && point.value > 0 ? point.value : 0,
  }));
  const total = slices.reduce((sum, slice) => sum + slice.value, 0);

  if (total === 0) {
    return (
      <text fill={palette.axis} fontSize="12" textAnchor="middle" x={PLOT_WIDTH / 2} y={PLOT_HEIGHT / 2}>
        No positive values to chart
      </text>
    );
  }

  const radius = 108;
  const innerRadius = 56;
  let startAngle = -Math.PI / 2;

  return (
    <g transform={`translate(${PLOT_WIDTH / 2}, ${PLOT_HEIGHT / 2 - 8})`}>
      {slices.map((slice) => {
        if (slice.value === 0) return null;
        const sweep = (slice.value / total) * 2 * Math.PI;
        const endAngle = startAngle + sweep;
        const path = donutSlice(startAngle, endAngle, radius, innerRadius);
        startAngle = endAngle;
        const share = (slice.value / total) * 100;
        return (
          <path
            key={slice.index}
            d={path}
            fill={colorAt(palette, slice.index)}
            opacity={0.9}
            stroke={palette.surface}
            strokeWidth="1"
          >
            <title>{`${String(data[slice.index]?.[xKey] ?? "")}: ${formatNumber(slice.value)} (${formatNumber(share, SHARE_DIGITS)}%)`}</title>
          </path>
        );
      })}
    </g>
  );
}

/**
 * One donut segment.
 *
 * A slice of a full turn cannot be drawn as a single arc — the endpoints
 * coincide and the path collapses — so a lone non-zero value becomes two
 * half-turn arcs instead of vanishing.
 */
function donutSlice(start: number, end: number, radius: number, innerRadius: number): string {
  const sweep = end - start;
  if (sweep >= Math.PI * 2 - 1e-6) {
    const half = start + Math.PI;
    return [donutSlice(start, half, radius, innerRadius), donutSlice(half, end, radius, innerRadius)].join(" ");
  }
  const largeArc = sweep > Math.PI ? 1 : 0;
  const outerStart = polar(start, radius);
  const outerEnd = polar(end, radius);
  const innerEnd = polar(end, innerRadius);
  const innerStart = polar(start, innerRadius);
  return [
    `M ${innerStart.x} ${innerStart.y}`,
    `L ${outerStart.x} ${outerStart.y}`,
    `A ${radius} ${radius} 0 ${largeArc} 1 ${outerEnd.x} ${outerEnd.y}`,
    `L ${innerEnd.x} ${innerEnd.y}`,
    `A ${innerRadius} ${innerRadius} 0 ${largeArc} 0 ${innerStart.x} ${innerStart.y}`,
    "Z",
  ].join(" ");
}

function polar(angle: number, radius: number): { x: number; y: number } {
  return { x: Math.cos(angle) * radius, y: Math.sin(angle) * radius };
}
