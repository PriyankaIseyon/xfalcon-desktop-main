/**
 * Scales for the hand-drawn SVG chart (spec: dynamic-canvas-artifacts).
 *
 * Pure and dependency-free, and every date is UTC. The same rows must place
 * their ticks identically on every machine, and the local zone is the one input
 * that differs between them.
 */

export interface NiceScale {
  min: number;
  max: number;
  step: number;
  ticks: number[];
}

/**
 * Rounds to 12 significant digits, the precision at which float drift stops
 * mattering. Comparing raw floats would never settle: `floor(x / step) * step`
 * can land one ulp off a value that is already on the grid.
 */
function roundSignificant(value: number): number {
  return Number(value.toPrecision(12));
}

/**
 * The tick step for `[start, stop]` aiming at `count` intervals.
 *
 * Keeps only the leading digit of the raw interval, rounded to 1, 2 or 5 (the
 * thresholds are the geometric midpoints between those), so the step is a round
 * number at any magnitude.
 */
function tickStep(start: number, stop: number, count: number): number {
  const raw = (stop - start) / Math.max(1, count);
  const power = Math.floor(Math.log10(raw));
  const error = raw / 10 ** power;
  const factor = error >= Math.sqrt(50) ? 10 : error >= Math.sqrt(10) ? 5 : error >= Math.sqrt(2) ? 2 : 1;
  return factor * 10 ** power;
}

/**
 * A value axis whose ticks fall on round numbers.
 *
 * A domain padded by ±8% with evenly divided gridlines puts ticks at values like
 * 13.8k. Here the step is 1, 2 or 5 times a power of ten and the domain is
 * padded out to whole steps, so both ends are ticks. The step is chosen again
 * after each padding pass, because a wider domain can need a coarser step. The
 * pass count is a guard against a pathological input, not a tuning knob.
 */
export function niceScale(min: number, max: number, targetTicks = 5): NiceScale {
  const span = max - min;
  if (!Number.isFinite(min) || !Number.isFinite(max) || !Number.isFinite(span)) {
    throw new RangeError(`niceScale needs finite bounds, got [${min}, ${max}]`);
  }
  let lo = Math.min(min, max);
  let hi = Math.max(min, max);
  // A single value has no span to divide into steps, so it is widened to the
  // axis that contains it and zero.
  if (lo === hi) {
    if (lo > 0) lo = 0;
    else if (lo < 0) hi = 0;
    else hi = 1;
  }

  let step = 1;
  for (let pass = 0; pass < 10; pass++) {
    step = tickStep(lo, hi, targetTicks);
    const nextLo = Math.floor(lo / step) * step;
    const nextHi = Math.ceil(hi / step) * step;
    if (roundSignificant(nextLo) === roundSignificant(lo) && roundSignificant(nextHi) === roundSignificant(hi)) {
      break;
    }
    lo = nextLo;
    hi = nextHi;
  }

  // Integer multiples of the step, never a running sum: adding 0.1 ten times
  // drifts to 0.9999999999999999, while 10 * 0.1 is exact to the printed digits.
  const ticks: number[] = [];
  for (let i = Math.round(lo / step); i <= Math.round(hi / step); i++) {
    const value = roundSignificant(i * step);
    // -0 would print as "-0" in a label.
    ticks.push(value === 0 ? 0 : value);
  }
  return { min: ticks[0], max: ticks[ticks.length - 1], step, ticks };
}

export type TimeGranularity = "year" | "quarter" | "month" | "day" | "time";

export interface TimeTick {
  position: number;
  label: string;
}

export interface TimeAxis {
  granularity: TimeGranularity;
  /** One position per input label, in INPUT order. Units: year -> the year number; quarter and month -> months since year 0 (year*12 + month0; a quarter is year*12 + (q-1)*3); day -> days since 1970-01-01 UTC; time -> epoch milliseconds UTC. */
  positions: number[];
  /** Input indices sorted by position, ties keeping input order (stable). */
  order: number[];
  min: number; // smallest position
  max: number; // largest position
  ticks: TimeTick[]; // every position lies in [min, max]
}

const DAY_MS = 86_400_000;
const MINUTE_MS = 60_000;
const HOUR_MS = 3_600_000;

