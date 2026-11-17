/**
 * Fence-content detection ladder (spec: chat-rich-content). Semantics are
 * ported from xfalcon-web (`lib/artifacts/fenced.ts`,
 * `detectInlineHtmlDocument`, `MarkdownRenderer`'s JSON-table gate) so the
 * two surfaces classify the same content the same way. Every function here
 * reads TEXT and returns data — nothing injects HTML.
 */

/** Web parity: a document span shorter than this is noise, not a document. */
const MIN_HTML_DOC_LENGTH = 100;
/** Preview payload cap — beyond this the source renders as code instead. */
export const HTML_PREVIEW_MAX_BYTES = 2 * 1024 * 1024;

const HTML_DOC_START_RE = /<!doctype\s+html|<html[\s>]/i;
const HTML_DOC_END_RE = /<\/html>/i;
const HTML_DOC_END_LENGTH = "</html>".length;

/** Web's `detectInlineHtmlDocument`, verbatim gate: full-document content in
 * an unlabelled or html-ish fence. */
export function detectHtmlDocument(language: string, code: string): boolean {
  const lang = language.toLowerCase();
  const normalized = code.trim().toLowerCase();
  const looksLikeHtml = normalized.startsWith("<!doctype html") || normalized.startsWith("<html");
  return looksLikeHtml && (lang === "" || lang === "html" || lang === "htm" || lang === "xml");
}

/** The document SPAN inside `text` (first start marker to FIRST `</html>`),
 * or null — web renders the span, not the raw fence body, so stray prose
 * around the document never leaks into the preview. */
export function extractHtmlDocument(text: string): string | null {
  const startMatch = HTML_DOC_START_RE.exec(text);
  if (!startMatch) return null;
  const start = startMatch.index;
  // Matched against the original text, not a lowercased copy: `toLowerCase`
  // is not length-preserving for every code point (U+0130 expands to two
  // chars), and an offset taken from the copy would then slice the original
  // at the wrong place.
  const closeMatch = HTML_DOC_END_RE.exec(text.slice(start));
  if (!closeMatch) return null;
  const end = start + closeMatch.index + HTML_DOC_END_LENGTH;
  if (end - start < MIN_HTML_DOC_LENGTH) return null;
  return text.slice(start, end);
}

/** Spaces and tabs only — the two languages agree on this set, where a bare
 * `trim()` would also eat vertical whitespace and Unicode spaces that Rust's
 * `str::trim` treats differently. */
function trimIndent(line: string): string {
  let start = 0;
  let end = line.length;
  while (start < end && (line[start] === " " || line[start] === "\t")) start += 1;
  while (end > start && (line[end - 1] === " " || line[end - 1] === "\t")) end -= 1;
  return line.slice(start, end);
}

type OpenFence = { fence: string; language: string };

/** An opening fence line: three or more backticks or tildes, then an info
 * string carrying neither fence character. CommonMark names the language as the
 * FIRST word of that info string, which is also what remark puts in the
 * `language-*` class the renderer reads. */
function parseOpenFence(line: string): OpenFence | null {
  const trimmed = trimIndent(line);
  const marker = trimmed[0];
  if (marker !== "`" && marker !== "~") return null;
  let length = 0;
  while (length < trimmed.length && trimmed[length] === marker) length += 1;
  if (length < 3) return null;
  const info = trimIndent(trimmed.slice(length));
  if (info.includes("`") || info.includes("~")) return null;
  return { fence: trimmed.slice(0, length), language: (info.split(/\s+/)[0] ?? "").toLowerCase() };
}

