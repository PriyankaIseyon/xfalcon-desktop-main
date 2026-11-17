import { describe, expect, it } from "vitest";
import { matchesSearch, normalizeForSearch, scoreSearch } from "./searchText";

describe("normalizeForSearch", () => {
  it("folds case and diacritics to one comparable form", () => {
    expect(normalizeForSearch("Café")).toBe("cafe");
    expect(normalizeForSearch("RÉSUMÉ")).toBe("resume");
  });

  it("leaves text that has neither alone", () => {
    expect(normalizeForSearch("quarterly review")).toBe("quarterly review");
  });
});

describe("matchesSearch", () => {
  it("ignores case", () => {
    expect(matchesSearch("Quarterly review", "QUARTERLY")).toBe(true);
  });

  it("ignores diacritics on either side", () => {
    // Titles carry whatever the user typed when they opened the chat; the
    // search that has to find it again is typed in a hurry.
    expect(matchesSearch("Café revenue", "cafe")).toBe(true);
    expect(matchesSearch("Cafe revenue", "café")).toBe(true);
  });

  it("matches anywhere in the text, not only at the start", () => {
    expect(matchesSearch("Q3 margin review", "margin")).toBe(true);
  });

  it("treats an empty or whitespace query as matching everything", () => {
    // A palette that has just opened should show what it can do.
    expect(matchesSearch("anything", "")).toBe(true);
    expect(matchesSearch("anything", "   ")).toBe(true);
  });

  it("reports a genuine miss", () => {
    expect(matchesSearch("Quarterly review", "hiring")).toBe(false);
  });

  it("requires every query token to be present", () => {
    expect(matchesSearch("Q3 margin review", "q3 hiring")).toBe(false);
    expect(matchesSearch("Q3 margin review", "q3 margin")).toBe(true);
  });

  it("accepts a one-letter typo in a longer token", () => {
    expect(matchesSearch("Q3 margin review", "mrgin")).toBe(true);
    expect(scoreSearch("Q3 margin review", "margin")).toBeGreaterThan(scoreSearch("Q3 margin review", "mrgin") ?? 0);
  });
});