const YEAR_STEPS = [1, 2, 5, 10, 20, 50, 100, 200, 500, 1000];
const MONTH_STEPS = [1, 2, 3, 6, 12, 24, 60, 120, 240, 600];
const QUARTER_STEPS = [3, 6, 12, 24, 60, 120, 240, 600];
/** Month steps on a day-granularity axis, where a tick is the 1st of a month. */
const DAY_MONTH_STEPS = [1, 2, 3, 6];
const TIME_STEPS = [
  MINUTE_MS,
  5 * MINUTE_MS,
  15 * MINUTE_MS,
  30 * MINUTE_MS,
  HOUR_MS,
  3 * HOUR_MS,
  6 * HOUR_MS,
  12 * HOUR_MS,
  DAY_MS,
];
/**
 * 1970-01-01 was a Thursday, so epoch day 4 is the first Monday. Weekly ticks
 * anchored here land on Mondays rather than on whatever weekday the axis starts.
 */
const MONDAY_ANCHOR = 4;

interface ParsedLabel {
  granularity: TimeGranularity;
  position: number;
}

const MONTH_NUMBERS: ReadonlyMap<string, number> = new Map([
  ["january", 1],
  ["jan", 1],
  ["february", 2],
  ["feb", 2],
  ["march", 3],
  ["mar", 3],
  ["april", 4],
  ["apr", 4],
  ["may", 5],
  ["june", 6],
  ["jun", 6],
  ["july", 7],
  ["jul", 7],
  ["august", 8],
  ["aug", 8],
  ["september", 9],
  ["sep", 9],
  ["sept", 9],
  ["october", 10],
  ["oct", 10],
  ["november", 11],
  ["nov", 11],
  ["december", 12],
  ["dec", 12],
]);

/**
 * The number of an English month name or abbreviation, or null.
 *
 * A dot is accepted only after an abbreviation ("Sept.", "Jan."), never after a
 * full name, so "January." is rejected rather than read as a looser spelling.
 */
function monthNumber(name: string, dotted: boolean): number | null {
  const lower = name.toLowerCase();
  const abbreviated = lower.length === 3 || lower === "sept";
  if (dotted && !abbreviated) return null;
  return MONTH_NUMBERS.get(lower) ?? null;
}

function isRealDate(year: number, month: number, day: number): boolean {
  if (month < 1 || month > 12 || day < 1) return false;
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  const lengths = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return day <= lengths[month - 1];
}

/**
 * Days since 1970-01-01 UTC. Built with `setUTCFullYear` because `Date.UTC`
 * maps the years 0 to 99 onto 1900 to 1999.
 */
function epochDayOf(year: number, month: number, day: number): number {
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  return date.getTime() / DAY_MS;
}

function monthPosition(year: number, month0: number): number {
  return year * 12 + month0;
}

function monthParsed(year: number, month: number): ParsedLabel | null {
  if (month < 1 || month > 12) return null;
  return { granularity: "month", position: monthPosition(year, month - 1) };
}

function dayParsed(year: number, month: number, day: number): ParsedLabel | null {
  if (!isRealDate(year, month, day)) return null;
  return { granularity: "day", position: epochDayOf(year, month, day) };
}

function parseYear(text: string): ParsedLabel | null {
  if (!/^\d{4}$/.test(text)) return null;
  const year = Number(text);
  // A bare four-digit number is a year only inside a plausible range. "5000" is
  // a count or an id, and read as a year it would stretch the axis to AD 5000.
  if (year < 1900 || year > 2100) return null;
  return { granularity: "year", position: year };
}

function parseQuarter(text: string): ParsedLabel | null {
  const leading = /^(\d{4})[- ]?Q([1-4])$/i.exec(text);
  const trailing = /^Q([1-4])[- ](\d{4})$/i.exec(text);
  let year: number;
  let quarter: number;
  if (leading) {
    year = Number(leading[1]);
    quarter = Number(leading[2]);
  } else if (trailing) {
    quarter = Number(trailing[1]);
    year = Number(trailing[2]);
  } else {
    return null;
  }
  return { granularity: "quarter", position: monthPosition(year, (quarter - 1) * 3) };
}

function parseMonth(text: string): ParsedLabel | null {
  const numeric = /^(\d{4})-(\d{2})$/.exec(text) ?? /^(\d{4})\/(\d{1,2})$/.exec(text);
  if (numeric) return monthParsed(Number(numeric[1]), Number(numeric[2]));

  const named = /^([A-Za-z]+)(\.?) (\d{4})$/.exec(text);
  if (named) {
    const month = monthNumber(named[1], named[2] === ".");
    return month === null ? null : monthParsed(Number(named[3]), month);
  }

  const reversed = /^(\d{4})[- ]([A-Za-z]+)(\.?)$/.exec(text);
  if (reversed) {
    const month = monthNumber(reversed[2], reversed[3] === ".");
    return month === null ? null : monthParsed(Number(reversed[1]), month);
  }
  return null;
}

