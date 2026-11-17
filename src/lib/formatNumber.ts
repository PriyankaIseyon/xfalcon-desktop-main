/**
 * Showing a number follows the reader's locale: separators and digits come from the
 * host, or from the locale passed to createNumberFormatter. Reading a number (parsing)
 * is a separate concern that follows the data, not the machine; this module only shows.
 */

export interface FractionDigits {
  minimumFractionDigits?: number;
  maximumFractionDigits?: number;
}

export interface NumberFormatter {
  /** Full-precision display. Defaults match toLocaleString(): min 0, max 3 fraction digits. */
  format(value: number, digits?: FractionDigits): string;
  /** Axis tick: magnitude suffix k/M/B/T chosen per value, fraction digits derived from the tick step. */
  formatTick(value: number, step: number): string;
}

const NOT_A_NUMBER = "—";
const MAX_TICK_DIGITS = 6;

// Ordered largest first. Suffixes stay English: only the numerals and separators are localised.
const TICK_UNITS: ReadonlyArray<{ divisor: number; suffix: string }> = [
  { divisor: 1e12, suffix: "T" },
  { divisor: 1e9, suffix: "B" },
  { divisor: 1e6, suffix: "M" },
  { divisor: 1e3, suffix: "k" },
];

/** Intl prints -0 as "-0"; the reader should see "0". */
function withoutNegativeZero(value: number): number {
  return value === 0 ? 0 : value;
}

/** Fraction digits needed to show a scaled step exactly, so a 2.5k step never rounds to 3k. */
function tickDigits(scaledStep: number): number {
  if (!Number.isFinite(scaledStep) || scaledStep <= 0) return 0;
  for (let digits = 0; digits < MAX_TICK_DIGITS; digits += 1) {
    const shifted = scaledStep * 10 ** digits;
    if (Math.abs(shifted - Math.round(shifted)) < 1e-9 * Math.max(1, shifted)) return digits;
  }
  return MAX_TICK_DIGITS;
}

export function createNumberFormatter(locales?: string | string[]): NumberFormatter {
  const instances = new Map<string, Intl.NumberFormat>();

  function instanceFor(digits: FractionDigits | undefined): Intl.NumberFormat {
    const minimum = digits?.minimumFractionDigits ?? 0;
    const maximum = digits?.maximumFractionDigits ?? 3;
    const key = `${minimum}:${maximum}`;
    let instance = instances.get(key);
    if (instance === undefined) {
      instance = new Intl.NumberFormat(locales, { minimumFractionDigits: minimum, maximumFractionDigits: maximum });
      instances.set(key, instance);
    }
    return instance;
  }

  return {
    format(value, digits) {
      if (!Number.isFinite(value)) return NOT_A_NUMBER;
      return instanceFor(digits).format(withoutNegativeZero(value));
    },
    formatTick(value, step) {
      if (!Number.isFinite(value)) return NOT_A_NUMBER;
      const magnitude = Math.abs(value);
      const unit = TICK_UNITS.find((candidate) => magnitude >= candidate.divisor) ?? { divisor: 1, suffix: "" };
      const digits = tickDigits(Math.abs(step) / unit.divisor);
      const scaled = instanceFor({ maximumFractionDigits: digits }).format(withoutNegativeZero(value) / unit.divisor);
      return `${scaled}${unit.suffix}`;
    },
  };
}

const defaultFormatter = createNumberFormatter();

export function formatNumber(value: number, digits?: FractionDigits): string {
  return defaultFormatter.format(value, digits);
}

export function formatTick(value: number, step: number): string {
  return defaultFormatter.formatTick(value, step);
}
