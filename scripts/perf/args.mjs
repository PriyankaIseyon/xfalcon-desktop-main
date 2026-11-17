// CLI helpers shared by the perf scripts. Validation runs before any browser or server starts.
export const LABEL_PATTERN = /^[A-Za-z0-9._-]+$/;

// The value after --name, or dflt when the flag is absent. A missing value, another flag or an empty string is a usage error.
export function arg(name, dflt) {
  const i = process.argv.indexOf(`--${name}`);
  if (i < 0) return dflt;
  const value = process.argv[i + 1];
  if (value === undefined || value.startsWith("--")) usageError(`--${name} needs a value`);
  if (value === "") usageError(`--${name} needs a non-empty value`);
  return value;
}

export function usageError(message) {
  console.error(message);
  process.exit(1);
}

// Numeric flag: a finite decimal number from min to max. integer: true also rejects fractions.
// Anything else (negative, NaN, exponent, text) is a usage error.
export function numArg(name, dflt, min, { integer = false, max = Infinity } = {}) {
  const v = arg(name, undefined);
  if (v === undefined) return dflt;
  const n = Number(v);
  const shape = integer ? /^\d+$/ : /^\d+(\.\d+)?$/;
  if (!shape.test(v) || !Number.isFinite(n) || n < min || n > max) {
    const kind = integer ? "an integer" : "a number";
    const range = max === Infinity ? `>= ${min}` : `from ${min} to ${max}`;
    usageError(`--${name} must be ${kind} ${range}, got "${v}"`);
  }
  return n;
}

export function intArg(name, dflt, min, max = Infinity) {
  return numArg(name, dflt, min, { integer: true, max });
}