function parseDay(text: string): ParsedLabel | null {
  // The separator is captured and repeated, so "2024-01/05" is not accepted.
  const iso = /^(\d{4})([-/])(\d{2})\2(\d{2})$/.exec(text);
  if (iso) return dayParsed(Number(iso[1]), Number(iso[3]), Number(iso[4]));

  const dayMonthYear = /^(\d{1,2}) ([A-Za-z]+)(\.?) (\d{4})$/.exec(text);
  if (dayMonthYear) {
    const month = monthNumber(dayMonthYear[2], dayMonthYear[3] === ".");
    return month === null ? null : dayParsed(Number(dayMonthYear[4]), month, Number(dayMonthYear[1]));
  }

  const monthDayYear = /^([A-Za-z]+)(\.?) (\d{1,2}),? (\d{4})$/.exec(text);
  if (monthDayYear) {
    const month = monthNumber(monthDayYear[1], monthDayYear[2] === ".");
    return month === null ? null : dayParsed(Number(monthDayYear[4]), month, Number(monthDayYear[3]));
  }
  return null;
}

const TIME_TEXT =
  /^(\d{4})-(\d{2})-(\d{2})[T ](\d{2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?(Z|[+-]\d{2}:\d{2}|[+-]\d{4})?$/i;

/**
 * A date and time, in UTC unless it carries an offset.
 *
 * The offset is applied to the position rather than kept beside it: two labels
 * that name the same instant in different offsets must sort and tick as one
 * instant, and that only holds if both reduce to epoch milliseconds.
 */
function parseTime(text: string): ParsedLabel | null {
  const match = TIME_TEXT.exec(text);
  if (!match) return null;
  const [, yearText, monthText, dayText, hourText, minuteText, secondText = "0", fraction = "", zone = "Z"] = match;

  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const hour = Number(hourText);
  const minute = Number(minuteText);
  const second = Number(secondText);
  if (!isRealDate(year, month, day) || hour > 23 || minute > 59 || second > 59) return null;

  let offsetMinutes = 0;
  if (zone.toUpperCase() !== "Z") {
    const digits = zone.replace(/\D/g, "");
    const offsetHours = Number(digits.slice(0, 2));
    const offsetRest = Number(digits.slice(2, 4));
    if (offsetHours > 23 || offsetRest > 59) return null;
    offsetMinutes = (zone.startsWith("-") ? -1 : 1) * (offsetHours * 60 + offsetRest);
  }

  // Three digits, truncated: a float multiply would turn ".29" into 289.99999 and floor it to 289.
  const millis = Number(fraction.slice(0, 3).padEnd(3, "0"));
  const wallClockMs = epochDayOf(year, month, day) * DAY_MS + ((hour * 60 + minute) * 60 + second) * 1000 + millis;
  return { granularity: "time", position: wallClockMs - offsetMinutes * MINUTE_MS };
}

/**
 * One label's granularity and position, or null when no format matches.
 *
 * Each shape is anchored and strict. Ambiguous numeric dates such as `01/02/2024`
 * (day first or month first?) and bare month or weekday names are rejected
 * rather than guessed at.
 */
function parseLabel(text: string): ParsedLabel | null {
  return parseYear(text) ?? parseQuarter(text) ?? parseMonth(text) ?? parseDay(text) ?? parseTime(text);
}

interface Formatters {
  /** "Jan 2024" */
  month: Intl.DateTimeFormat;
  /** "Jan 5" */
  day: Intl.DateTimeFormat;
  /** "Jan 5, 2024" */
  dayWithYear: Intl.DateTimeFormat;
  /** "13:30" */
  time: Intl.DateTimeFormat;
}

function formatters(locale: string | undefined): Formatters {
  return {
    month: new Intl.DateTimeFormat(locale, { month: "short", year: "numeric", timeZone: "UTC" }),
    day: new Intl.DateTimeFormat(locale, { month: "short", day: "numeric", timeZone: "UTC" }),
    dayWithYear: new Intl.DateTimeFormat(locale, { month: "short", day: "numeric", year: "numeric", timeZone: "UTC" }),
    time: new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit", hour12: false, timeZone: "UTC" }),
  };
}

function mod(value: number, divisor: number): number {
  return ((value % divisor) + divisor) % divisor;
}

/**
 * Up to `cap` values in `[first, last]` that are multiples of `step` counted
 * from `offset`. Stopping at `cap` keeps a fine rung over a long span cheap: the
 * caller only needs to know whether the count is too large.
 */
function aligned(first: number, last: number, step: number, offset: number, cap: number): number[] {
  const out: number[] = [];
  for (let value = first + mod(offset - first, step); value <= last && out.length < cap; value += step) {
    out.push(value);
  }
  return out;
}

function yearOf(ms: number): number {
  return new Date(ms).getUTCFullYear();
}

function monthFirstDay(monthIndex: number): number {
  const year = Math.floor(monthIndex / 12);
  return epochDayOf(year, monthIndex - year * 12 + 1, 1);
}

function yearLabel(year: number): string {
  return String(year).padStart(4, "0");
}

function quarterLabel(monthIndex: number): string {
  const year = Math.floor(monthIndex / 12);
  const quarter = Math.floor((monthIndex - year * 12) / 3) + 1;
  return `Q${quarter} ${yearLabel(year)}`;
}

function monthLabel(fmt: Formatters, monthIndex: number): string {
  return fmt.month.format(new Date(monthFirstDay(monthIndex) * DAY_MS));
}

function dayLabel(fmt: Formatters, day: number, withYear: boolean): string {
  return (withYear ? fmt.dayWithYear : fmt.day).format(new Date(day * DAY_MS));
}

/**
 * The clock, with the date in front when the axis covers more than a day.
 *
 * Without the date a multi-day axis shows "00:00" four times with no way to tell
 * which day each one is.
 */
function timeLabel(fmt: Formatters, ms: number, withDate: boolean, withYear: boolean): string {
  const clock = fmt.time.format(new Date(ms));
  return withDate ? `${dayLabel(fmt, Math.floor(ms / DAY_MS), withYear)} ${clock}` : clock;
}

/** A tick candidate set, capped at `cap` entries. */
type Rung = (cap: number) => TimeTick[];

/**
 * The first rung whose tick count is at least two and at most `maxTicks`.
 *
 * Two is the floor because one tick cannot show a scale. Rungs run from the
 * finest step upward, so the first to fit is the densest that still fits.
 */
function firstFit(rungs: readonly Rung[], maxTicks: number): TimeTick[] | null {
  for (const rung of rungs) {
    // Asking for one more than the maximum is enough to tell "too many" apart from "fits".
    const ticks = rung(maxTicks + 1);
    if (ticks.length >= 2 && ticks.length <= maxTicks) return ticks;
  }
  return null;
}

/** The axis ends, labelled, used when no rung gives a usable count. */
function endpoints(min: number, max: number, label: (position: number) => string): TimeTick[] {
  return [
    { position: min, label: label(min) },
    { position: max, label: label(max) },
  ];
}

function yearRungs(min: number, max: number): Rung[] {
  return YEAR_STEPS.map(
    (step) => (cap) => aligned(min, max, step, 0, cap).map((year) => ({ position: year, label: yearLabel(year) })),
  );
}

/**
 * Rungs for month-unit positions. A step of a year or more labels by year, and
 * shorter steps use `pointLabel`, which is what makes a quarter axis read "Q1 2024".
 */
function monthRungs(min: number, max: number, steps: readonly number[], pointLabel: (month: number) => string): Rung[] {
  return steps.map(
    (step) => (cap) =>
      aligned(min, max, step, 0, cap).map((month) => ({
        position: month,
        label: step >= 12 ? yearLabel(Math.floor(month / 12)) : pointLabel(month),
      })),
  );
}

/**
 * Rungs for day-unit positions over `[lo, hi]`.
 *
 * Month and year rungs tick at the 1st and Jan 1, so the first candidate is
 * the first such date at or after `lo`. A month that starts before the window
 * must not be pulled in, or the tick would sit outside `[min, max]`.
 */
function dayRungs(lo: number, hi: number, withYear: boolean, fmt: Formatters): Rung[] {
  const dayRung =
    (step: number, anchor: number): Rung =>
    (cap) =>
      aligned(lo, hi, step, anchor, cap).map((day) => ({ position: day, label: dayLabel(fmt, day, withYear) }));

  const monthRung =
    (step: number): Rung =>
    (cap) => {
      const firstMonth = new Date(lo * DAY_MS);
      const startIndex = firstMonth.getUTCFullYear() * 12 + firstMonth.getUTCMonth();
      const start = monthFirstDay(startIndex) < lo ? startIndex + 1 : startIndex;
      const endDate = new Date(hi * DAY_MS);
      const end = endDate.getUTCFullYear() * 12 + endDate.getUTCMonth();
      return aligned(start, end, step, 0, cap).map((month) => ({
        position: monthFirstDay(month),
        label: monthLabel(fmt, month),
      }));
    };

  const yearRung =
    (step: number): Rung =>
    (cap) => {
      const startYear = yearOf(lo * DAY_MS);
      const first = epochDayOf(startYear, 1, 1) < lo ? startYear + 1 : startYear;
      return aligned(first, yearOf(hi * DAY_MS), step, 0, cap).map((year) => ({
        position: epochDayOf(year, 1, 1),
        label: yearLabel(year),
      }));
    };

  return [
    dayRung(1, 0),
    dayRung(2, 0),
    dayRung(7, MONDAY_ANCHOR),
    dayRung(14, MONDAY_ANCHOR),
    ...DAY_MONTH_STEPS.map(monthRung),
    ...YEAR_STEPS.map(yearRung),
  ];
}

function chooseTicks(
  granularity: TimeGranularity,
  min: number,
  max: number,
  maxTicks: number,
  fmt: Formatters,
): TimeTick[] {
  switch (granularity) {
    case "year":
      return firstFit(yearRungs(min, max), maxTicks) ?? endpoints(min, max, yearLabel);
    case "quarter":
      return firstFit(monthRungs(min, max, QUARTER_STEPS, quarterLabel), maxTicks) ?? endpoints(min, max, quarterLabel);
    case "month": {
      const pointLabel = (month: number) => monthLabel(fmt, month);
      return firstFit(monthRungs(min, max, MONTH_STEPS, pointLabel), maxTicks) ?? endpoints(min, max, pointLabel);
    }
    case "day": {
      const withYear = yearOf(min * DAY_MS) !== yearOf(max * DAY_MS);
      const pointLabel = (day: number) => dayLabel(fmt, day, withYear);
      return firstFit(dayRungs(min, max, withYear, fmt), maxTicks) ?? endpoints(min, max, pointLabel);
    }
    case "time": {
      const withDate = max - min > DAY_MS;
      const withYear = yearOf(min) !== yearOf(max);
      const pointLabel = (ms: number) => timeLabel(fmt, ms, withDate, withYear);
      const timeRungs: Rung[] = TIME_STEPS.map(
        (step) => (cap) => aligned(min, max, step, 0, cap).map((ms) => ({ position: ms, label: pointLabel(ms) })),
      );
      const timeFit = firstFit(timeRungs, maxTicks);
      if (timeFit) return timeFit;

      // Past a day the time rungs run out of room (the daily rung already has
      // too many midnights), so a coarser day grid takes over. Its ticks are
      // whole days inside [min, max], converted back to milliseconds.
      const dayFit = firstFit(dayRungs(Math.ceil(min / DAY_MS), Math.floor(max / DAY_MS), withYear, fmt), maxTicks);
      if (dayFit) return dayFit.map((tick) => ({ position: tick.position * DAY_MS, label: tick.label }));
      return endpoints(min, max, pointLabel);
    }
  }
}

/**
 * Resolves a categorical column of date-like labels into a time axis, or null
 * to keep the axis categorical.
 *
 * Null for any label that does not parse, any mix of granularities, and fewer
 * than two distinct positions. A single point or a column of one repeated date
 * has no spacing to show, so it stays a category axis rather than gaining a
 * meaningless scale.
 */
export function resolveTimeAxis(labels: readonly string[], locale?: string, maxTicks = 7): TimeAxis | null {
  if (labels.length < 2) return null;

  const parsed: ParsedLabel[] = [];
  for (const label of labels) {
    const result = parseLabel(label.trim());
    if (result === null) return null;
    parsed.push(result);
  }

  const granularity = parsed[0].granularity;
  if (parsed.some((entry) => entry.granularity !== granularity)) return null;

  const positions = parsed.map((entry) => entry.position);
  // reduce rather than Math.min(...positions): a long chart column would overflow the argument list.
  const min = positions.reduce((a, b) => Math.min(a, b), Infinity);
  const max = positions.reduce((a, b) => Math.max(a, b), -Infinity);
  if (min === max) return null;

  // The index tiebreak is what makes the sort stable across equal positions,
  // so two rows on the same date keep their input order.
  const order = positions
    .map((position, index) => ({ position, index }))
    .sort((a, b) => a.position - b.position || a.index - b.index)
    .map((entry) => entry.index);

  const ticks = chooseTicks(granularity, min, max, maxTicks, formatters(locale));
  return { granularity, positions, order, min, max, ticks };
}
