import { fromMarkdown } from "mdast-util-from-markdown";
import { gfmFromMarkdown } from "mdast-util-gfm";
import { gfm } from "micromark-extension-gfm";

/**
 * Splits model prose into its top-level markdown blocks.
 *
 * A streamed answer is append-only, so every block except the last one has a
 * source slice that is byte-identical from one token to the next. That is what
 * makes per-block memoization work: only the final, still-growing block re-parses
 * and re-renders, and a settled fence stops paying for text that arrives after
 * it (spec: chat-streaming-performance, "Settled Prose Renders Once").
 *
 * The boundaries come from mdast rather than from a blank-line scan because
 * `react-markdown` is built on the same parser: a hand-rolled splitter cuts a
 * fenced block that contains a blank line, and turns one list into two. Matching
 * the renderer's own parse is the only way the split is guaranteed invisible.
 *
 * Inter-block whitespace — the blank lines between two blocks — belongs to no
 * node's span and is dropped. That is safe because every block-level element
 * carries its own margins, so rendering the slices independently lays out the
 * same as rendering the whole string.
 *
 * This is the desktop reading of LibreChat's `splitMarkdown.ts`
 * (`references/LibreChat`). Their version also returns per-block counts of the
 * code and artifact indices each block consumes, because their code blocks are
 * numbered by mount order. This one does not need them: `AssistantMessage`
 * resolves an HTML document's ordinal by looking the content up in the message's
 * own document list (`htmlDocumentIndex`), which does not care which block drew
 * it.
 *
 * # Why there is an incremental path
 *
 * Re-parsing the whole message per token is quadratic over a stream, and
 * measurably so: 3.7 KB in 80 steps costs 261 ms of mdast parsing, 12.6 KB in
 * 260 steps costs 1090 ms, and 25.4 KB in 520 steps costs 4521 ms — doubling the
 * answer and its step count quadruples the bill, and by 25 KB the parse is the
 * dominant per-frame cost of a streaming answer. [`splitMarkdown`] reuses the
 * settled prefix and re-parses only the tail, which makes the whole stream linear
 * in the answer's length.
 */

/** Only the fields this module reads. mdast nodes carry far more. */
type MdastNode = {
  type: string;
  children?: MdastNode[];
  position?: { start?: { offset?: number }; end?: { offset?: number } };
};

/**
 * A completed split, and what the next one needs to reuse it.
 *
 * `tailOffset` is where the last block begins — the ONLY part of the source a
 * further append can change, which is the premise the incremental path rests on.
 * `reusable` is false when this parse cannot serve as a prefix for the next one,
 * for either of the reasons below.
 */
export type MarkdownSplit = {
  source: string;
  blocks: string[];
  tailOffset: number;
  reusable: boolean;
};

const EMPTY_SPLIT: MarkdownSplit = { source: "", blocks: [], tailOffset: 0, reusable: false };

/**
 * A construct whose meaning is document-scoped, so severing it from the rest of
 * the message would change what renders.
 *
 * A definition may sit inside a blockquote or a list item, so this descends
 * rather than testing the top level only.
 */
function containsDefinition(node: MdastNode): boolean {
  if (node.type === "definition" || node.type === "footnoteDefinition") {
    return true;
  }
  return (node.children ?? []).some(containsDefinition);
}

function parseTopLevel(source: string): MdastNode[] {
  const tree = fromMarkdown(source, {
    extensions: [gfm()],
    mdastExtensions: [gfmFromMarkdown()],
  }) as MdastNode;
  return tree.children ?? [];
}

/**
 * Splits `source` on its own, with no prefix reused.
 *
 * Two escapes back out to one whole-message block, because rendering the slices
 * independently would not produce the same page:
 *
 * - a raw `html` node: the sanitize schema escapes it to text, and the separator
 *   between two adjacent HTML blocks lives in the gap between their slices,
 *   which slicing drops;
 * - a reference or footnote definition: it is document-scoped, so a reference in
 *   another block would render as literal text once the definition is in a
 *   different render pass.
 */
function splitWhole(source: string): MarkdownSplit {
  const children = parseTopLevel(source);
  if (children.length === 0) {
    return { source, blocks: [source], tailOffset: 0, reusable: false };
  }
  if (children.some((node) => node.type === "html" || containsDefinition(node))) {
    return { source, blocks: [source], tailOffset: 0, reusable: false };
  }

  const blocks: string[] = [];
  let tailOffset = 0;
  for (const node of children) {
    const start = node.position?.start?.offset;
    const end = node.position?.end?.offset;
    // A node without a position cannot be sliced, and guessing its bounds would
    // silently drop or duplicate text. Fall back to the whole message.
    if (start === undefined || end === undefined) {
      return { source, blocks: [source], tailOffset: 0, reusable: false };
    }
    tailOffset = start;
    blocks.push(source.slice(start, end));
  }
  return { source, blocks, tailOffset, reusable: isLineStart(source, tailOffset) };
}

/**
 * Whether an offset sits at the beginning of a line.
 *
 * The guard that makes reuse safe. Parsing `source.slice(tailOffset)` in
 * isolation is equivalent to parsing the whole and keeping the blocks from
 * `tailOffset` on ONLY while the tail begins where a line begins: an indented
 * code block's node can start after its four spaces, and a slice taken there
 * parses as a paragraph rather than as code. Anything else re-parses whole,
 * which is always correct and merely slower.
 */
function isLineStart(source: string, offset: number): boolean {
  return offset === 0 || source[offset - 1] === "\n";
}

/**
 * Splits `source`, reusing `previous` when it is a prefix of it.
 *
 * Only the last block of an append-only source can change, so the settled blocks
 * are carried over verbatim and only `source.slice(previous.tailOffset)` is
 * parsed. Reuse is refused unless every condition holds:
 *
 * - `previous` came from a parse that produced real block boundaries (a
 *   whole-message escape has none to reuse);
 * - `source` actually begins with `previous.source` — a caller that replaces the
 *   text rather than appending to it gets a full parse, which is what makes this
 *   safe to hand any two strings;
 * - the tail begins at a line start (see [`isLineStart`]).
 *
 * The tail parse applies the same escapes, so a definition or a raw HTML block
 * that arrives later still turns the whole message into one block: every byte
 * passes through a tail parse at some point, and the first parse that escapes
 * marks the result unusable as a prefix, so every later one parses whole too.
 */
export function splitMarkdown(source: string, previous?: MarkdownSplit): MarkdownSplit {
  if (!source) return EMPTY_SPLIT;
  if (
    previous === undefined ||
    !previous.reusable ||
    previous.blocks.length === 0 ||
    !source.startsWith(previous.source)
  ) {
    return splitWhole(source);
  }

  const tail = source.slice(previous.tailOffset);
  const children = parseTopLevel(tail);
  if (children.length === 0) return splitWhole(source);
  if (children.some((node) => node.type === "html" || containsDefinition(node))) {
    return splitWhole(source);
  }

  const settled = previous.blocks.slice(0, -1);
  const blocks = settled.slice();
  let tailOffset = previous.tailOffset;
  for (const node of children) {
    const start = node.position?.start?.offset;
    const end = node.position?.end?.offset;
    if (start === undefined || end === undefined) return splitWhole(source);
    tailOffset = previous.tailOffset + start;
    blocks.push(tail.slice(start, end));
  }
  return { source, blocks, tailOffset, reusable: isLineStart(source, tailOffset) };
}

/** The blocks alone, for a caller with one string and no stream behind it. */
export function splitMarkdownIntoBlocks(source: string): string[] {
  return splitMarkdown(source).blocks;
}
