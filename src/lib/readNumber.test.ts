import { describe, expect, it } from "vitest";
import { readNumber, resolveNumberFormats, type NumberFormat } from "./readNumber";

/** One column holding exactly these cells, read the way the grid and chart do. */
function readColumn(cells: string[]): (number | null)[] | null {
  const [format] = resolveNumberFormats([cells]);
  if (format === null || format === undefined) return null;
  return cells.map((cell) => readNumber(cell, format));
}

describe("a cell is read by the arrangement of its own separators", () => {
  // Cells the separators settle on their own: no column context is needed.
  const settled: [string, number][] = [
    ["1234", 1234],
    ["0", 0],
    ["007", 7],
    ["1.5", 1.5],
    ["1,5", 1.5],
    ["1.50", 1.5],
    ["12,5", 12.5],
    ["1234,5678", 1234.5678],
    ["1,234.56", 1234.56],
    ["1.234,56", 1234.56],
    ["1,234,567", 1234567],
    ["1.234.567", 1234567],
    ["1,234,567.89", 1234567.89],
    ["1.234.567,89", 1234567.89],
    ["12,34,567", 1234567],
    ["12,34,567.5", 1234567.5],
    ["1 234", 1234],
    ["1 234,5", 1234.5],
    ["1 234.5", 1234.5],
    ["1 234,5", 1234.5],
    ["1 234,56", 1234.56],
    ["1'234'567.5", 1234567.5],
    ["1’234", 1234],
    // A head that cannot be a group is a decimal even with a three-digit tail.
    ["0,500", 0.5],
    ["0.123", 0.123],
    ["1234,567", 1234.567],
    ["1234.567", 1234.567],
  ];

  it.each(settled)("reads %s as %d", (cell, expected) => {
    expect(readColumn([cell])).toEqual([expected]);
  });

  it("reads the same cell as the same number whatever the machine locale is", () => {
    // Reading takes no locale input at all; this pins that the answer for the
    // two famously confusable spellings is fixed by the text.
    expect(readColumn(["1.234,56"])).toEqual([1234.56]);
    expect(readColumn(["1,234.56"])).toEqual([1234.56]);
  });
});

describe("sign, currency and percent are read, never dropped", () => {
  const cases: [string, number][] = [
    ["-5", -5],
    ["+5", 5],
    ["−5", -5],
    ["$5", 5],
    ["-$5", -5],
    ["+$5", 5],
    ["$-5", -5],
    ["-$1,200", -1200],
    ["-€3.5", -3.5],
    ["€ 5,00", 5],
    ["5,00 €", 5],
    ["5,00 €", 5],
    ["1.234,56 €", 1234.56],
    ["-1.234,56 €", -1234.56],
    ["£1,000,000", 1000000],
    ["₹1,23,456", 123456],
    ["12%", 12],
    ["-5%", -5],
    ["5 %", 5],
    ["-$1,200.50", -1200.5],
    ["  7  ", 7],
  ];

  it.each(cases)("reads %s as %d", (cell, expected) => {
    expect(readColumn([cell])).toEqual([expected]);
  });

  it("does not divide a percent by a hundred: the number shown is the number written", () => {
    expect(readColumn(["12%"])).toEqual([12]);
  });

  it("never reads a signed zero", () => {
    const [value] = readColumn(["-0"]) as number[];
    expect(Object.is(value, 0)).toBe(true);
  });
});

describe("a cell that is not a number is refused, and refuses its column", () => {
  const refused = [
    "",
    "-",
    "+",
    "North",
    "2026-03",
    "2026-03-04",
    "10:30",
    "1.2.3",
    "1,2,3",
    "1,,2",
    "1, 234",
    "v2",
    "0x10",
    "1e3",
    "3.5M",
    "$1.2B",
    "(1,234)",
    "5-",
    "-$-5",
    "$5€",
    "- 5",
    "$$5",
    "1.234,56.7",
    "1.234,56,7",
    "1,234.5,6",
    "1 234 567,8 9",
    "2026 03",
    "012,345",
    "1 234'567",
    "NaN",
    "Infinity",
  ];

  it.each(refused)("%j is not a measure", (cell) => {
    // A blank is ignored rather than refused, so pair it with a real number.
    const column = cell.trim() === "" ? [cell] : ["5", cell];
    const [format] = resolveNumberFormats([column]);
    expect(format).toBeNull();
  });

  it("ignores blank cells in a column of numbers", () => {
    expect(readColumn(["5", "", "  ", "7"])).toEqual([5, null, null, 7]);
  });

  it("does not make a column of only blanks a measure", () => {
    expect(resolveNumberFormats([["", " "]])).toEqual([null]);
  });
});

