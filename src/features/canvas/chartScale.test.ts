import { describe, expect, it } from "vitest";
import { niceScale, resolveTimeAxis } from "./chartScale";

const DAY_MS = 86_400_000;

/** A seeded linear congruential generator, so the "random" pairs are the same on every run. */
function seededRandom(seed: number): () => number {
  let state = seed;
  return () => {
    state = (state * 1664525 + 1013904223) % 4294967296;
    return state / 4294967296;
  };
}

/** Epoch day for a calendar date, computed in UTC. */
function day(year: number, month: number, date: number): number {
  return Date.UTC(year, month - 1, date) / DAY_MS;
}

describe("niceScale", () => {
  it("puts ticks on round numbers for an uneven domain", () => {
    const scale = niceScale(0, 13800);
    expect(scale.step).toBe(2000);
    expect(scale.ticks).toEqual([0, 2000, 4000, 6000, 8000, 10000, 12000, 14000]);
    expect(scale.min).toBe(0);
    expect(scale.max).toBe(14000);
  });

  it("pads the top to a whole step", () => {
    const scale = niceScale(0, 55000);
    expect(scale.step).toBe(10000);
    expect(scale.max).toBe(60000);
    expect(scale.ticks).toEqual([0, 10000, 20000, 30000, 40000, 50000, 60000]);
  });

  it("includes zero as a tick when the domain crosses it", () => {
    const scale = niceScale(-300, 1200);
    expect(scale.ticks).toEqual([-500, 0, 500, 1000, 1500]);
    expect(scale.min).toBe(-500);
    expect(scale.max).toBe(1500);
    expect(scale.step).toBe(500);
  });

  it("produces 0.2 steps for the unit interval", () => {
    const scale = niceScale(0, 1);
    expect(scale.step).toBe(0.2);
    expect(scale.ticks).toEqual([0, 0.2, 0.4, 0.6, 0.8, 1]);
  });

  it("widens a zero-width domain to [0, 1]", () => {
    const scale = niceScale(0, 0);
    expect(scale.min).toBe(0);
    expect(scale.max).toBe(1);
    expect(scale.ticks).toEqual([0, 0.2, 0.4, 0.6, 0.8, 1]);
  });

  it("widens a positive single value down to zero", () => {
    expect(niceScale(5, 5)).toEqual({ min: 0, max: 5, step: 1, ticks: [0, 1, 2, 3, 4, 5] });
  });

  it("widens a negative single value up to zero", () => {
    expect(niceScale(-5, -5)).toEqual({ min: -5, max: 0, step: 1, ticks: [-5, -4, -3, -2, -1, 0] });
  });

  it("swaps reversed bounds", () => {
    expect(niceScale(13800, 0)).toEqual(niceScale(0, 13800));
  });

  it("handles a span entirely below zero", () => {
    expect(niceScale(-7, -2)).toEqual({ min: -7, max: -2, step: 1, ticks: [-7, -6, -5, -4, -3, -2] });
  });

  it("keeps tiny ranges free of float drift", () => {
    const scale = niceScale(0.001, 0.0034);
    expect(scale.ticks).not.toContain(0.30000000000000004);
    for (const tick of scale.ticks) {
      expect(tick).toBe(Number(tick.toPrecision(12)));
    }
    expect(scale.min).toBeLessThanOrEqual(0.001);
    expect(scale.max).toBeGreaterThanOrEqual(0.0034);
  });

  it("does not drift when stepping in tenths", () => {
    const scale = niceScale(0, 0.3);
    expect(scale.step).toBe(0.05);
    expect(scale.ticks).toEqual([0, 0.05, 0.1, 0.15, 0.2, 0.25, 0.3]);
  });

  it("handles a domain of a trillion", () => {
    expect(niceScale(0, 1e12)).toEqual({
      min: 0,
      max: 1e12,
      step: 2e11,
      ticks: [0, 2e11, 4e11, 6e11, 8e11, 1e12],
    });
  });

  it("handles a domain spanning zero at a trillion", () => {
    expect(niceScale(-1e12, 1e12).ticks).toEqual([-1e12, -5e11, 0, 5e11, 1e12]);
  });

  it("rejects non-finite bounds", () => {
    expect(() => niceScale(Number.NaN, 1)).toThrow(RangeError);
    expect(() => niceScale(0, Number.POSITIVE_INFINITY)).toThrow(RangeError);
    expect(() => niceScale(Number.NEGATIVE_INFINITY, 0)).toThrow(RangeError);
  });

  it("rejects a span too wide to represent", () => {
    expect(() => niceScale(-Number.MAX_VALUE, Number.MAX_VALUE)).toThrow(RangeError);
  });

  it("keeps every guarantee across 300 seeded random ranges", () => {
    const next = seededRandom(20240607);
    for (let i = 0; i < 300; i++) {
      const magnitude = 10 ** Math.floor(next() * 14 - 7);
      const a = (next() * 2 - 1) * magnitude;
      // One pair in five is degenerate, so the single-value widening runs too.
      const b = next() < 0.2 ? a : a + next() * magnitude * 3;
      const scale = niceScale(a, b);
      const lowest = Math.min(a, b);
      const highest = Math.max(a, b);

      expect(scale.min).toBeLessThanOrEqual(lowest);
      expect(scale.max).toBeGreaterThanOrEqual(highest);
      expect(scale.ticks.length).toBeGreaterThanOrEqual(2);
      expect(scale.ticks.length).toBeLessThanOrEqual(11);
      expect(scale.ticks[0]).toBe(scale.min);
      expect(scale.ticks[scale.ticks.length - 1]).toBe(scale.max);

      for (let t = 1; t < scale.ticks.length; t++) {
        expect(Math.abs(scale.ticks[t] - scale.ticks[t - 1] - scale.step)).toBeLessThan(scale.step * 1e-9);
      }

      const exponent = Math.floor(Math.log10(scale.step));
      const mantissa = scale.step / 10 ** exponent;
      expect([1, 2, 5].some((allowed) => Math.abs(mantissa - allowed) < 1e-9)).toBe(true);
    }
  });
});

