/**
 * The short type tag the file tree draws in place of a generic file icon —
 * `CSV`, `HTML`, `SQL` — each in its own colour, so a folder of mixed files can
 * be scanned by type. `null` for a file with no extension (`LICENSE`, a
 * dotfile), which keeps the plain file icon.
 */
export type FileTypeTone = "blue" | "red" | "cyan" | "amber" | "violet" | "green" | "orange" | "pink" | "muted";

export interface FileTypeLabel {
  text: string;
  tone: FileTypeTone;
}

/** Extensions whose tag is not simply the extension, upper-cased. */
const TAG_ALIAS: Readonly<Record<string, string>> = {
  htm: "HTML",
  markdown: "MD",
  ndjson: "JSONL",
  sqlite: "DB",
  sqlite3: "DB",
  duckdb: "DB",
  parquet: "PARQ",
  yml: "YAML",
};

const TONE: Readonly<Record<string, FileTypeTone>> = {
  // Office files: Word blue, PowerPoint orange, Excel green, as their own icons.
  doc: "blue",
  docx: "blue",
  ppt: "orange",
  pptx: "orange",
  csv: "cyan",
  tsv: "cyan",
  tab: "cyan",
  html: "red",
  htm: "red",
  xhtml: "red",
  svg: "orange",
  json: "amber",
  jsonl: "amber",
  ndjson: "amber",
  md: "violet",
  markdown: "violet",
  sql: "green",
  xlsx: "green",
  xls: "green",
  parquet: "cyan",
  sqlite: "cyan",
  sqlite3: "cyan",
  db: "cyan",
  duckdb: "cyan",
  pdf: "red",
  png: "orange",
  jpg: "orange",
  jpeg: "orange",
  gif: "orange",
  webp: "orange",
  py: "pink",
  r: "pink",
  ipynb: "pink",
  js: "pink",
  ts: "pink",
  tsx: "pink",
  jsx: "pink",
  rs: "pink",
  go: "pink",
  java: "pink",
  sh: "pink",
  ps1: "pink",
  dax: "pink",
  kql: "pink",
  yaml: "violet",
  yml: "violet",
  toml: "violet",
  xml: "orange",
};

/** Tags stay short enough to sit in the fixed slot beside the name. */
const MAX_TAG_CHARS = 5;

export function fileTypeLabel(path: string): FileTypeLabel | null {
  const name = path.split("/").pop() ?? "";
  const dot = name.lastIndexOf(".");
  // No extension, or a dotfile such as `.gitignore`, whose "extension" is its name.
  if (dot <= 0 || dot === name.length - 1) return null;
  const extension = name.slice(dot + 1).toLowerCase();
  const text = TAG_ALIAS[extension] ?? extension.toUpperCase();
  // A tag cut short reads as a typo; a long, rare extension keeps the icon.
  if (text.length > MAX_TAG_CHARS) return null;
  return { text, tone: TONE[extension] ?? "muted" };
}

/** Longer names are cut in the middle, keeping their end (and so their
 * extension) visible; the whole name is on hover. */
export const MAX_TREE_NAME_CHARS = 36;

export function cappedName(name: string): string {
  const characters = Array.from(name);
  if (characters.length <= MAX_TREE_NAME_CHARS) return name;
  const tail = 12;
  return `${characters.slice(0, MAX_TREE_NAME_CHARS - tail - 1).join("")}…${characters.slice(-tail).join("")}`;
}
