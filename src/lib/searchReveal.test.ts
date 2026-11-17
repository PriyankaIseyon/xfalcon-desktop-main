import { describe, expect, it } from "vitest";
import { splitHighlighted } from "./searchReveal";

describe("splitHighlighted", () => {
  it("marks the query inside the original spelling", () => {
    expect(splitHighlighted("Café revenue", "cafe")).toEqual([
      { value: "Café", hit: true },
      { value: " revenue", hit: false },
    ]);
  });

  it("marks every token", () => {
    const parts = splitHighlighted("Diagnostics and troubleshooting", "diag trouble");
    expect(parts.filter((part) => part.hit).map((part) => part.value)).toEqual(["Diag", "trouble"]);
  });
});
