import { cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";

import { LatticeLoader } from "./LatticeLoader";

afterEach(cleanup);

const delayAt = (root: HTMLElement, cell: number) =>
  (root.children[cell] as HTMLElement).style.getPropertyValue("--lattice-delay");

describe("LatticeLoader", () => {
  it("is a decorative grid whose light orbits the edge cells of a 3x3", () => {
    const { container } = render(<LatticeLoader grid={3} pattern="orbit" />);
    const root = container.firstElementChild as HTMLElement;

    expect(root).toHaveAttribute("aria-hidden", "true");
    expect(root.children).toHaveLength(9);
    expect(Array.from(root.children).filter((cell) => cell.hasAttribute("data-lit"))).toHaveLength(8);
    // The centre cell never lights.
    expect(root.children[4]).not.toHaveAttribute("data-lit");
    // Clockwise from the top-left: along the top, down the right, back along
    // the bottom, up the left. Each cell starts one step after the last.
    expect([0, 1, 2, 5, 8, 7, 6, 3].map((cell) => delayAt(root, cell))).toEqual([
      "0",
      "1",
      "2",
      "3",
      "4",
      "5",
      "6",
      "7",
    ]);
  });

  it("sweeps a right-pointing arrow across a 4x4: middle rows lead, outer rows trail", () => {
    const { container } = render(<LatticeLoader grid={4} pattern="arrow" />);
    const root = container.firstElementChild as HTMLElement;

    expect(root.children).toHaveLength(16);
    expect(root.style.getPropertyValue("--lattice-n")).toBe("4");
    const delay = (row: number, column: number) => Number(delayAt(root, row * 4 + column));
    for (let column = 0; column < 4; column += 1) {
      // Same column, middle rows light before the outer rows: that is the head
      // of the arrow.
      expect(delay(1, column)).toBeLessThan(delay(0, column));
      expect(delay(2, column)).toBe(delay(1, column));
      expect(delay(3, column)).toBe(delay(0, column));
    }
    // And it moves to the right.
    expect(delay(1, 1)).toBeGreaterThan(delay(1, 0));
    expect(delay(1, 3)).toBeGreaterThan(delay(1, 2));
  });

  it("takes its size and speed from props", () => {
    const { container } = render(<LatticeLoader cellSize={6} gap={0.5} step={140} shape="square" />);
    const root = container.firstElementChild as HTMLElement;

    expect(root.style.getPropertyValue("--lattice-cell")).toBe("6px");
    expect(root.style.getPropertyValue("--lattice-gap")).toBe("0.5px");
    expect(root.style.getPropertyValue("--lattice-step")).toBe("140ms");
    expect(root).toHaveAttribute("data-shape", "square");
  });
});