describe("resolveTimeAxis: value parsing", () => {
  it("reads monthly labels as consecutive month positions", () => {
    const labels = Array.from({ length: 12 }, (_, i) => `2024-${String(i + 1).padStart(2, "0")}`);
    const axis = resolveTimeAxis(labels, "en-US");
    expect(axis?.granularity).toBe("month");
    expect(axis?.positions).toEqual([
      24288, 24289, 24290, 24291, 24292, 24293, 24294, 24295, 24296, 24297, 24298, 24299,
    ]);
    expect(axis?.min).toBe(24288);
    expect(axis?.max).toBe(24299);
  });

  it("aligns monthly ticks to January-based steps", () => {
    const labels = Array.from({ length: 12 }, (_, i) => `2024-${String(i + 1).padStart(2, "0")}`);
    const axis = resolveTimeAxis(labels, "en-US");
    // Step 1 gives 12 ticks, over the limit of 7. Step 2 gives six, aligned to even month indices.
    expect(axis?.ticks).toEqual([
      { position: 24288, label: "Jan 2024" },
      { position: 24290, label: "Mar 2024" },
      { position: 24292, label: "May 2024" },
      { position: 24294, label: "Jul 2024" },
      { position: 24296, label: "Sep 2024" },
      { position: 24298, label: "Nov 2024" },
    ]);
  });

  it("reads uneven day spacing as true time", () => {
    const axis = resolveTimeAxis(["2024-01-01", "2024-01-02", "2024-01-30"], "en-US");
    expect(axis?.granularity).toBe("day");
    const [first, second, third] = axis?.positions ?? [];
    expect(second - first).toBe(1);
    expect(third - second).toBe(28);
  });

  it("aligns weekly day ticks to Mondays", () => {
    // 2024-01-01 was a Monday, so every seventh day from it is a Monday.
    const axis = resolveTimeAxis(["2024-01-01", "2024-01-02", "2024-01-30"], "en-US");
    expect(axis?.ticks.map((tick) => tick.label)).toEqual(["Jan 1", "Jan 8", "Jan 15", "Jan 22", "Jan 29"]);
    expect(axis?.ticks.map((tick) => tick.position)).toEqual([
      day(2024, 1, 1),
      day(2024, 1, 8),
      day(2024, 1, 15),
      day(2024, 1, 22),
      day(2024, 1, 29),
    ]);
  });

  it("accepts a leap day", () => {
    const axis = resolveTimeAxis(["2024-02-28", "2024-02-29", "2024-03-01"], "en-US");
    expect(axis?.granularity).toBe("day");
    expect(axis?.ticks.map((tick) => tick.label)).toEqual(["Feb 28", "Feb 29", "Mar 1"]);
  });

  it("rejects 2023-02-29 because 2023 is not a leap year", () => {
    expect(resolveTimeAxis(["2023-02-28", "2023-02-29"], "en-US")).toBeNull();
  });

  it("rejects a day that does not exist in its month", () => {
    expect(resolveTimeAxis(["2023-02-30", "2023-03-01"], "en-US")).toBeNull();
  });

  it("reads quarter labels in every accepted shape", () => {
    const axis = resolveTimeAxis(["2024-Q1", "2024 Q2", "2024Q3", "Q4-2024"], "en-US");
    expect(axis?.granularity).toBe("quarter");
    expect(axis?.positions).toEqual([24288, 24291, 24294, 24297]);
  });

  it("labels quarters with Q and year", () => {
    const axis = resolveTimeAxis(["Q1 2024", "Q2 2024", "Q3 2024"], "en-US");
    expect(axis?.granularity).toBe("quarter");
    expect(axis?.positions).toEqual([24288, 24291, 24294]);
    expect(axis?.ticks).toEqual([
      { position: 24288, label: "Q1 2024" },
      { position: 24291, label: "Q2 2024" },
      { position: 24294, label: "Q3 2024" },
    ]);
  });

  it("reads month names and abbreviations, with a dot after an abbreviation", () => {
    const axis = resolveTimeAxis(["Sept. 2024", "Oct 2024"], "en-US");
    expect(axis?.granularity).toBe("month");
    expect(axis?.positions).toEqual([24296, 24297]);
    expect(axis?.ticks.map((tick) => tick.label)).toEqual(["Sep 2024", "Oct 2024"]);
  });

  it("rejects a dot after a full month name", () => {
    expect(resolveTimeAxis(["January. 2024", "February 2024"], "en-US")).toBeNull();
  });

  it("reads day names in each accepted shape", () => {
    const axis = resolveTimeAxis(["Jan 5, 2024", "5 Jan 2024", "Jan 7 2024"], "en-US");
    expect(axis?.granularity).toBe("day");
    expect(axis?.positions).toEqual([day(2024, 1, 5), day(2024, 1, 5), day(2024, 1, 7)]);
  });

  it("reads Mon YYYY and YYYY-Mon", () => {
    const axis = resolveTimeAxis(["Jan 2024", "2024-Feb", "2024 Mar"], "en-US");
    expect(axis?.granularity).toBe("month");
    expect(axis?.positions).toEqual([24288, 24289, 24290]);
    expect(axis?.ticks.map((tick) => tick.label)).toEqual(["Jan 2024", "Feb 2024", "Mar 2024"]);
  });

  it("reads a slash-separated month and day", () => {
    expect(resolveTimeAxis(["2024/1", "2024/02"], "en-US")?.positions).toEqual([24288, 24289]);
    expect(resolveTimeAxis(["2024/01/05", "2024/01/06"], "en-US")?.granularity).toBe("day");
  });

  it("reads a year label only inside 1900 to 2100", () => {
    const axis = resolveTimeAxis(["2020", "2021", "2022"], "en-US");
    expect(axis?.granularity).toBe("year");
    expect(axis?.positions).toEqual([2020, 2021, 2022]);
    expect(axis?.ticks.map((tick) => tick.label)).toEqual(["2020", "2021", "2022"]);
  });

  it("thins a wide year range to round years", () => {
    const axis = resolveTimeAxis(["1950", "2050"], "en-US");
    // Steps of 1 and 10 give 101 and 11 ticks, over the limit. A step of 20 gives five.
    expect(axis?.ticks.map((tick) => tick.label)).toEqual(["1960", "1980", "2000", "2020", "2040"]);
  });

  it("does not read a four-digit count as a year", () => {
    expect(resolveTimeAxis(["5000", "5001"], "en-US")).toBeNull();
    expect(resolveTimeAxis(["2024", "5000"], "en-US")).toBeNull();
  });

  it("rejects ambiguous numeric dates", () => {
    expect(resolveTimeAxis(["01/02/2024", "02/03/2024"], "en-US")).toBeNull();
  });

  it("rejects bare month names, weekday names and plain integers", () => {
    expect(resolveTimeAxis(["Jan", "Feb"], "en-US")).toBeNull();
    expect(resolveTimeAxis(["Monday", "Tuesday"], "en-US")).toBeNull();
    expect(resolveTimeAxis(["12", "13"], "en-US")).toBeNull();
  });

  it("rejects an impossible month", () => {
    expect(resolveTimeAxis(["2024-13", "2024-01"], "en-US")).toBeNull();
  });

  it("rejects a mix of granularities", () => {
    expect(resolveTimeAxis(["2024-01", "2024-01-05"], "en-US")).toBeNull();
  });

  it("rejects the whole axis when one label does not parse", () => {
    expect(resolveTimeAxis(["2024-01-01", "hello"], "en-US")).toBeNull();
  });

  it("returns null for fewer than two labels", () => {
    expect(resolveTimeAxis([], "en-US")).toBeNull();
    expect(resolveTimeAxis(["2024-01"], "en-US")).toBeNull();
  });

  it("returns null when every label names the same position", () => {
    expect(resolveTimeAxis(["2024-01", "2024-01"], "en-US")).toBeNull();
    expect(resolveTimeAxis(["2024-01-05", "2024-01-05 "], "en-US")).toBeNull();
  });

  it("orders unsorted input and keeps input order for ties", () => {
    const axis = resolveTimeAxis(["2024-02", "2024-01", "2024-02"], "en-US");
    expect(axis?.positions).toEqual([24289, 24288, 24289]);
    expect(axis?.order).toEqual([1, 0, 2]);
    expect(axis?.min).toBe(24288);
    expect(axis?.max).toBe(24289);
  });

  it("orders unsorted input by position", () => {
    const axis = resolveTimeAxis(["2024-03", "2024-01", "2024-02"], "en-US");
    expect(axis?.order).toEqual([1, 2, 0]);
  });

  it("trims each label before parsing", () => {
    const axis = resolveTimeAxis(["  2024-01 ", "\t2024-02"], "en-US");
    expect(axis?.positions).toEqual([24288, 24289]);
  });
});