describe("an ambiguous cell defers to its column", () => {
  it("reads 1,234 as 1234 when the column groups with commas", () => {
    expect(readColumn(["1,234", "9,876.5"])).toEqual([1234, 9876.5]);
  });

  it("reads 1.234 as 1234 when the column groups with dots", () => {
    expect(readColumn(["1.234", "9.876,5"])).toEqual([1234, 9876.5]);
  });

  it("reads 1,234 as 1.234 when the column writes its decimal with commas", () => {
    expect(readColumn(["1,234", "2,5"])).toEqual([1.234, 2.5]);
  });

  it("takes repeated grouping as evidence", () => {
    expect(readColumn(["1,234", "1,234,567"])).toEqual([1234, 1234567]);
    expect(readColumn(["1.234", "1.234.567"])).toEqual([1234, 1234567]);
  });

  it("falls back to comma-groups when every cell is ambiguous and nothing else decides", () => {
    // The commonest table there is: `$1,200 / $3,400 / $12,500`. Refusing it
    // would stop it charting; the producers of these tables write this way.
    expect(readColumn(["1,234", "5,678", "12,500"])).toEqual([1234, 5678, 12500]);
    expect(readColumn(["$1,200", "-$3,400", "+$12,500"])).toEqual([1200, -3400, 12500]);
  });

  it("refuses a dot-ambiguous column when nothing else decides", () => {
    // German thousands or English three-decimal values: a factor of a thousand
    // apart and equally plausible, so it is refused rather than guessed.
    expect(resolveNumberFormats([["4.064", "-810.553", "55.719"]])).toEqual([null]);
    expect(resolveNumberFormats([["1,234", "5.678"]])).toEqual([null]);
  });

  it("reads a dot-ambiguous column once the column or the table decides", () => {
    expect(readColumn(["4.064", "9.876,5"])).toEqual([4064, 9876.5]);
    const formats = resolveNumberFormats([
      ["4.064", "55.719"],
      ["3,5", "10,25"],
    ]);
    expect(formats).toEqual([{ decimal: "," }, { decimal: "," }]);
    expect(readNumber("4.064", formats[0] as NumberFormat)).toBe(4064);
  });

  it("does not refuse a column that has no ambiguous cell, whatever the table says", () => {
    expect(readColumn(["1234", "5678", "12"])).toEqual([1234, 5678, 12]);
  });

  it("lets the rest of the table decide a column with no evidence of its own", () => {
    const formats = resolveNumberFormats([
      ["1,234", "5,678", "12,500"],
      ["1,234.50", "99.25", "3.1"],
    ]);
    expect(formats).toEqual([{ decimal: "." }, { decimal: "." }]);
    expect(readNumber("12,500", formats[0] as NumberFormat)).toBe(12500);
  });

  it("reads a German table the same way when its own column is the ambiguous one", () => {
    const formats = resolveNumberFormats([
      ["1.234", "5.678"],
      ["1.234,50", "99,25"],
    ]);
    expect(formats).toEqual([{ decimal: "," }, { decimal: "," }]);
    expect(readNumber("1.234", formats[0] as NumberFormat)).toBe(1234);
  });

  it("falls back to the default when the other columns disagree about the table", () => {
    const formats = resolveNumberFormats([
      ["1,234", "5,678"],
      ["1,5", "2,25"],
      ["1.5", "2.25"],
    ]);
    // The two columns that disagree leave the table silent, so the first has
    // nothing to go on; each of the other two still reads under its own marks.
    expect(formats).toEqual([{ decimal: "." }, { decimal: "," }, { decimal: "." }]);
  });

  it("uses a column's own evidence over the table's", () => {
    const formats = resolveNumberFormats([
      ["1,234", "2,5"],
      ["1,234.50", "99.25"],
    ]);
    expect(formats).toEqual([{ decimal: "," }, { decimal: "." }]);
  });
});

describe("a column whose own unambiguous cells disagree is not a measure", () => {
  it("refuses 1.234,5 beside 9,876.5", () => {
    expect(resolveNumberFormats([["1.234,5", "9,876.5"]])).toEqual([null]);
  });

  it("refuses 1,5 beside 1.5", () => {
    expect(resolveNumberFormats([["1,5", "1.5"]])).toEqual([null]);
  });
});

describe("excluded columns", () => {
  it("are never measures and give the table no evidence", () => {
    const formats = resolveNumberFormats(
      [
        ["1,5", "2,5"],
        ["1,234", "5,678"],
      ],
      [true, false],
    );
    // The excluded column's decimal commas do not decide the other column.
    expect(formats).toEqual([null, { decimal: "." }]);
  });
});

describe("readNumber on a resolved format", () => {
  it("passes finite numbers through and refuses everything that is not text or a number", () => {
    const format: NumberFormat = { decimal: "." };
    expect(readNumber(42, format)).toBe(42);
    expect(readNumber(Number.NaN, format)).toBeNull();
    expect(readNumber(Number.POSITIVE_INFINITY, format)).toBeNull();
    expect(readNumber(true, format)).toBeNull();
    expect(readNumber(null, format)).toBeNull();
    expect(readNumber(undefined, format)).toBeNull();
  });

  it("returns null for a cell whose own separators contradict the format", () => {
    expect(readNumber("1,5", { decimal: "." })).toBeNull();
    expect(readNumber("1.5", { decimal: "," })).toBeNull();
  });

  it("returns null for a refused cell", () => {
    expect(readNumber("North", { decimal: "." })).toBeNull();
  });

  it("agrees with the classifier: every cell that makes a column numeric reads", () => {
    const cells = ["-$1,200", "+$5", "$ 5", "-5%", "1 234", "1,234,567.89", "12,34,567"];
    const [format] = resolveNumberFormats([cells]);
    expect(format).not.toBeNull();
    for (const cell of cells) {
      expect(readNumber(cell, format as NumberFormat), cell).not.toBeNull();
    }
  });
});
