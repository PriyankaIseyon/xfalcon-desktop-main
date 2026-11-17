import type { PluggableList } from "unified";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import rehypeSanitize, { defaultSchema } from "rehype-sanitize";
import { HIGHLIGHT_ALIASES, HIGHLIGHT_LANGUAGES, languageOf } from "./richContent/codeLanguages";
import { detectHtmlDocument, detectSvgDocument } from "./richContent/detect";

/**
 * The one markdown pipeline every surface that renders model prose runs.
 *
 * Extracted from `AssistantMessage` when the export document became a second
 * consumer (spec: lean-report-export). It has to be one list rather than two
 * matching ones: the sanitize schema is a security boundary, and a copy of it
 * elsewhere in the tree is a copy that can be relaxed on its own. A printed
 * document renders the same untrusted model output as the chat transcript, so
 * it gets the same treatment or the boundary has a hole in the shape of the
 * newest feature.
 */

/**
 * Sanitize runs FIRST (untrusted model output), highlight second — so the
 * `<span class="hljs-…">` tokens the highlighter adds survive sanitization.
 * The schema only needs to keep the fence's `language-*` class on `<code>`
 * for the highlighter to pick the right grammar; everything else stays the
 * default GitHub schema.
 */
export const SANITIZE_SCHEMA = {
  ...defaultSchema,
  attributes: {
    ...defaultSchema.attributes,
    code: [["className", /^language-./, "hljs"]],
  },
} satisfies typeof defaultSchema;

/**
 * `rehype-highlight` builds and populates its lowlight registry when its plugin
 * attacher runs. ReactMarkdown creates a fresh processor for each changed
 * block, so putting the configured plugin tuple in a module-level array alone
 * still rebuilt that registry on every streamed update. The returned
 * transformer has no per-document state; it only reads the configured registry
 * while transforming the tree it receives, so all processors can share it.
 */
const highlight = rehypeHighlight({
  aliases: HIGHLIGHT_ALIASES,
  detect: false,
  languages: HIGHLIGHT_LANGUAGES,
});

function rehypeConfiguredHighlight() {
  return highlight;
}

/**
 * One plugin list, shared by the message pipeline, the nested-markdown one, and
 * the export document, so a fence inside a markdown fence highlights
 * identically to one outside it — and to one on a printed page.
 *
 * The grammar set and the alias map come from `codeLanguages.ts` rather than
 * `rehype-highlight`'s default `common`: that default carries `sql` with no
 * aliases and nothing else this product answers in, so every dialect tag a model
 * writes — `snowflake`, `sparksql`, `tsql` — used to turn highlighting off.
 *
 * `detect: false` is the default and is stated anyway, because it is a decision:
 * an untagged fence gets no grammar guessed for it. A misdetected grammar
 * highlights the wrong tokens with complete confidence, which a reader has more
 * trouble disbelieving than plain text.
 *
 * The stable attacher below returns the module-level transformer, so
 * `createLowlight` registers every grammar once rather than once for every
 * ReactMarkdown processor created during a stream.
 */
export const REHYPE_PLUGINS: PluggableList = [[rehypeSanitize, SANITIZE_SCHEMA], rehypeConfiguredHighlight];

/**
 * The minimal hast shape this file reads. Declared here rather than imported
 * from `hast`, which is not a direct dependency of this package.
 */
type HastNode = {
  type: string;
  tagName?: string;
  value?: string;
  properties?: { className?: unknown };
  children?: HastNode[];
};

/** rehype-highlight leaves a `<code>` alone when its class list carries this. */
const NO_HIGHLIGHT = "no-highlight";

function textOf(node: HastNode): string {
  if (node.type === "text") return node.value ?? "";
  return (node.children ?? []).map(textOf).join("");
}

/**
 * Marks a fence that renders as an HTML or SVG document card. The predicate and
 * the component's branch are the same two detect helpers, read from the same
 * text (a fence's source, less one trailing newline, as the component reads it),
 * so the mark and the card cannot disagree.
 */
function markDocumentFence(code: HastNode): void {
  const properties = code.properties;
  const className = properties?.className;
  if (properties === undefined || !Array.isArray(className)) return;
  const classes = className.map(String);
  const joined = classes.join(" ");
  if (!joined.includes("language-")) return;
  const language = languageOf(joined);
  const source = textOf(code).replace(/\n$/, "");
  if (detectHtmlDocument(language, source) || detectSvgDocument(language, source)) {
    properties.className = [...classes, NO_HIGHLIGHT];
  }
}

function markDocumentFences(node: HastNode, parentTagName?: string): void {
  if (node.tagName === "code" && parentTagName === "pre") markDocumentFence(node);
  for (const child of node.children ?? []) markDocumentFences(child, node.tagName);
}

/**
 * Runs after sanitize and before highlight, so a document fence carries
 * `no-highlight` into the highlighter. The class is added after sanitize on
 * purpose: the schema would strip it.
 */
function rehypeSkipDocumentHighlight() {
  return (tree: HastNode) => {
    markDocumentFences(tree);
  };
}

/**
 * The depth-0 message pipeline. An HTML or SVG document fence is drawn as a
 * preview card and its highlighted source is never rendered, so tokenising it
 * was per-delta work thrown away: a 60 KB dashboard produced about 13,500
 * tokens on every frame of its stream. Those fences skip the highlighter here.
 *
 * Only the depth-0 list is marked. A fence nested in a markdown fence (depth 1)
 * is still a code card that renders its highlighted children, so it keeps
 * `REHYPE_PLUGINS`.
 */
export const DOCUMENT_AWARE_REHYPE_PLUGINS: PluggableList = [
  [rehypeSanitize, SANITIZE_SCHEMA],
  rehypeSkipDocumentHighlight,
  rehypeConfiguredHighlight,
];

/** GitHub-flavoured markdown: tables, strikethrough, task lists, autolinks. */
export const REMARK_PLUGINS: PluggableList = [remarkGfm];
