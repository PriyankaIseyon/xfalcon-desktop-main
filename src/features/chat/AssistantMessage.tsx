import { isValidElement, memo, useLayoutEffect, useMemo, useRef, type ReactNode } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import { splitMarkdown, type MarkdownSplit } from "./richContent/splitMarkdown";
import { DOCUMENT_AWARE_REHYPE_PLUGINS, REHYPE_PLUGINS, REMARK_PLUGINS } from "./markdownPipeline";
import { SaveAsReportAction } from "../reports/SaveAsReportAction";
import { CodeArtifact, HtmlArtifact, MarkdownArtifact, SvgArtifact, TableArtifact } from "./richContent/cards";
import { ARTIFACT_ACTION_CLASS } from "./richContent/artifactAction";
import { languageOf, resolveLanguage } from "./richContent/codeLanguages";
import { Link } from "../../ui/Link";
import {
  detectHtmlDocument,
  detectSvgDocument,
  extractHtmlDocument,
  findHtmlDocuments,
  hasExternalReference,
  htmlDocumentIndex,
  parseDelimited,
  parseJsonTable,
  resolveHtmlTitle,
} from "./richContent/detect";

/** The cache a message starts with: nothing parsed, nothing to reuse. */
const EMPTY_SPLIT: MarkdownSplit = { source: "", blocks: [], tailOffset: 0, reusable: false };

/**
 * Places the visual streaming caret at the rendered end of the final text
 * node. Markdown can end in a paragraph, heading, list item, quote, or code
 * block, so a pseudo-element on the top-level block is not a reliable text
 * endpoint. The range selects the final painted character instead of the
 * collapsed insertion point after it. At a soft wrap, Chromium may place that
 * insertion point on the next line even though the character is still on the
 * previous line; the selected character's final client rect is unambiguous.
 */
function positionStreamingCaret(root: HTMLElement, caret: HTMLElement) {
  const document = root.ownerDocument;
  const walker = document.createTreeWalker(root, 4);

  // Start at the answer's end. Walking every settled block on each streamed
  // frame makes caret placement grow with history that has not changed.
  for (let current = walker.lastChild(); current; current = walker.previousNode()) {
    if (
      !(current instanceof Text) ||
      current.data.length === 0 ||
      current.parentElement?.closest(".streaming-text-caret")
    )
      continue;
    const node = current;
    const visibleEnd = node.data.search(/\s*$/u);
    if (visibleEnd === 0) continue;

    // DOM Range offsets count UTF-16 code units. Keep a surrogate pair intact
    // so the measured range always contains one complete final code point.
    let visibleStart = visibleEnd - 1;
    if (visibleStart > 0) {
      const finalUnit = node.data.charCodeAt(visibleStart);
      const previousUnit = node.data.charCodeAt(visibleStart - 1);
      if (finalUnit >= 0xdc00 && finalUnit <= 0xdfff && previousUnit >= 0xd800 && previousUnit <= 0xdbff) {
        visibleStart -= 1;
      }
    }

    const range = document.createRange();
    range.setStart(node, visibleStart);
    range.setEnd(node, visibleEnd);
    const rects = typeof range.getClientRects === "function" ? Array.from(range.getClientRects()) : [];
    const endpoint =
      rects.at(-1) ?? (typeof range.getBoundingClientRect === "function" ? range.getBoundingClientRect() : null);
    if (endpoint === null || endpoint.height === 0) continue;

    const rootRect = root.getBoundingClientRect();
    caret.style.setProperty("--xf-stream-caret-x", `${endpoint.right - rootRect.left}px`);
    caret.style.setProperty("--xf-stream-caret-y", `${endpoint.top - rootRect.top}px`);
    caret.style.setProperty("--xf-stream-caret-height", `${endpoint.height}px`);
    caret.dataset["positioned"] = "true";
    return;
  }

  delete caret.dataset["positioned"];
}

/** Fence tags that render as a data table, and the delimiter each one means. */
const DELIMITED_FENCES: Record<string, string> = { csv: ",", tsv: "\t", tab: "\t" };

/** Recursive React-children → text, for reading a fence's raw source out of
 * the (possibly highlighter-wrapped) code element (web's childrenToText). */
