import { describe, it, expect } from "vitest";
import { cn } from "./cn";

describe("cn", () => {
  it("joins truthy classes and drops falsy ones", () => {
    expect(cn("a", false, "c")).toBe("a c");
  });
  it("later tailwind utilities win conflicts (merge)", () => {
    expect(cn("px-2", "px-4")).toBe("px-4");
  });
});