describe("resolveTimeAxis: ticks", () => {
  it("honours maxTicks", () => {
    const labels = Array.from({ length: 12 }, (_, i) => `2024-${String(i + 1).padStart(2, "0")}`);
    // Step 3 gives four ticks, over three. Step 6 gives two.
    const axis = resolveTimeAxis(labels, "en-US", 3);
    expect(axis?.ticks.map((tick) => tick.label)).toEqual(["Jan 2024", "Jul 2024"]);
  });

  it("falls back to the two endpoints when no rung fits", () => {
    const labels = Array.from({ length: 12 }, (_, i) => `2024-${String(i + 1).padStart(2, "0")}`);
    const axis = resolveTimeAxis(labels, "en-US", 1);
    expect(axis?.ticks).toEqual([
      { position: 24288, label: "Jan 2024" },
      { position: 24299, label: "Dec 2024" },
    ]);
  });

  it("labels a day axis that crosses a year boundary with the year", () => {
    const axis = resolveTimeAxis(["2023-12-30", "2024-01-02"], "en-US");
    expect(axis?.ticks.map((tick) => tick.label)).toEqual([
      "Dec 30, 2023",
      "Dec 31, 2023",
      "Jan 1, 2024",
      "Jan 2, 2024",
    ]);
  });

  it("uses hours and minutes within one day", () => {
    const axis = resolveTimeAxis(["2024-03-10T12:00", "2024-03-10T13:30"], "en-US");
    expect(axis?.granularity).toBe("time");
    expect(axis?.ticks.map((tick) => tick.label)).toEqual([
      "12:00",
      "12:15",
      "12:30",
      "12:45",
      "13:00",
      "13:15",
      "13:30",
    ]);
  });

  it("prefixes the date when the time axis covers more than a day", () => {
    const axis = resolveTimeAxis(["2024-03-10T12:00", "2024-03-13T12:00"], "en-US");
    expect(axis?.ticks.map((tick) => tick.label)).toEqual([
      "Mar 10 12:00",
      "Mar 11 00:00",
      "Mar 11 12:00",
      "Mar 12 00:00",
      "Mar 12 12:00",
      "Mar 13 00:00",
      "Mar 13 12:00",
    ]);
  });

  it("falls through to whole-day ticks in milliseconds past a day", () => {
    const axis = resolveTimeAxis(["2024-03-01T10:00", "2024-03-21T10:00"], "en-US");
    expect(axis?.granularity).toBe("time");
    expect(axis?.ticks).toEqual([
      { position: day(2024, 3, 4) * DAY_MS, label: "Mar 4" },
      { position: day(2024, 3, 11) * DAY_MS, label: "Mar 11" },
      { position: day(2024, 3, 18) * DAY_MS, label: "Mar 18" },
    ]);
  });

  it("falls back to the two endpoints for a sub-minute span", () => {
    const axis = resolveTimeAxis(["2024-03-10T10:00:10", "2024-03-10T10:00:40"], "en-US");
    expect(axis?.ticks.map((tick) => tick.position)).toEqual([
      Date.UTC(2024, 2, 10, 10, 0, 10),
      Date.UTC(2024, 2, 10, 10, 0, 40),
    ]);
  });
});

