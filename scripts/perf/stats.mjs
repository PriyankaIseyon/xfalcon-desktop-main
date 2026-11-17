// Summary statistics shared by the perf scripts.

// Median of a non-empty list. For an even count it averages the two middle values.
export function median(xs) {
  if (xs.length === 0) throw new Error("median of no samples");
  const s = [...xs].sort((a, b) => a - b);
  const mid = Math.floor(s.length / 2);
  return s.length % 2 ? s[mid] : (s[mid - 1] + s[mid]) / 2;
}
