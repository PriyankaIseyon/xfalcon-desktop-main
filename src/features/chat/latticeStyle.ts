import type { LatticeLoaderProps } from "../../ui/LatticeLoader";

/** The run block's working indicator: a three-by-three grid with an arrow
 * sweeping to the right. The same in the waiting pill and the working card, so
 * the swap between them changes nothing the reader can see.
 *
 * Compact, and the gap is a WHOLE pixel. At 0.5px the cell pitch is 6.5px, which
 * lands between device pixels even at 100% display scaling, so the browser
 * snapped neighbouring dots to different pixels and the grid looked unevenly
 * spaced. At 1px the pitch is 7px: whole device pixels at 100% and 200%. At a
 * fractional scale (125%, 150%) the pitch is still fractional in device pixels
 * and dots can differ by a pixel; that residual is accepted. */
export const LATTICE: Pick<LatticeLoaderProps, "grid" | "pattern" | "shape" | "cellSize" | "gap" | "step"> = {
  grid: 3,
  pattern: "arrow",
  shape: "round",
  cellSize: 6,
  gap: 1,
  step: 140,
};