describe("resolveTimeAxis: time positions", () => {
  it("applies a +05:30 offset to the instant", () => {
    const axis = resolveTimeAxis(["2024-03-10T12:00:00+05:30", "2024-03-10T06:31:00Z"], "en-US");
    expect(axis?.granularity).toBe("time");
    expect(axis?.positions[0]).toBe(Date.UTC(2024, 2, 10, 6, 30));
    expect(axis?.positions[1]).toBe(Date.UTC(2024, 2, 10, 6, 31));
  });

  it("reads the compact +HHMM offset and the -HH:MM offset", () => {
    const axis = resolveTimeAxis(["2024-03-10T11:30+0530", "2024-03-10T01:00:00.5-05:00"], "en-US");
    expect(axis?.positions[0]).toBe(Date.UTC(2024, 2, 10, 6, 0));
    expect(axis?.positions[1]).toBe(Date.UTC(2024, 2, 10, 6, 0, 0, 500));
  });

  it("returns null when two offsets name the same instant", () => {
    expect(resolveTimeAxis(["2024-03-10T11:30+0530", "2024-03-10T01:00:00-05:00"], "en-US")).toBeNull();
  });

  it("treats a label with no offset as UTC", () => {
    const axis = resolveTimeAxis(["2024-03-10 12:00", "2024-03-10T12:00:30.5Z"], "en-US");
    expect(axis?.positions[0]).toBe(Date.UTC(2024, 2, 10, 12, 0));
    expect(axis?.positions[1]).toBe(Date.UTC(2024, 2, 10, 12, 0, 30, 500));
  });

  it("rejects an offset with an out-of-range hour", () => {
    expect(resolveTimeAxis(["2024-03-10T12:00+25:00", "2024-03-10T13:00"], "en-US")).toBeNull();
  });

  it("rejects an out-of-range clock", () => {
    expect(resolveTimeAxis(["2024-03-10T24:00", "2024-03-10T13:00"], "en-US")).toBeNull();
    expect(resolveTimeAxis(["2024-03-10T12:60", "2024-03-10T13:00"], "en-US")).toBeNull();
  });
});
