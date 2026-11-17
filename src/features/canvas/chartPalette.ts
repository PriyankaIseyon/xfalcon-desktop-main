import { useSyncExternalStore } from "react";

/** Colours a chart draws with, already resolved to literal values. */
export interface ChartPalette {
  /** One colour per series, in order. Reused cyclically past the end. */
  readonly series: readonly string[];
  /** Axis labels and the zero line. */
  readonly axis: string;
  /** Grid rules behind the plot. */
  readonly grid: string;
  /** What a point ring and a pie separator are cut out of. */
  readonly surface: string;
}

const SERIES_COUNT = 7;

/**
 * The light palette, in literal form.
 *
 * Only reached where there is no stylesheet to read -- a unit test's jsdom,
 * chiefly. In the built app `scripts/check-style-contract.mjs` fails the build
 * if any `--xf-chart-*` token is missing, so a chart cannot silently fall back
 * to the wrong theme.
 */
const FALLBACK: ChartPalette = {
  series: ["#1596b0", "#7851a9", "#c77700", "#2a78d6", "#d55181", "#008300", "#606a79"],
  axis: "#606a79",
  grid: "#dce5ef",
  surface: "#ffffff",
};

let cached: ChartPalette | null = null;
const listeners = new Set<() => void>();
let stopWatching: (() => void) | null = null;

function token(styles: CSSStyleDeclaration, name: string, fallback: string): string {
  const value = styles.getPropertyValue(name).trim();
  return value === "" ? fallback : value;
}

/**
 * Reads the tokens as the document currently computes them.
 *
 * The values come back resolved, so `--xf-chart-axis: var(--xf-ink-3)` arrives
 * as a colour rather than as a reference. That is the whole point: the chart is
 * serialized to a standalone `.svg`, and a file that carried `var(--xf-ink-3)`
 * would render black in a viewer that has none of this app's stylesheets.
 */
function resolve(): ChartPalette {
  if (typeof document === "undefined" || typeof getComputedStyle !== "function") return FALLBACK;
  const styles = getComputedStyle(document.documentElement);
  return {
    series: Array.from({ length: SERIES_COUNT }, (_, index) =>
      token(styles, `--xf-chart-${index + 1}`, FALLBACK.series[index]!),
    ),
    axis: token(styles, "--xf-chart-axis", FALLBACK.axis),
    grid: token(styles, "--xf-chart-grid", FALLBACK.grid),
    surface: token(styles, "--xf-chart-surface", FALLBACK.surface),
  };
}

export function getChartPalette(): ChartPalette {
  if (cached === null) cached = resolve();
  return cached;
}

function invalidate(): void {
  cached = null;
  for (const listener of listeners) listener();
}

/**
 * Everything that can change what those tokens compute to.
 *
 * `data-theme` covers the in-app switch, whichever surface throws it. The media
 * queries cover the choices the app does not own: the reader turns contrast up,
 * or the operating system enters forced colours, and the palette has to follow
 * without a re-mount.
 */
const WATCHED_QUERIES = ["(prefers-color-scheme: dark)", "(prefers-contrast: more)", "(forced-colors: active)"];

function startWatching(): () => void {
  const observer = new MutationObserver(invalidate);
  observer.observe(document.documentElement, { attributeFilter: ["data-theme"] });

  const queries =
    typeof window.matchMedia === "function" ? WATCHED_QUERIES.map((query) => window.matchMedia(query)) : [];
  for (const query of queries) query.addEventListener("change", invalidate);

  return () => {
    observer.disconnect();
    for (const query of queries) query.removeEventListener("change", invalidate);
  };
}

export function subscribeChartPalette(listener: () => void): () => void {
  listeners.add(listener);
  if (stopWatching === null && typeof window !== "undefined") stopWatching = startWatching();
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && stopWatching !== null) {
      stopWatching();
      stopWatching = null;
    }
  };
}

export function useChartPalette(): ChartPalette {
  return useSyncExternalStore(subscribeChartPalette, getChartPalette, () => FALLBACK);
}

/** Drops the resolved values, so the next read sees the current stylesheet. */
export function resetChartPaletteForTests(): void {
  cached = null;
}
