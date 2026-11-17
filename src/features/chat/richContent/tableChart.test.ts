import { describe, expect, it } from "vitest";
import { chartSpecFor, numberFormatsFor } from "./tableChart";
import type { TableData } from "./detect";

function table(headers: string[], rows: string[][]): TableData {
  return { headers, rows };
}

describe("which columns are measures", () => {
  it("reads a signed currency column as numbers, so a loss charts below zero", () => {
    const amounts = table(
      ["label", "amount"],
      [
        ["Refund", "$300"],
        ["Fee", "-$1,200"],
        ["Bonus", "+$5"],
      ],
    );
    expect(numberFormatsFor(amounts)[1]).not.toBeNull();
    expect(chartSpecFor(amounts, numberFormatsFor(amounts))?.data.map((point) => point.y0)).toEqual([300, -1200, 5]);
  });

  it("reads German and English amounts of the same quantities as the same chart data", () => {
    const english = table(
      ["label", "amount"],
      [
        ["A", "1,234.56"],
        ["B", "2,000.00"],
      ],
    );
    const german = table(
      ["label", "amount"],
      [
        ["A", "1.234,56"],
        ["B", "2.000,00"],
      ],
    );
    const englishData = chartSpecFor(english, numberFormatsFor(english))?.data;
    const germanData = chartSpecFor(german, numberFormatsFor(german))?.data;
    expect(englishData?.map((point) => point.y0)).toEqual([1234.56, 2000]);
    expect(germanData).toEqual(englishData);
  });

  it("never treats an identifier column as a measure, even when every cell is numeric", () => {
    const withId = table(
      ["id", "name", "amount"],
      [
        ["1", "Acme", "300"],
        ["2", "Beta", "500"],
      ],
    );
    const formats = numberFormatsFor(withId);
    expect(formats[0]).toBeNull();
    const spec = chartSpecFor(withId, formats);
    expect(spec?.yKeys).toEqual(["y0"]);
    expect(spec?.seriesLabels).toEqual({ y0: "amount" });
  });

  it("draws a blank amount cell as a gap, not as zero or text", () => {
    const sparse = table(
      ["label", "amount"],
      [
        ["A", "300"],
        ["B", ""],
        ["C", "500"],
      ],
    );
    expect(chartSpecFor(sparse, numberFormatsFor(sparse))?.data.map((point) => point.y0)).toEqual([300, null, 500]);
  });

  it("refuses a column whose only numeric-looking values are dot-ambiguous, unless a row fallback is allowed", () => {
    // `4.064` may be a thousands group or a decimal, and nothing else in the table decides.
    const ambiguous = table(
      ["label", "value"],
      [
        ["A", "4.064"],
        ["B", "-810.553"],
      ],
    );
    expect(numberFormatsFor(ambiguous)[1]).toBeNull();
    expect(chartSpecFor(ambiguous, numberFormatsFor(ambiguous))).toBeNull();
  });

  it("keeps a label column that holds a year and month textual", () => {
    const months = table(
      ["month", "amount"],
      [
        ["2026-03", "10"],
        ["2026-04", "20"],
      ],
    );
    const formats = numberFormatsFor(months);
    expect(formats[0]).toBeNull();
    expect(chartSpecFor(months, formats)?.data.map((point) => point.x)).toEqual(["2026-03", "2026-04"]);
  });
});