function childrenToText(children: ReactNode): string {
  if (children === null || children === undefined || typeof children === "boolean") return "";
  if (typeof children === "string" || typeof children === "number") return String(children);
  if (Array.isArray(children)) return children.map(childrenToText).join("");
  if (isValidElement<{ children?: ReactNode }>(children)) return childrenToText(children.props.children);
  return "";
}

function isArtifactCode(className: string | undefined): boolean {
  return className?.includes("language-") === true || className?.includes("hljs") === true;
}

/**
 * Type-aware fence rendering (spec: chat-rich-content), the web
 * `MarkdownRenderer` ladder scoped to the desktop runtime: HTML documents →
 * sandboxed preview, SVG → sandboxed image preview, `md`/`markdown` →
 * nested prose, JSON arrays / CSV / TSV → data tables, everything else → the
 * code card under the language's own name (`codeLanguages.ts`). Detection
 * failures fall through to the code card — malformed content never renders
 * blank.
 *
 * `depth` guards the one level of nesting the spec allows: fences inside a
 * rendered markdown fence always land on the code card, never a further
 * preview/table/prose rung.
 */
/**
 * A message whose HTML documents can be saved as reports: the conversation it
 * belongs to, its durable id, and the documents themselves in the order the
 * cards draw them.
 *
 * `documents` is scanned from the message's own raw text once per message, and a
 * card resolves its ordinal by looking itself up in that list. Not a counter
 * incremented as fence components mount — the markdown pipeline may memoize and
 * re-render them in an order unrelated to the document, so a mount counter
 * would be right only by luck. Not a content match against stored bytes either;
 * that is the approach xfalcon-web tried and abandoned.
 */
type SaveableMessage = {
  conversationId: string;
  messageId: string;
  documents: string[];
};

function markdownComponents(streaming: boolean, depth: number, saveable: SaveableMessage | null): Components {
  return {
    // Labelled blocks get card chrome below, so their default <pre> would
    // double-box them. Unlabelled fences and indented blocks do not enter that
    // ladder; keep their semantic pre/code pair so whitespace stays exact.
    pre({ children }) {
      const className = isValidElement<{ className?: string }>(children) ? children.props.className : undefined;
      if (isArtifactCode(className)) return <>{children}</>;
      return (
        <pre>
          <code>{childrenToText(children)}</code>
        </pre>
      );
    },
    /**
     * Prose links go through the product anchor, and until they did they did
     * NOTHING. `react-markdown` renders a bare `<a href>`; a bare anchor is a
     * same-frame navigation; the navigation boundary refuses every origin but
     * the app's own; and `tauri_plugin_opener`'s click handler only forwards
     * an anchor whose target is `_blank`. So a citation in an answer swallowed
     * the click and reported nothing. `Link` sets the target for exactly the
     * schemes the opener forwards, which is what makes the click reach the
     * browser.
     */
    a({ href, children }) {
      return <Link href={href}>{children}</Link>;
    },
    // Keep the semantic table intact and give overflow to a labelled wrapper.
    // Styling the table itself as `display: block` creates an anonymous table
    // box and makes assistive and visual column behavior diverge.
    table({ children }) {
      return (
        <div aria-label="Scrollable table" className="assistant-markdown__table-scroll" role="region" tabIndex={0}>
          <table>{children}</table>
        </div>
      );
    },
    // The app CSP refuses remote image loads. Lazy decoding still matters for
    // allowed local/data images, especially in long stored transcripts.
    img({ src, alt }) {
      return <img alt={alt ?? ""} decoding="async" loading="lazy" referrerPolicy="no-referrer" src={src} />;
    },
    // Named props only, never a spread: `react-markdown` hands each component
    // the hast `node` alongside the element's own props, and forwarding it to
    // the DOM renders `node="[object Object]"` on the element -- which every
    // inline code span in every answer was carrying. A markdown link title is
    // dropped with it, deliberately: a native tooltip is not this app's
    // hover-help grammar (spec: the tooltip contract).
    code({ className, children }) {
      if (!isArtifactCode(className)) {
        return <code className="assistant-markdown__inline-code">{children}</code>;
      }
      const language = languageOf(className);
      const code = childrenToText(children).replace(/\n$/, "");
      const highlighted = <code className={className}>{children}</code>;

      if (depth === 0) {
        // Each rung hands its type the artifact and whether it is still
        // arriving; the type produces both its building state and its settled
        // one, so the two cannot label the same artifact differently.
        if (detectHtmlDocument(language, code)) {
          const html = extractHtmlDocument(code) ?? code;
          const ordinal = saveable === null ? -1 : htmlDocumentIndex(saveable.documents, html);
          return (
            <HtmlArtifact
              building={streaming}
              html={html}
              saveAction={
                saveable === null || ordinal === -1 ? undefined : (
                  <SaveAsReportAction
                    className={ARTIFACT_ACTION_CLASS}
                    defaultTitle={resolveHtmlTitle(html) ?? "Dashboard"}
                    externalReference={hasExternalReference(html)}
                    source={{
                      conversationId: saveable.conversationId,
                      messageId: saveable.messageId,
                      htmlIndex: ordinal,
                    }}
                  />
                )
              }
            />
          );
        }
        if (detectSvgDocument(language, code)) {
          return <SvgArtifact building={streaming} svg={code.trim()} />;
        }
        if (language === "md" || language === "markdown") {
          // Prose flows progressively while streaming — no placeholder.
          return (
            <MarkdownArtifact source={code} sourceView={highlighted}>
              <NestedMarkdown text={code} />
            </MarkdownArtifact>
          );
        }
        if (language === "json" && code.trimStart().startsWith("[")) {
          if (streaming) return <TableArtifact building title="Query results" />;
          const table = parseJsonTable(code);
          if (table)
            return <TableArtifact source={code} sourceView={highlighted} table={table} title="Query results" />;
        }
        const delimiter = DELIMITED_FENCES[language];
        if (delimiter !== undefined) {
          if (streaming) return <TableArtifact building title="Data" />;
          const table = parseDelimited(code, delimiter);
          if (table) return <TableArtifact source={code} sourceView={highlighted} table={table} title="Data" />;
        }
      }
      return (
        <CodeArtifact code={code} language={resolveLanguage(language).name}>
          {highlighted}
        </CodeArtifact>
      );
    },
  };
}