/**
 * Every HTML document in one message's raw text, in the order the renderer
 * draws them.
 *
 * This is one half of a contract with `find_html_documents` in
 * `src-tauri/src/reports/capture.rs`: the renderer supplies a document's
 * ordinal and the backend resolves it, so index `n` must mean the same
 * document on both sides. Both are pinned to `./htmlDocumentFixtures.json` —
 * a case added for one is enforced on the other. Display-time detection stays
 * here and resolution stays in Rust; the fixtures are what keeps them from
 * drifting, which is the worst failure this feature can have (the user saves a
 * document other than the one they clicked, and nothing says so).
 *
 * Line-based rather than one regex, because the JS regex web uses leans on a
 * `\2` backreference that Rust's `regex` crate does not support at all — two
 * different algorithms pinned by fixtures is a weaker guarantee than one
 * algorithm written twice.
 *
 * The ladder is `AssistantMessage`'s, not a second one: a fence whose language
 * and body satisfy {@link detectHtmlDocument} contributes
 * `extractHtmlDocument(body) ?? body`, the exact string that fence's card
 * renders, copies, and shows as source. Consuming a closed fence whole is what
 * gives the renderer's `depth === 0` gate for free — a ``` fence inside a
 * ```` block is never seen as a block of its own. Unfenced documents are
 * absent because desktop renders none; counting one would offer to save a
 * document the reader was never shown a card for.
 */
export function findHtmlDocuments(text: string): string[] {
  const lines = text.split("\n");
  const documents: string[] = [];
  let index = 0;
  while (index < lines.length) {
    const open = parseOpenFence(lines[index] ?? "");
    if (open === null) {
      index += 1;
      continue;
    }
    let cursor = index + 1;
    let closed = false;
    const body: string[] = [];
    while (cursor < lines.length) {
      if (trimIndent(lines[cursor] ?? "") === open.fence) {
        closed = true;
        break;
      }
      body.push(lines[cursor] ?? "");
      cursor += 1;
    }
    // An unclosed fence is not a block: the renderer has no settled fence to
    // draw a card for either. Nothing after it can be a block either, since
    // everything left is inside it.
    if (!closed) break;
    const code = body.join("\n");
    if (detectHtmlDocument(open.language, code)) {
      documents.push(extractHtmlDocument(code) ?? code);
    }
    index = cursor + 1;
  }
  return documents;
}

/**
 * Which document of `documents` is `html`, or -1. Lets a card that holds only
 * its own document tell the save path which one of the message it is showing,
 * without threading a counter through the markdown tree — a counter incremented
 * as fence components mount would be right only by luck, because the pipeline
 * may memoize and re-render them in an order unrelated to the document.
 */
export function htmlDocumentIndex(documents: string[], html: string): number {
  const exact = documents.indexOf(html);
  if (exact !== -1) return exact;
  const target = html.trim();
  return documents.findIndex((document) => document.trim() === target);
}

/**
 * Whether a document references a script or stylesheet it cannot load. The
 * preview scheme allows no network at all, so such a reference is dead on
 * every open — and a saved report freezes whatever it renders, so the reader
 * is told before they commit to keeping it. Advisory only: refusing a working
 * dashboard over a false positive would be worse than the warning.
 */
