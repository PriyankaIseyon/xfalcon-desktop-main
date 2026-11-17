import { afterEach, describe, expect, it, vi } from "vitest";
import { createNumberFormatter, formatNumber, formatTick } from "./formatNumber";

describe("format", () => {
  const en = createNumberFormatter("en-US");

  it("uses up to three fraction digits by default, like toLocaleString", () => {
    expect(en.format(1234.5678)).toBe("1,234.568");
  });

  it("never prints negative zero", () => {
    expect(en.format(-0)).toBe("0");
  });

  it("shows a non-finite value as an em dash, never NaN or Infinity", () => {
    expect(en.format(NaN)).toBe("—");
    expect(en.format(Infinity)).toBe("—");
    expect(en.format(-Infinity)).toBe("—");
  });

  it("honours explicit fraction digits", () => {
    expect(en.format(1, { minimumFractionDigits: 1, maximumFractionDigits: 1 })).toBe("1.0");
  });

  it("uses German separators for de-DE", () => {
    expect(createNumberFormatter("de-DE").format(1234.5)).toBe("1.234,5");
  });

  it("matches the platform's fr-FR output, which uses a narrow no-break space", () => {
    const expected = new Intl.NumberFormat("fr-FR").format(1234.5);
    expect(createNumberFormatter("fr-FR").format(1234.5).normalize("NFKC")).toBe(expected.normalize("NFKC"));
  });
});

describe("formatTick", () => {
  const en = createNumberFormatter("en-US");

  it.each([
    [1500, 500, "1.5k"],
    [2000, 500, "2k"],
    [0.25, 0.05, "0.25"],
    [1_200_000, 200_000, "1.2M"],
    [-2500, 500, "-2.5k"],
    [60000, 10000, "60k"],
    [500, 500, "500"],
    [2500, 2500, "2.5k"],
    [0.25, 0.25, "0.25"],
    [3e9, 1e9, "3B"],
    [1.5e12, 5e11, "1.5T"],
    [0, 500, "0"],
    [-0, 500, "0"],
  ])("formats %s at step %s as %s", (value, step, expected) => {
    expect(en.formatTick(value, step)).toBe(expected);
  });

  it("reads a whole axis from 0 to 60000 at step 10000 as clean ticks", () => {
    const ticks = [0, 10_000, 20_000, 30_000, 40_000, 50_000, 60_000].map((value) => en.formatTick(value, 10_000));
    expect(ticks).toEqual(["0", "10k", "20k", "30k", "40k", "50k", "60k"]);
  });

  it("uses German separators for de-DE and keeps the English suffix", () => {
    expect(createNumberFormatter("de-DE").formatTick(1500, 500)).toBe("1,5k");
  });

  it("shows a non-finite value as an em dash", () => {
    expect(en.formatTick(NaN, 500)).toBe("—");
  });
});

describe("host locale and agreement", () => {
  it("default exports follow the host locale", () => {
    expect(formatNumber(1234.5)).toBe((1234.5).toLocaleString());
    expect(formatTick(1500, 500)).toBe(createNumberFormatter().formatTick(1500, 500));
  });

  it("axis ticks and value display agree on separators in de-DE", () => {
    const de = createNumberFormatter("de-DE");
    expect(de.formatTick(1500, 500)).toContain(",");
    expect(de.format(1.5)).toBe("1,5");
  });
});

describe("instance reuse", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("constructs at most one Intl.NumberFormat per digit key across 1000 calls", () => {
    const RealNumberFormat = Intl.NumberFormat;
    const spy = vi.spyOn(Intl, "NumberFormat").mockImplementation(function (locales, options) {
      return new RealNumberFormat(locales, options);
    });
    const formatter = createNumberFormatter("en-US");
    for (let index = 0; index < 1000; index += 1) {
      formatter.format(index, { maximumFractionDigits: 2 });
    }
    expect(spy.mock.calls.length).toBe(1);
  });
});
