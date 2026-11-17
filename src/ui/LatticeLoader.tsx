import type { CSSProperties } from "react";

import { cn } from "../lib/cn";

export type LatticePattern = "orbit" | "arrow";

export type LatticeLoaderProps = {
  className?: string;
  /** Cells per side. */
  grid?: 3 | 4;
  /** How the light moves: round the edge, or a chevron sweeping to the right. */
  pattern?: LatticePattern;
  shape?: "round" | "square";
  /** Side of one cell, in px. */
  cellSize?: number;
  /** Space between cells, in px. */
  gap?: number;
  /** How long the light takes to move from one cell to its neighbour, in ms. */
  step?: number;
};

type Wave = {
  /** When each cell lights, in steps after the loop starts; `null` for a cell
   * that never does. Row by row, left to right. */
  delays: (number | null)[];
  /** How many steps one pass takes, including the rest before the next. */
  loop: number;
};

/** The light travels clockwise round the edge of the grid. The inner cells
 * stay dim. */
function orbit(n: number): Wave {
  const delays: (number | null)[] = Array.from({ length: n * n }, () => null);
  const ring: number[] = [];
  for (let c = 0; c < n; c += 1) ring.push(c);
  for (let r = 1; r < n; r += 1) ring.push(r * n + (n - 1));
  for (let c = n - 2; c >= 0; c -= 1) ring.push((n - 1) * n + c);
  for (let r = n - 2; r >= 1; r -= 1) ring.push(r * n);
  ring.forEach((cell, place) => {
    delays[cell] = place;
  });
  return { delays, loop: ring.length };
}

/** A chevron pointing right sweeps across the grid: the middle rows lead and
 * the outer rows trail, so the front of the wave is the head of an arrow. */
function arrow(n: number): Wave {
  const middle = (n - 1) / 2;
  const delays = Array.from({ length: n * n }, (_, cell) => {
    const row = Math.floor(cell / n);
    const column = cell % n;
    return column + Math.abs(row - middle);
  });
  return { delays, loop: Math.ceil(Math.max(...delays)) + n + 1 };
}

const WAVES: Record<LatticePattern, (n: number) => Wave> = { orbit, arrow };

/**
 * A decorative working indicator: a light that travels across a grid of dots.
 *
 * Drawn in `currentColor`, so the surrounding text colour sets its colour with
 * no prop. The motion is CSS only — each cell runs the same fade, offset by its
 * place in the wave — so a run that streams for minutes costs the page no
 * React renders and no timers. Under `prefers-reduced-motion` the dots hold
 * still, half lit.
 *
 * Always `aria-hidden`: whatever it sits beside already says the app is
 * working.
 */
export function LatticeLoader({
  className,
  grid = 3,
  pattern = "orbit",
  shape = "round",
  cellSize = 5,
  gap = 2,
  step = 90,
}: LatticeLoaderProps) {
  const wave = WAVES[pattern](grid);
  const style = {
    "--lattice-n": grid,
    "--lattice-cell": `${cellSize}px`,
    "--lattice-gap": `${gap}px`,
    "--lattice-step": `${step}ms`,
    "--lattice-loop": wave.loop,
  } as CSSProperties;
  return (
    <span
      aria-hidden
      className={cn("lattice-loader", className)}
      data-pattern={pattern}
      data-shape={shape}
      style={style}
    >
      {wave.delays.map((delay, cell) => (
        <span
          key={cell}
          className="lattice-loader__cell"
          data-lit={delay === null ? undefined : ""}
          style={delay === null ? undefined : ({ "--lattice-delay": delay } as CSSProperties)}
        />
      ))}
    </span>
  );
}