export function hasExternalReference(html: string): boolean {
  return /<script\b[^>]*\bsrc\s*=/i.test(html) || /<link\b[^>]*\brel\s*=\s*["']?stylesheet/i.test(html);
}

/** `<title>` text of a document, for the preview card header. */
export function resolveHtmlTitle(html: string): string | null {
  const match = /<title[^>]*>([^<]*)<\/title>/i.exec(html);
  const title = match?.[1]?.trim();
  return title ? title : null;
}

/** Web parity (`STANDALONE_SVG_RE` + the ```svg artifact rung): an `svg`
 * fence, or an unlabelled fence whose whole body is one standalone SVG
 * document (optional XML prolog/comments, `<svg …>` … `</svg>`). */
const SVG_DOC_START_RE = /^\s*(?:<\?xml[^>]*\?>\s*)?(?:<!--[\s\S]*?-->\s*)*<svg[\s>]/i;

export function detectSvgDocument(language: string, code: string): boolean {
  const lang = language.toLowerCase();
  const body = code.trim();
  const looksLikeSvg = SVG_DOC_START_RE.test(body) && body.toLowerCase().endsWith("</svg>");
  if (lang === "svg") return looksLikeSvg;
  return lang === "" && looksLikeSvg;
}

export type TableData = {
  headers: string[];
  rows: string[][];
};

/** Union of keys across the sampled rows — sparse objects render empty cells. */
const KEY_SAMPLE_ROWS = 50;

function cellText(value: unknown): string {
  if (value === null || value === undefined) return "";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

/** Web's JSON-table gate: a ```json fence whose body is a non-empty array of
 * objects. Returns tabular data or null (fall through to the code card). */
export function parseJsonTable(code: string): TableData | null {
  if (!code.trimStart().startsWith("[")) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(code);
  } catch {
    return null;
  }
  return objectRowsTable(parsed);
}

/** The table a parsed JSON value draws: a non-empty array whose every item is
 * an object, or null. Split from {@link parseJsonTable} so a JSON Lines file,
 * which arrives as one object per line, is tabled by the same rule. */
export function objectRowsTable(parsed: unknown): TableData | null {
  if (!Array.isArray(parsed) || parsed.length === 0) return null;
  const objects = parsed.filter(
    (row): row is Record<string, unknown> => typeof row === "object" && row !== null && !Array.isArray(row),
  );
  if (objects.length !== parsed.length) return null;
  const headers: string[] = [];
  for (const row of objects.slice(0, KEY_SAMPLE_ROWS)) {
    for (const key of Object.keys(row)) {
      if (!headers.includes(key)) headers.push(key);
    }
  }
  if (headers.length === 0) return null;
  return {
    headers,
    rows: objects.map((row) => headers.map((key) => cellText(row[key]))),
  };
}

/**
 * Minimal RFC-4180: quoted fields, `""` escapes, CRLF/LF. Structural failures
 * (unterminated quote, fewer than two rows, a blank header) return null — the
 * code card is the fallback.
 *
 * The delimiter is a PARAMETER, not a second parser. A warehouse CLI export is
 * as likely to be tab-separated as comma-separated, and the quoting, escape, and
 * ragged-row rules are the whole reason this function exists — written twice
 * they would drift on exactly the cases they are here to get right. A quoted
 * field containing the delimiter is one cell either way.
 */
export function parseDelimited(code: string, delimiter: string): TableData | null {
  const text = code.replace(/\r\n/g, "\n").replace(/\n+$/, "");
  if (!text.trim()) return null;
  const rows: string[][] = [];
  let field = "";
  let row: string[] = [];
  let inQuotes = false;
  let index = 0;
  while (index < text.length) {
    const char = text[index];
    if (inQuotes) {
      if (char === '"') {
        if (text[index + 1] === '"') {
          field += '"';
          index += 2;
          continue;
        }
        inQuotes = false;
        index += 1;
        continue;
      }
      field += char;
      index += 1;
      continue;
    }
    if (char === '"' && field === "") {
      inQuotes = true;
      index += 1;
      continue;
    }
    if (char === delimiter) {
      row.push(field);
      field = "";
      index += 1;
      continue;
    }
    if (char === "\n") {
      row.push(field);
      rows.push(row);
      row = [];
      field = "";
      index += 1;
      continue;
    }
    field += char;
    index += 1;
  }
  if (inQuotes) return null;
  row.push(field);
  rows.push(row);
  if (rows.length < 2) return null;
  const [headers, ...body] = rows;
  if (!headers || headers.every((cell) => cell.trim() === "")) return null;
  // Tolerate ragged rows (pad/truncate to the header width) — real exports
  // are messy; only structural quote failures reject the table.
  const width = headers.length;
  return {
    headers,
    rows: body.map((cells) =>
      cells.length === width
        ? cells
        : cells.slice(0, width).concat(Array.from({ length: Math.max(0, width - cells.length) }, () => "")),
    ),
  };
}
