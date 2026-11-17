import { formatNumber } from "./formatNumber";

/**
 * Byte counts for humans, in one place so every surface that reports a size
 * (approval plans, withheld file metadata) spells the same number the same way.
 * The number follows the host locale; the unit word (KB, MB, GB) does not.
 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${formatNumber(bytes, { maximumFractionDigits: 0 })} B`;
  const units = ["KB", "MB", "GB"];
  let value = bytes / 1024;
  let unitIndex = 0;
  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const digits = value >= 10 ? 0 : 1;
  return `${formatNumber(value, { minimumFractionDigits: digits, maximumFractionDigits: digits })} ${units[unitIndex]}`;
}
