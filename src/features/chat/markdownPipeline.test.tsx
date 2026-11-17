import { cleanup, render, screen } from "@testing-library/react";
import ReactMarkdown from "react-markdown";
import { afterEach, describe, expect, it, vi } from "vitest";

const highlightInstrumentation = vi.hoisted(() => ({ attach: vi.fn() }));
/** Which artifact card the real `AssistantMessage` drew, recorded by a
 * pass-through wrapper around the real card. */
const drawn = vi.hoisted(() => ({ kinds: [] as string[] }));

vi.mock("./richContent/cards", async (original) => {
  const actual = await original<typeof import("./richContent/cards")>();
  return {
    ...actual,
    HtmlArtifact(props: Parameters<typeof actual.HtmlArtifact>[0]) {
      drawn.kinds.push("html");
      return <actual.HtmlArtifact {...props} />;
    },
    SvgArtifact(props: Parameters<typeof actual.SvgArtifact>[0]) {
      drawn.kinds.push("svg");
      return <actual.SvgArtifact {...props} />;
    },
  };
});

vi.mock("rehype-highlight", async (original) => {
  const actual = await original<typeof import("rehype-highlight")>();
  return {
    ...actual,
    default(options: Parameters<typeof actual.default>[0]) {
      highlightInstrumentation.attach(options);
      return actual.default(options);
    },
  };
});

vi.mock("../../lib/native", () => ({
  previewDocumentRegister: vi.fn(async () => "beef".repeat(16)),
  previewDocumentSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

import { AssistantMessage } from "./AssistantMessage";
import { DOCUMENT_AWARE_REHYPE_PLUGINS, REMARK_PLUGINS } from "./markdownPipeline";

afterEach(cleanup);

describe("markdown highlighting pipeline", () => {
  it("reuses one real language registry across streamed ReactMarkdown processors", () => {
    const first = [
      "<script data-stream-attack>window.__streamAttack = true</script>",
      "",
      "```snowflake",
      "select count(*) from sales where region = 'EMEA'",
      "```",
      "",
      "The result",
    ].join("\n");

    const { container, rerender } = render(<AssistantMessage streaming text={first} />);
    expect(container.querySelector("script[data-stream-attack]")).toBeNull();
    expect(screen.getByText("Snowflake SQL")).toBeInTheDocument();
    expect(container.querySelectorAll(".rich-card__code [class*='hljs-']").length).toBeGreaterThan(0);
    expect(highlightInstrumentation.attach).toHaveBeenCalledTimes(1);

    rerender(<AssistantMessage streaming text={`${first} is ready.`} />);
    rerender(<AssistantMessage streaming text={`${first} is ready.\n\n\`\`\`zzql\nfrom sales | count\n\`\`\``} />);

    const unknownSource = screen.getByLabelText("zzql source");
    expect(unknownSource).toHaveTextContent("from sales | count");
    expect(unknownSource.querySelector("[class*='hljs-']")).toBeNull();
    expect(container.querySelector("script[data-stream-attack]")).toBeNull();
    expect(highlightInstrumentation.attach).toHaveBeenCalledTimes(1);
  });
});

describe("document-aware highlighting (spec: chat-streaming-performance)", () => {
  const doc =
    "<!DOCTYPE html>\n<html><head><title>Board</title></head><body>" + "<p>row</p>".repeat(8) + "</body></html>";
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><circle r="2"/></svg>';

  /** Each fence is a case the marker and the component must agree on. The
   * boundary cases are the point: a comment before the doctype, a document
   * under a non-html tag, and an svg whose document never closes. */
  const FENCES: Array<{ name: string; text: string }> = [
    { name: "html document with leading blank lines", text: `\`\`\`html\n\n\n${doc}\n\`\`\`` },
    {
      name: "html with a comment before the doctype (not a document)",
      text: `\`\`\`html\n<!-- generated -->\n${doc}\n\`\`\``,
    },
    { name: "<html> without a doctype", text: `\`\`\`html\n<html><body>${"<p>x</p>".repeat(20)}</html>\n\`\`\`` },
    { name: "htm language", text: `\`\`\`htm\n${doc}\n\`\`\`` },
    { name: "xml language", text: `\`\`\`xml\n${doc}\n\`\`\`` },
    { name: "unlabelled fence containing an html document (never a card)", text: `\`\`\`\n${doc}\n\`\`\`` },
    { name: "svg with an xml prolog", text: `\`\`\`svg\n<?xml version="1.0"?>\n${svg}\n\`\`\`` },
    {
      name: "svg without a closing tag",
      text: `\`\`\`svg\n<svg xmlns="http://www.w3.org/2000/svg"><circle r="2"/>\n\`\`\``,
    },
    { name: "html fragment", text: '```html\n<div class="x">hi</div>\n```' },
    { name: "plain javascript", text: "```js\nconst total = 1;\n```" },
    { name: "sql", text: "```sql\nSELECT 1;\n```" },
    { name: "json array", text: '```json\n[{"a":1}]\n```' },
    { name: "csv", text: "```csv\nname,age\nJo,40\n```" },
  ];

  /** The fences that ARE documents, by name, in FENCES order. Pins the detectors'
   * current behaviour: `detectHtmlDocument` and `detectSvgDocument`, read through
   * the `language-` class gate that the plugin and `AssistantMessage` both apply.
   * A detector change that moves a fence in or out of this list is a behaviour
   * change and has to be made here on purpose. */
  const expectedDocuments = [
    "html document with leading blank lines",
    "<html> without a doctype",
    "htm language",
    "xml language",
    "svg with an xml prolog",
  ];

  /** Whether the real list marks this fence's `<code>` for the highlighter to skip. */
  function markedAsDocument(text: string): boolean {
    let className = "";
    const { unmount } = render(
      <ReactMarkdown
        components={{
          code({ className: name }) {
            className = name ?? "";
            return null;
          },
        }}
        rehypePlugins={DOCUMENT_AWARE_REHYPE_PLUGINS}
        remarkPlugins={REMARK_PLUGINS}
      >
        {text}
      </ReactMarkdown>,
    );
    unmount();
    return className.includes("no-highlight");
  }

  /** Whether the real `AssistantMessage` drew this fence as an HTML or SVG document card. */
  function drawnAsDocument(text: string): boolean {
    drawn.kinds = [];
    const { unmount } = render(<AssistantMessage text={text} />);
    unmount();
    return drawn.kinds.length > 0;
  }

  it("marks exactly the fences the component draws as a document card", () => {
    const marked = FENCES.filter((fence) => markedAsDocument(fence.text)).map((fence) => fence.name);
    const documentCards = FENCES.filter((fence) => drawnAsDocument(fence.text)).map((fence) => fence.name);
    expect(documentCards.length).toBeGreaterThan(0);
    expect(documentCards.length).toBeLessThan(FENCES.length);
    expect(marked).toEqual(documentCards);
    expect(marked).toEqual(expectedDocuments);
    expect(documentCards).toEqual(expectedDocuments);
  });
});
