import { useMemo, type ReactNode } from "react";
import { AssistantMessage } from "../chat/AssistantMessage";
import { ArtifactFrame, HtmlArtifact, MarkdownArtifact, SvgArtifact, TableArtifact } from "../chat/richContent/cards";
import { ArtifactHostActions } from "../chat/richContent/artifactActionsContext";
import { resolveLanguage } from "../chat/richContent/codeLanguages";
import {
  HTML_PREVIEW_MAX_BYTES,
  objectRowsTable,
  parseDelimited,
  parseJsonTable,
  type TableData,
} from "../chat/richContent/detect";
import { highlightLines, renderHighlightedLine, type HighlightToken } from "../chat/richContent/highlight";
import { projectPreviewRegister } from "../../lib/native";
import { PROJECT_FILE_LIMITS } from "./projectFileLimits";
import type { ProjectFileRendering } from "./projectFileRendering";
import { DISPLAY_LINE_CHARS, displayLine, lineColumns, longestLine, splitLines } from "./fileLines";
import { VirtualLines } from "./VirtualLines";

/**
 * A project file held whole, drawn the way the chat draws its type (spec:
 * project-file-editing).
 *
 * The documents, tables and prose are the chat's own artifact cards, so a
 * dashboard in `reports/` looks exactly like the card it was saved from. The
 * file adds no trust of its own: HTML and SVG load in the chat's sandboxed
 * `xf-preview` frame — registered from disk by the backend, so the document
 * never crosses IPC — Markdown passes the chat's sanitizer, and cells and code
 * are text nodes. Text of any length renders as virtualized lines: only what is
 * on screen exists in the DOM.
 */

type Body =
  | { kind: "html" }
  | { kind: "svg" }
  | { kind: "markdown" }
  | { kind: "table"; table: TableData }
  | { kind: "code"; tag: string; lines: HighlightToken[][] }
  | { kind: "lines"; note: string | null };

const NOT_A_TABLE = "This file could not be read as a table, so it is shown as text.";
const TOO_LARGE_TO_HIGHLIGHT = "This file is too large to highlight, so it is shown as plain text.";
const TOO_LARGE_FOR_TABLE = "This file is too large to parse here, so it is shown as text.";
const TOO_LARGE_FOR_PROSE = "This file is too large to render as prose, so it is shown as text.";
const TOO_LARGE_TO_PREVIEW = "This file is too large to preview, so it is shown as text.";

/** U+FEFF at a file's start is its byte-order mark. The whole load keeps it, so
 * a save writes the file back byte for byte; the parsers must not see it — a
 * JSON file with a BOM is not JSON to `JSON.parse`, and a CSV's first header
 * would carry it. */
function withoutByteOrderMark(text: string): string {
  return text.charCodeAt(0) === 0xfeff ? text.slice(1) : text;
}

/** One object per non-blank line, or `null` if any line is not JSON. */
function parseJsonLines(text: string): unknown[] | null {
  const rows: unknown[] = [];
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    try {
      rows.push(JSON.parse(line));
    } catch {
      return null;
    }
  }
  return rows;
}

function codeBody(tag: string, text: string, sizeBytes: number): Body {
  if (sizeBytes > PROJECT_FILE_LIMITS.highlightBytes) return { kind: "lines", note: TOO_LARGE_TO_HIGHLIGHT };
  const lines = highlightLines(tag, text);
  return lines === null ? { kind: "lines", note: null } : { kind: "code", tag, lines };
}

/** The bounds are in bytes, as `PROJECT_FILE_LIMITS` states them, so they are
 * measured against the file's size, not the decoded string's length. */
function resolveBody(rendering: ProjectFileRendering, text: string, sizeBytes: number): Body {
  const tableSized = sizeBytes <= PROJECT_FILE_LIMITS.inMemoryTableBytes;
  switch (rendering.kind) {
    case "html":
    case "svg":
      return sizeBytes <= PROJECT_FILE_LIMITS.previewBytes
        ? { kind: rendering.kind }
        : { kind: "lines", note: TOO_LARGE_TO_PREVIEW };
    case "markdown":
      return sizeBytes <= PROJECT_FILE_LIMITS.markdownBytes
        ? { kind: "markdown" }
        : { kind: "lines", note: TOO_LARGE_FOR_PROSE };
    case "delimited": {
      if (!tableSized) return { kind: "lines", note: TOO_LARGE_FOR_TABLE };
      const table = parseDelimited(text, rendering.delimiter);
      return table === null ? { kind: "lines", note: NOT_A_TABLE } : { kind: "table", table };
    }
    case "json": {
      const table = tableSized ? parseJsonTable(text) : null;
      return table === null ? codeBody("json", text, sizeBytes) : { kind: "table", table };
    }
    case "jsonl": {
      const rows = tableSized ? parseJsonLines(text) : null;
      const table = rows === null ? null : objectRowsTable(rows);
      return table === null ? codeBody("jsonl", text, sizeBytes) : { kind: "table", table };
    }
    case "code":
      return codeBody(rendering.tag, text, sizeBytes);
    case "plain":
      return { kind: "lines", note: null };
  }
}

/** A file's own actions (reload, edit), on one row, for a file shown without a
 * card to carry them. */
