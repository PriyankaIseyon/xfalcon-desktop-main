import { describe, expect, it } from "vitest";
import { LATTICE } from "./latticeStyle";

describe("the run block's lattice", () => {
  it("keeps a whole-pixel gap, so the dots are spaced evenly at any display scaling", () => {
    // A fractional gap puts the cell pitch between device pixels at 100% and
    // 150% scaling, and the browser snaps neighbouring dots unevenly.
    expect(Number.isInteger(LATTICE.gap)).toBe(true);
    expect(LATTICE.gap).toBeGreaterThanOrEqual(1);
  });

  it("draws every dot at one size", () => {
    expect(Number.isInteger(LATTICE.cellSize)).toBe(true);
  });
});
