import { describe, expect, it } from "vitest";
import { displayPath } from "./displayPath";

describe("displayPath", () => {
  it("drops the Windows extended-length prefix a canonical path carries", () => {
    expect(displayPath("\\\\?\\D:\\work\\xfalcon\\test project 2")).toBe("D:\\work\\xfalcon\\test project 2");
  });

  it("restores a UNC path to the form people recognise", () => {
    expect(displayPath("\\\\?\\UNC\\server\\share\\reports")).toBe("\\\\server\\share\\reports");
  });

  it("leaves a path that never had the prefix alone", () => {
    expect(displayPath("D:\\work\\xfalcon")).toBe("D:\\work\\xfalcon");
    expect(displayPath("/home/jeevan/work")).toBe("/home/jeevan/work");
    // A genuine UNC path is not an extended-length one and must survive intact.
    expect(displayPath("\\\\server\\share")).toBe("\\\\server\\share");
  });

  it("is display-only, so the value it is given is never what gets used for I/O", () => {
    // Guards the reason this is a formatter and not a normaliser: the prefix is
    // what lifts MAX_PATH, so the original must remain usable afterwards.
    const canonical = "\\\\?\\D:\\deep";
    displayPath(canonical);
    expect(canonical).toBe("\\\\?\\D:\\deep");
  });
});
