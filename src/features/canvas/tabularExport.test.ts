import { describe, expect, it } from "vitest";
import { neutralizeSpreadsheetFormula, toCsv } from "./tabularExport";

describe("CSV quoting", () => {
  it("quotes every field, so a delimiter inside a value cannot end it", () => {
    expect(toCsv(["a", "b"], [["x,y", "z"]])).toBe('"a","b"\r\n"x,y","z"');
  });

  it("doubles an embedded quote", () => {
    expect(toCsv(["a"], [['say "hi"']])).toBe('"a"\r\n"say ""hi"""');
  });

  it("keeps a newline inside a quoted field rather than splitting the row", () => {
    const csv = toCsv(["note"], [["line one\nline two"]]);
    expect(csv).toBe('"note"\r\n"line one\nline two"');
    // Three physical lines, two logical records.
    expect(csv.split("\r\n")).toHaveLength(2);
  });

  it("writes a blank for a missing cell instead of the word null", () => {
    expect(toCsv(["a", "b"], [[null, "z"]])).toBe('"a","b"\r\n"","z"');
  });

  it("emits a header even when there are no rows", () => {
    expect(toCsv(["a", "b"], [])).toBe('"a","b"');
  });

  it("renders numbers and booleans as their text", () => {
    expect(toCsv(["n", "b"], [[42, true]])).toBe('"n","b"\r\n"42","true"');
  });

  it("carries a number exactly as it arrived, with the formula guard on a leading sign", () => {
    // `1.234,56` is not a formula and is written as it is. `-$1,200` leads with
    // `-`, so the guard prefixes it with `'`; the value itself is not changed.
    expect(toCsv(["v"], [["1.234,56"], ["-$1,200"]])).toBe('"v"\r\n"1.234,56"\r\n"\'-$1,200"');
  });
});

describe("spreadsheet formula injection", () => {
  it("neutralizes every character that starts a formula", () => {
    for (const payload of [
      '=HYPERLINK("http://evil.test","click")',
      "+1+1",
      "-2+3",
      "@SUM(A1:A9)",
      "\t=1+1",
      "\r=1+1",
    ]) {
      const safe = neutralizeSpreadsheetFormula(payload);
      expect(safe.startsWith("'")).toBe(true);
      // Nothing is deleted — the value a reader sees is the value that arrived.
      expect(safe.slice(1)).toBe(payload);
    }
  });

  it("leaves ordinary text alone", () => {
    for (const plain of ["North", "2026-03-04", "12", "a=b", "", " =1+1"]) {
      expect(neutralizeSpreadsheetFormula(plain)).toBe(plain);
    }
  });

  it("carries the guard through a whole export, header included", () => {
    const csv = toCsv(["=cmd"], [["=1+1"], ["-5"]]);
    expect(csv).toBe('"\'=cmd"\r\n"\'=1+1"\r\n"\'-5"');
  });
});

// Reading a cell as a number moved to src/lib/readNumber.test.ts, which covers
// every shape these cases did; the sort case is the DataGrid's numeric sort test.
