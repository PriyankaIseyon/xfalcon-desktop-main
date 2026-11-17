import React from "react";
import { cleanup, render, screen } from "@testing-library/react";
import ReactMarkdown from "react-markdown";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantMessage } from "../../src/features/chat/AssistantMessage";
import { REHYPE_PLUGINS, REMARK_PLUGINS } from "../../src/features/chat/markdownPipeline";

const highlighting = vi.hoisted(() => ({ fences: 0, spans: 0 }));

/** The real highlighter, with its output counted on the hast tree it returns.
 * A fence it tokenised carries the `hljs` class on its `<code>`, and each token
 * is a `span` with an `hljs-` class. Counting the tree rather than the DOM is
 * what makes this visible: the artifact branch never renders the highlighted
 * children, so the DOM shows nothing whether or not the highlighter ran. */
type HastNode = { type?: string; tagName?: string; properties?: { className?: unknown }; children?: HastNode[] };

function tally(node: HastNode): void {
  const classes = Array.isArray(node.properties?.className) ? node.properties.className.map(String) : [];
  if (node.tagName === "code" && classes.includes("hljs")) highlighting.fences += 1;
  if (node.tagName === "span" && classes.some((name) => name.startsWith("hljs-"))) highlighting.spans += 1;
  for (const child of node.children ?? []) tally(child);
}

vi.mock("rehype-highlight", async (original) => {
  const actual = await original<typeof import("rehype-highlight")>();
  return {
    ...actual,
    default(options: Parameters<typeof actual.default>[0]) {
      const transform = actual.default(options);
      return (tree: Parameters<typeof transform>[0], file: Parameters<typeof transform>[1]) => {
        transform(tree, file);
        tally(tree as HastNode);
      };
    },
  };
});

vi.mock("../../src/lib/native", () => ({
  previewDocumentRegister: vi.fn(async () => "beef".repeat(16)),
  previewDocumentSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

/**
 * Structural budget for a document fence (spec: chat-streaming-performance).
 *
 * A full HTML document in an answer is drawn as a preview card, and its source
 * is never shown highlighted. Before `DOCUMENT_AWARE_REHYPE_PLUGINS`, the
 * highlighter tokenised the whole fence on every streamed delta anyway, and the
 * tokens were thrown away. The gate is the count of fences and tokens the
 * highlighter produces for the document: zero on the chat path. The bare
 * pipeline row renders the same fence through the unchanged `REHYPE_PLUGINS`
 * and must highlight it, which proves the counter sees highlighting when it
 * happens.
 *
 * The counts are structural, so they do not depend on the machine.
 */

/** A dashboard-sized document, built the way a model's answer is: one HTML
 * document with a head, a style block and a long table body. */
function buildDashboard(bytes: number): string {
  const head =
    '<!DOCTYPE html>\n<html><head><meta charset="utf-8"><title>Weekly Sales Dashboard</title>' +
    "<style>body{font-family:sans-serif}td{padding:4px}</style></head><body>\n<h1>Weekly sales</h1>\n<table>\n";
  const rows: string[] = [];
  let length = head.length;
  for (let i = 0; length < bytes; i += 1) {
    const row = `<tr class="row"><td>Region ${i}</td><td data-value="${i * 37}">${i * 1.5}</td></tr>\n`;
    rows.push(row);
    length += row.length;
  }
  return `${head}${rows.join("")}</table>\n</body></html>\n`;
}

const DOCUMENT = buildDashboard(60 * 1024);
const DOCUMENT_TEXT = `Here is the dashboard.\n\n\`\`\`html\n${DOCUMENT}\`\`\`\n`;

beforeEach(() => {
  highlighting.fences = 0;
  highlighting.spans = 0;
});

afterEach(cleanup);
afterEach(() => vi.restoreAllMocks());

describe("document fence highlighting budget (spec: chat-streaming-performance)", () => {
  it("bare pipeline highlights the document fence", () => {
    render(
      <ReactMarkdown rehypePlugins={REHYPE_PLUGINS} remarkPlugins={REMARK_PLUGINS}>
        {DOCUMENT_TEXT}
      </ReactMarkdown>,
    );
    expect(highlighting.fences).toBeGreaterThan(0);
    expect(highlighting.spans).toBeGreaterThan(0);
  }, 120_000);

  it("chat path renders the document as a preview card without highlighting it", async () => {
    render(<AssistantMessage text={DOCUMENT_TEXT} />);
    await screen.findByTitle("Weekly Sales Dashboard");
    expect({ fences: highlighting.fences, spans: highlighting.spans }).toEqual({ fences: 0, spans: 0 });
  }, 120_000);
});