export function FileActionsRow({ children }: { children: ReactNode }) {
  return <div className="flex flex-wrap items-center justify-end gap-2">{children}</div>;
}

/** A file's text as numbered, virtualized lines — the fallback for every type,
 * and the whole view for a type with no richer rendering. */
export function FileLines({ path, text, note = null }: { path: string; text: string; note?: string | null }) {
  const lines = useMemo(() => splitLines(text), [text]);
  const longest = useMemo(() => longestLine(lines), [lines]);
  return (
    <>
      {note ? <p className="type-caption text-muted">{note}</p> : null}
      <VirtualLines
        count={lines.length}
        firstLineNumber={1}
        label={`${path} contents`}
        maxLineColumns={longest}
        renderLine={(index) => displayLine(lines[index] ?? "")}
      />
    </>
  );
}

function tokensLength(tokens: HighlightToken[]): number {
  let length = 0;
  for (const token of tokens) length += token.text.length;
  return length;
}

/** Highlighted code as virtualized lines inside the chat's code card frame. */
function CodeLines({ language, text, lines }: { language: string; text: string; lines: HighlightToken[][] }) {
  // The empty line after a trailing newline is not a line, as in `splitLines`.
  const trimmed = useMemo(
    () => (lines.length > 1 && lines[lines.length - 1]!.length === 0 ? lines.slice(0, -1) : lines),
    [lines],
  );
  const longest = useMemo(
    () => trimmed.reduce((max, line) => Math.max(max, lineColumns(line.map((token) => token.text).join(""))), 0),
    [trimmed],
  );
  return (
    <ArtifactFrame
      description={`${trimmed.length.toLocaleString()} line${trimmed.length === 1 ? "" : "s"}`}
      source={text}
      title={language}
    >
      <VirtualLines
        count={trimmed.length}
        firstLineNumber={1}
        label={`${language} source`}
        maxLineColumns={longest}
        renderLine={(index) => {
          const tokens = trimmed[index] ?? [];
          // A line past the display bound is shown plain and cut: its tokens
          // would be one enormous row either way.
          if (tokensLength(tokens) > DISPLAY_LINE_CHARS) {
            return displayLine(tokens.map((token) => token.text).join(""));
          }
          return <code className="hljs">{renderHighlightedLine(tokens)}</code>;
        }}
      />
    </ArtifactFrame>
  );
}

export function ProjectFileRichView({
  projectId,
  path,
  rendering,
  text: fileText,
  sizeBytes,
  actions,
}: {
  projectId: string;
  path: string;
  rendering: ProjectFileRendering;
  /** The whole file, as read. */
  text: string;
  /** The file's size in bytes, from the stat it was read at. */
  sizeBytes: number;
  /** File-level actions for the card's action row (see `ArtifactHostActions`). */
  actions?: ReactNode;
}) {
  const text = useMemo(() => withoutByteOrderMark(fileText), [fileText]);
  const body = useMemo(() => resolveBody(rendering, text, sizeBytes), [rendering, text, sizeBytes]);
  const fileName = path.split("/").pop() ?? path;

  if (body.kind === "lines") {
    return (
      <div className="flex min-h-0 flex-1 flex-col gap-2">
        {actions ? <FileActionsRow>{actions}</FileActionsRow> : null}
        <FileLines note={body.note} path={path} text={text} />
      </div>
    );
  }

  // The side panel re-registers a document from its content after an
  // eviction, through the renderer's own bound; past it the panel is withheld.
  const panelSized = sizeBytes <= HTML_PREVIEW_MAX_BYTES;
  let card: ReactNode;
  switch (body.kind) {
    case "html":
      card = (
        <HtmlArtifact
          allowPanel={panelSized}
          html={text}
          maxBytes={PROJECT_FILE_LIMITS.previewBytes}
          register={() => projectPreviewRegister(projectId, path, "html")}
        />
      );
      break;
    case "svg":
      card = (
        <SvgArtifact
          allowPanel={panelSized}
          maxBytes={PROJECT_FILE_LIMITS.previewBytes}
          register={() => projectPreviewRegister(projectId, path, "svg")}
          svg={text.trim()}
        />
      );
      break;
    case "markdown":
      // The whole chat ladder, not the one-level nested one a markdown FENCE
      // gets: a README is a document in its own right, and its tables and
      // fenced dashboards should render as they would in an answer.
      card = (
        <MarkdownArtifact source={text} sourceView={<code>{text}</code>}>
          <AssistantMessage text={text} />
        </MarkdownArtifact>
      );
      break;
    case "table":
      card = (
        <TableArtifact
          allowRowFallback={rendering.kind === "delimited"}
          source={text}
          sourceView={<code>{text}</code>}
          table={body.table}
          title={fileName}
        />
      );
      break;
    case "code":
      card = <CodeLines language={resolveLanguage(body.tag).name} lines={body.lines} text={text} />;
      break;
  }

  // `assistant-markdown` is what gives the cards their chat styling — spacing,
  // prose, and the highlight token colours are all scoped under it.
  return (
    <div className="assistant-markdown project-file-rich min-h-0 flex-1 overflow-hidden">
      <ArtifactHostActions.Provider value={actions ?? null}>{card}</ArtifactHostActions.Provider>
    </div>
  );
}