/** A `md`/`markdown` fence rendered as prose through the SAME sanitize →
 * highlight pipeline as the parent message, one level deep (spec:
 * chat-rich-content). Nested fences never reach the preview rung, so no message
 * identity is threaded in.
 *
 * It keeps `REHYPE_PLUGINS`, not the document-aware list: a document fence
 * nested here is a code card that shows its highlighted source, so it must be
 * highlighted. */
function NestedMarkdown({ text }: { text: string }) {
  const components = useMemo(() => markdownComponents(false, 1, null), []);
  return (
    <ReactMarkdown components={components} rehypePlugins={REHYPE_PLUGINS} remarkPlugins={REMARK_PLUGINS}>
      {text}
    </ReactMarkdown>
  );
}

/**
 * Whether the backend can resolve this message id to a stored row.
 *
 * `useChat` synthesizes the in-flight turn with a `stream-` id and an optimistic
 * user turn with a `local-` id. Those are how the renderer tells a durable row
 * from one that has not landed, and offering Save to project on one would produce
 * a save that can only answer "not found" — which reads as a lost report rather
 * than an unsaved answer (spec: reports).
 */
function isDurableMessageId(messageId: string): boolean {
  return !messageId.startsWith("stream-") && !messageId.startsWith("local-");
}

export type AssistantMessageProps = {
  text: string;
  /** The row is the live stream tail — preview/table fences render building
   * placeholders until the closing fence lands (spec: chat-rich-content). */
  streaming?: boolean;
  /** This turn's durable id, and the conversation it belongs to. Both are
   * required before an HTML document in it can be saved to a project; either
   * missing means the action is absent rather than offered and broken. */
  messageId?: string;
  conversationId?: string | null;
};

