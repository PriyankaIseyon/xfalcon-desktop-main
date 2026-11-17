import { describe, expect, it } from "vitest";
import { MAX_TREE_NAME_CHARS, cappedName, fileTypeLabel } from "./fileTypeLabel";

describe("fileTypeLabel", () => {
  it.each([
    ["data/regions.csv", "CSV", "cyan"],
    ["reports/brief.docx", "DOCX", "blue"],
    ["reports/brief.DOC", "DOC", "blue"],
    ["decks/IPL-Deep-Dive.pptx", "PPTX", "orange"],
    ["decks/old.ppt", "PPT", "orange"],
    ["books/model.xlsx", "XLSX", "green"],
    ["data/regions.TSV", "TSV", "cyan"],
    ["reports/Q3.html", "HTML", "red"],
    ["site/index.htm", "HTML", "red"],
    ["README.markdown", "MD", "violet"],
    ["data/events.ndjson", "JSONL", "amber"],
    ["queries/pipeline.sql", "SQL", "green"],
    ["notes.txt", "TXT", "muted"],
    ["warehouse/orders.parquet", "PARQ", "cyan"],
  ])("tags %s as %s in %s", (path, text, tone) => {
    expect(fileTypeLabel(path)).toEqual({ text, tone });
  });

  it("gives the common data and Office types different colours", () => {
    const tones = ["a.csv", "a.docx", "a.pptx", "a.xlsx", "a.pdf", "a.md", "a.py"].map(
      (path) => fileTypeLabel(path)?.tone,
    );
    expect(new Set(tones).size).toBe(tones.length);
  });

  it.each(["LICENSE", "Dockerfile", ".gitignore", "trailing.", "archive.backup"])(
    "keeps the plain icon for %s",
    (path) => {
      expect(fileTypeLabel(path)).toBeNull();
    },
  );
});

describe("cappedName", () => {
  it("leaves a name that fits alone", () => {
    expect(cappedName("regions.csv")).toBe("regions.csv");
  });

  it("cuts a long name in the middle and keeps its extension", () => {
    const name = "Global Geography Trivia Dashboard for the Quarterly Review.html";
    const capped = cappedName(name);
    expect(Array.from(capped)).toHaveLength(MAX_TREE_NAME_CHARS);
    expect(capped).toContain("…");
    expect(capped.endsWith("Review.html")).toBe(true);
    expect(name.startsWith(capped.slice(0, capped.indexOf("…")))).toBe(true);
  });
});
