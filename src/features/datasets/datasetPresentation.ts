import type { DatasetErrorCode, DatasetFileView, DatasetRecipeStep, DatasetView } from "../../lib/generated/contracts";

/**
 * Everything the Datasets destination has to turn into words.
 *
 * The error copy lives here because the backend sends a code, never a sentence
 * — DA's own prose can name a presigned URL, a storage key, or a bucket, so it
 * is dropped at the boundary (`src-tauri/src/datasets/errors.rs`). One total
 * mapping means a new code cannot compile without someone writing what it says.
 */

/** What to tell a person about a refused datasets operation. */
export function describeError(code: DatasetErrorCode): string {
  switch (code) {
    case "auth_required":
      return "Sign in again to work with datasets.";
    case "not_configured":
      return "This device is not set up for a workspace yet.";
    case "uploads_disabled":
      return "Uploads are turned off for this workspace. An administrator can turn them back on.";
    case "update_pending":
      return "Restart xFalcon to finish the installed update before uploading another dataset.";
    case "stage_not_found":
      return "That file is no longer waiting to upload. Choose it again.";
    case "stage_limit_reached":
      return "Too many files are waiting to upload. Finish or discard one first.";
    case "no_file_chosen":
      return "No file was chosen.";
    case "unsupported_format":
      return "Only .csv and .xlsx files can become datasets.";
    case "content_mismatch":
      return "That file's contents do not match its extension. Re-save it and try again.";
    case "encrypted_file":
      return "That file is password-protected. Remove the password and try again.";
    case "empty_file":
      return "That file has no rows.";
    case "file_too_large":
      return "That file is larger than this workspace accepts.";
    case "unreadable":
      return "That file could not be read.";
    case "recipe_too_long":
      return "That is more cleanup steps than one upload can carry.";
    case "recipe_invalid_step":
      return "A cleanup step cannot be applied. Fix or remove it.";
    case "recipe_file_too_large":
      return "This file is too large to clean up before uploading. Upload it as it is.";
    case "not_found":
      return "That dataset is no longer here.";
    case "server_refused":
      return "The workspace refused that. You can try again.";
    case "network":
      return "The workspace could not be reached.";
    case "storage":
      return "That file could not be written.";
  }
}

/** The refusal a step-scoped error names, so the editor can point at it. */
export function describeStepError(code: DatasetErrorCode, stepIndex: number | null): string {
  if (code !== "recipe_invalid_step" || stepIndex === null) return describeError(code);
  return `Step ${stepIndex + 1} cannot be applied. It names a column that does not exist at that point, or a value that cannot be read.`;
}

/** A byte count in the units a person reads. */
export function formatBytes(bytes: number | null): string {
  if (bytes === null) return "Unknown";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

/**
 * A row or column count, with absence distinct from zero.
 *
 * A table of no rows and a table whose count the server never recorded are
 * different facts. Rendering both as `0` tells someone their data is empty.
 */
export function formatCount(count: number | null): string {
  if (count === null) return "Unknown";
  return count.toLocaleString();
}

/** How long ago, in the shape the Reports destination already uses. */
export function formatUploaded(epochMs: number): string {
  if (epochMs <= 0) return "Unknown";
  const elapsed = Date.now() - epochMs;
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  if (elapsed < minute) return "Just now";
  if (elapsed < hour) return `${Math.floor(elapsed / minute)}m ago`;
  if (elapsed < day) return `${Math.floor(elapsed / hour)}h ago`;
  if (elapsed < 7 * day) return `${Math.floor(elapsed / day)}d ago`;
  // A named month rather than the sidebar's `dd/mm/yyyy`. That form is right in
  // a 200px column where the date competes with a conversation title; here the
  // column has room, and `29/7` next to `2h ago` reads as a serial number
  // rather than as a date. The exact instant is a hover away — see
  // [`formatUploadedExact`].
  return new Date(epochMs).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
  });
}

/**
 * The full instant, for the title of whatever shows [`formatUploaded`].
 *
 * "2h ago" is the right thing to read and the wrong thing to reason about. A
 * person deciding which of two uploads is the one they want should not have to
 * open anything to find out.
 */
export function formatUploadedExact(epochMs: number): string {
  if (epochMs <= 0) return "Not recorded";
  return new Date(epochMs).toLocaleString(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
}

/** Newest first, or by name, in the direction asked for. */
export type DatasetSortKey = "name" | "uploaded";
export type DatasetSortDirection = "asc" | "desc";

export function sortDatasets(
  datasets: DatasetView[],
  key: DatasetSortKey,
  direction: DatasetSortDirection,
): DatasetView[] {
  const sign = direction === "asc" ? 1 : -1;
  return [...datasets].sort((left, right) => {
    if (key === "name") {
      return sign * left.display_name.localeCompare(right.display_name);
    }
    if (left.created_at_ms === right.created_at_ms) {
      return left.display_name.localeCompare(right.display_name);
    }
    return sign * (left.created_at_ms > right.created_at_ms ? 1 : -1);
  });
}

/** A dataset's totals, summed over its files. */
export function totals(files: DatasetFileView[]): { rows: number | null; columns: number | null } {
  let rows: number | null = null;
  let columns: number | null = null;
  for (const file of files) {
    if (file.row_count !== null) rows = (rows ?? 0) + Number(file.row_count);
    if (file.column_count !== null) columns = (columns ?? 0) + file.column_count;
  }
  return { rows, columns };
}

/** One cleanup step, as a line a person can check before uploading. */
export function describeStep(step: DatasetRecipeStep): string {
  switch (step.kind) {
    case "filter": {
      if (step.op === "is_null") return `keep rows where ${step.column} is empty`;
      if (step.op === "is_not_null") return `keep rows where ${step.column} is not empty`;
      const operator = FILTER_WORDS[step.op];
      return `keep rows where ${step.column} ${operator} "${step.value ?? ""}"`;
    }
    case "sort":
      return `sort by ${step.column}, ${step.descending ? "largest first" : "smallest first"}`;
    case "rename":
      return `rename ${step.column} to ${step.new_name}`;
    case "cast":
      return `read ${step.column} as ${step.cast_to}`;
    case "trim":
      return `trim spaces in ${step.column}`;
    case "upper":
      return `uppercase ${step.column}`;
    case "lower":
      return `lowercase ${step.column}`;
    case "replace":
      return `in ${step.column}, replace "${step.find}" with "${step.replace_with}"`;
    case "split":
      return `split ${step.column} on "${step.delimiter}" into ${step.parts}`;
    case "dedup":
      return "drop duplicate rows";
    case "remove_nulls":
      return `drop rows where ${step.column} is empty`;
    case "fill_nulls":
      return `fill empty ${step.column} with "${step.value}"`;
  }
}

const FILTER_WORDS: Record<string, string> = {
  equals: "is",
  not_equals: "is not",
  greater_than: "is above",
  less_than: "is below",
  at_least: "is at least",
  at_most: "is at most",
  contains: "contains",
};