/**
 * One top-level markdown block, memoized on its own source slice.
 *
 * A streamed answer only ever changes its LAST block, so every earlier one hits
 * this memo and pays nothing — no re-parse, no re-highlight, no re-render of the
 * card it drew (spec: chat-streaming-performance). `components` is memoized by
 * the parent, so the default shallow comparison is the right one here.
 *
 * Top-level blocks use `DOCUMENT_AWARE_REHYPE_PLUGINS`: an HTML or SVG document
 * fence is drawn as a preview card and never shows its highlighted source, so it
 * is not tokenised. Only this depth-0 list is document-aware (see `NestedMarkdown`).
 */
const MarkdownBlock = memo(function MarkdownBlock({ source, components }: { source: string; components: Components }) {
  return (
    <ReactMarkdown components={components} rehypePlugins={DOCUMENT_AWARE_REHYPE_PLUGINS} remarkPlugins={REMARK_PLUGINS}>
      {source}
    </ReactMarkdown>
  );
});

export const AssistantMessage = memo(function AssistantMessage({
  text,
  streaming = false,
  messageId,
  conversationId,
}: AssistantMessageProps) {
  const markdownRef = useRef<HTMLDivElement | null>(null);
  const caretRef = useRef<HTMLSpanElement | null>(null);
  const saveable = useMemo<SaveableMessage | null>(() => {
    if (streaming) return null;
    if (messageId === undefined || conversationId == null) return null;
    if (!isDurableMessageId(messageId)) return null;
    const documents = findHtmlDocuments(text);
    if (documents.length === 0) return null;
    return { conversationId, messageId, documents };
  }, [conversationId, messageId, streaming, text]);
  // Two component sets, because "still arriving" is a per-BLOCK fact, not a
  // per-message one. A streamed answer is append-only, so only the last block
  // can still change; every block before it is final the moment the next one
  // opens. Handing the settled ones `streaming: false` is what lets a completed
  // fence render its table or its preview immediately, instead of holding a
  // "building" placeholder until the whole answer lands (spec:
  // chat-rich-content, chat-streaming-performance).
  const settledComponents = useMemo(() => markdownComponents(false, 0, saveable), [saveable]);
  const arrivingComponents = useMemo(() => markdownComponents(true, 0, saveable), [saveable]);
  // The split carries its own previous result forward, so a streamed answer
  // re-parses only its growing tail. Parsing the whole message per token is
  // quadratic over a stream and measurably the dominant per-frame cost by about
  // 25 KB (see `splitMarkdown`).
  //
  // A ref rather than a memo because the derivation depends on its own last
  // result, not on `text` alone. It stays safe under a re-render React throws
  // away: `splitMarkdown` re-parses whole whenever the new source is not an
  // extension of the cached one, so no cache state can produce a wrong split.
  const splitRef = useRef<MarkdownSplit>(EMPTY_SPLIT);
  if (splitRef.current.source !== text) {
    splitRef.current = splitMarkdown(text, splitRef.current);
  }

  useLayoutEffect(() => {
    const root = markdownRef.current;
    const caret = caretRef.current;
    if (!streaming || !root || !caret) return;
    positionStreamingCaret(root, caret);
  }, [streaming, text]);

  useLayoutEffect(() => {
    const root = markdownRef.current;
    const caret = caretRef.current;
    if (!streaming || !root || !caret || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => positionStreamingCaret(root, caret));
    observer.observe(root);
    return () => observer.disconnect();
  }, [streaming]);

  const blocks = splitRef.current.blocks;
  const lastIndex = blocks.length - 1;
  return (
    // Size comes from the reading column (`--xf-type-reading`, web parity) —
    // a role class here would pin the prose one step smaller than the bubbles
    // and composer around it.
    <div ref={markdownRef} className="assistant-markdown">
      {blocks.map((source, index) => (
        // The index is the key because the split is positional and append-only:
        // block N is the same block from one token to the next, and keying by
        // content instead would remount every block whose text merely grew.
        <MarkdownBlock
          components={streaming && index === lastIndex ? arrivingComponents : settledComponents}
          key={index}
          source={source}
        />
      ))}
      {streaming ? <span ref={caretRef} aria-hidden className="streaming-text-caret" /> : null}
    </div>
  );
});
