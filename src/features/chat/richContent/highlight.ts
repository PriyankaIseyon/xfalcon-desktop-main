import { createElement, type ReactNode } from "react";
import { createLowlight } from "lowlight";
import { HIGHLIGHT_ALIASES, HIGHLIGHT_LANGUAGES } from "./codeLanguages";

/**
 * Highlighting for one whole text outside the markdown pipeline — a project
 * file shown in the file view (spec: project-file-editing).
 *
 * The transcript highlights inside `rehype-highlight`, which only sees fences.
 * A file is not a fence, and wrapping one in a synthetic fence to borrow that
 * path would re-parse a megabyte of text as markdown to reach the same grammar.
 * So this reads the SAME registry — `HIGHLIGHT_LANGUAGES` and
 * `HIGHLIGHT_ALIASES` from `codeLanguages.ts` — and a tag highlights here
 * exactly as it does in a fence, under the same name.
 *
 * The output is React text nodes inside `span`s carrying highlight.js classes.
 * Nothing is parsed as markup, so the file's bytes cannot inject anything.
 */

type Lowlight = ReturnType<typeof createLowlight>;
type HighlightTree = ReturnType<Lowlight["highlight"]>;
type HighlightNode = HighlightTree["children"][number];

let registry: Lowlight | null = null;

/** Built on first use: the file view is rarely opened, and the transcript's
 * own registry is a different instance inside `rehype-highlight`. */
function lowlight(): Lowlight {
  if (registry === null) {
    registry = createLowlight(HIGHLIGHT_LANGUAGES);
    registry.registerAlias(HIGHLIGHT_ALIASES);
  }
  return registry;
}

/** One run of text in one highlight scope. */
export type HighlightToken = { text: string; className: string | null };

/**
 * `code` highlighted with the grammar `tag` names, one token list per line, or
 * `null` when the registry has no grammar for it. Never auto-detects: a guessed
 * grammar highlights the wrong tokens with total confidence
 * (`codeLanguages.ts`).
 *
 * Per line so the view can virtualize: only the visible lines become DOM. The
 * whole text is highlighted in one pass first, so a string or comment spanning
 * lines keeps its scope on every one of them.
 */
export function highlightLines(tag: string, code: string): HighlightToken[][] | null {
  const highlighter = lowlight();
  if (!highlighter.registered(tag)) return null;
  const lines: HighlightToken[][] = [[]];
  const walk = (node: HighlightNode, scopes: string[]) => {
    if (node.type === "text") {
      const className = scopes.length === 0 ? null : scopes.join(" ");
      const parts = node.value.split("\n");
      parts.forEach((part, index) => {
        if (index > 0) lines.push([]);
        if (part !== "") lines[lines.length - 1]!.push({ text: part, className });
      });
      return;
    }
    if (node.type !== "element") return;
    const own = node.properties?.className;
    const next = Array.isArray(own) ? [...scopes, ...own.map(String)] : scopes;
    for (const child of node.children) walk(child, next);
  };
  for (const node of highlighter.highlight(tag, code).children) walk(node, []);
  // A CRLF file's `\r` belongs to the line ending, not the line. It can sit in
  // a different scope than the `\n` after it — a line comment ends before the
  // newline, not before the carriage return — so it is trimmed per line here.
  for (const line of lines) {
    const last = line[line.length - 1];
    if (last === undefined || !last.text.endsWith("\r")) continue;
    if (last.text.length === 1) line.pop();
    else line[line.length - 1] = { text: last.text.slice(0, -1), className: last.className };
  }
  return lines;
}

/** One highlighted line as React nodes. */
export function renderHighlightedLine(tokens: HighlightToken[]): ReactNode[] {
  return tokens.map((token, index) =>
    token.className === null
      ? token.text
      : createElement("span", { key: index, className: token.className }, token.text),
  );
}
