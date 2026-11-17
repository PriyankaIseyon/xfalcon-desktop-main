import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantMessage } from "./AssistantMessage";

/** The real highlighter, pass-through, with what it tokenised counted on the
 * hast tree it returns. The artifact branch never renders highlighted children,
 * so the DOM cannot show whether a document fence was tokenised; the tree can. */
const highlighted = vi.hoisted(() => ({ fences: 0, spans: 0 }));

vi.mock("rehype-highlight", async (original) => {
  const actual = await original<typeof import("rehype-highlight")>();
  type HastNode = { tagName?: string; properties?: { className?: unknown }; children?: HastNode[] };
  function tally(node: HastNode): void {
    const classes = Array.isArray(node.properties?.className) ? node.properties.className.map(String) : [];
    if (node.tagName === "code" && classes.includes("hljs")) highlighted.fences += 1;
    if (node.tagName === "span" && classes.some((name) => name.startsWith("hljs-"))) highlighted.spans += 1;
    for (const child of node.children ?? []) tally(child);
  }
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

vi.mock("../../lib/native", () => ({
  previewDocumentRegister: vi.fn(async () => "beef".repeat(16)),
  previewDocumentSrc: (id: string) => `http://xf-preview.localhost/${id}`,
}));

afterEach(cleanup);
afterEach(() => vi.restoreAllMocks());

const DOC = `<!DOCTYPE html>\n<html><head><title>Sales Report</title></head><body><h1>Hi</h1>${"x".repeat(60)}</body></html>`;

describe("AssistantMessage", () => {
  it("renders markdown without executing raw HTML", () => {
    render(<AssistantMessage text={'# Title\n\n<script>alert("x")</script>\n\n`code`'} />);
    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Title");
    expect(screen.queryByRole("script")).not.toBeInTheDocument();
  });

  it("uses the final painted character rect for the streaming caret", () => {
    const priorLineRect = {
      left: 18,
      right: 46,
      top: 8,
      bottom: 26,
      width: 28,
      height: 18,
      x: 18,
      y: 8,
      toJSON: () => ({}),
    };
    let finalCharacterRect = {
      left: 44,
      right: 52,
      top: 36,
      bottom: 54,
      width: 8,
      height: 18,
      x: 44,
      y: 36,
      toJSON: () => ({}),
    };
    const range = {
      setStart: vi.fn(),
      setEnd: vi.fn(),
      collapse: vi.fn(),
      getClientRects: () => [priorLineRect, finalCharacterRect],
      getBoundingClientRect: () => finalCharacterRect,
    } as unknown as Range;
    vi.spyOn(document, "createRange").mockReturnValue(range);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      left: 10,
      right: 310,
      top: 20,
      bottom: 120,
      width: 300,
      height: 100,
      x: 10,
      y: 20,
      toJSON: () => ({}),
    });

    const { container, rerender } = render(<AssistantMessage streaming text="The first wrapped line" />);
    const caret = container.querySelector<HTMLElement>(".streaming-text-caret");
    expect(range.setStart).toHaveBeenLastCalledWith(expect.any(Text), "The first wrapped line".length - 1);
    expect(range.setEnd).toHaveBeenLastCalledWith(expect.any(Text), "The first wrapped line".length);
    expect(range.collapse).not.toHaveBeenCalled();
    expect(caret?.style.getPropertyValue("--xf-stream-caret-x")).toBe("42px");
    expect(caret?.style.getPropertyValue("--xf-stream-caret-y")).toBe("16px");
    expect(caret?.style.getPropertyValue("--xf-stream-caret-height")).toBe("18px");

    finalCharacterRect = {
      ...finalCharacterRect,
      left: 26,
      right: 34,
      top: 64,
      bottom: 82,
      x: 26,
      y: 64,
    };
    rerender(<AssistantMessage streaming text="The first wrapped line plus more" />);
    expect(caret?.style.getPropertyValue("--xf-stream-caret-x")).toBe("24px");
    expect(caret?.style.getPropertyValue("--xf-stream-caret-y")).toBe("44px");
  });

  it("measures the complete final code point when it is a surrogate pair", () => {
    const endpoint = {
      left: 40,
      right: 52,
      top: 20,
      bottom: 38,
      width: 12,
      height: 18,
      x: 40,
      y: 20,
      toJSON: () => ({}),
    };
    const range = {
      setStart: vi.fn(),
      setEnd: vi.fn(),
      collapse: vi.fn(),
      getClientRects: () => [endpoint],
      getBoundingClientRect: () => endpoint,
    } as unknown as Range;
    vi.spyOn(document, "createRange").mockReturnValue(range);
    vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      left: 10,
      right: 310,
      top: 10,
      bottom: 110,
      width: 300,
      height: 100,
      x: 10,
      y: 10,
      toJSON: () => ({}),
    });

    render(<AssistantMessage streaming text="Done 🚀" />);
    expect(range.setStart).toHaveBeenLastCalledWith(expect.any(Text), "Done 🚀".length - 2);
    expect(range.setEnd).toHaveBeenLastCalledWith(expect.any(Text), "Done 🚀".length);
  });

  it("finds the caret endpoint without inspecting settled paragraphs", () => {
    const endpoint = { right: 20, top: 10, height: 18 } as DOMRect;
    const range = {
      setStart: vi.fn(),
      setEnd: vi.fn(),
      getClientRects: () => [endpoint],
    } as unknown as Range;
    vi.spyOn(document, "createRange").mockReturnValue(range);
    const closest = vi.spyOn(Element.prototype, "closest");
    const history = Array.from({ length: 250 }, (_, index) => `Paragraph ${index}.`).join("\n\n");
    const { rerender } = render(<AssistantMessage streaming text={`${history}\n\nThe final **word**`} />);
    const endpointNode = vi.mocked(range.setEnd).mock.lastCall?.[0] as Text;
    expect(endpointNode.data).toBe("word");
    expect(closest.mock.calls.filter(([selector]) => selector === ".streaming-text-caret")).toHaveLength(1);

    rerender(<AssistantMessage streaming text={`${history}\n\nThe final **word** grows`} />);
    expect((vi.mocked(range.setEnd).mock.lastCall?.[0] as Text).data).toBe(" grows");
    expect(closest.mock.calls.filter(([selector]) => selector === ".streaming-text-caret")).toHaveLength(2);
  });

  /**
   * A prose link used to swallow its click. `react-markdown` renders a bare
   * `<a href>`; a bare anchor is a same-frame navigation; the navigation
   * boundary refuses every origin but the app's own; and the opener plugin
   * only forwards an anchor whose target is `_blank`. Nothing happened, and
   * nothing said so.
   */
  it("gives a prose link the target that makes the click reach the browser", () => {
    render(<AssistantMessage text={"See the [pricing page](https://xfalcon.ai/pricing) for the tiers."} />);
    const link = screen.getByRole("link", { name: "pricing page, opens in the browser" });
    expect(link).toHaveAttribute("href", "https://xfalcon.ai/pricing");
    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
  });

  /** `react-markdown` hands every component the hast node beside the element
   * props, and forwarding it to the DOM stamps `node="[object Object]"` on the
   * element -- which every inline code span in every answer used to carry. */
  it("keeps the hast node out of the rendered markup", () => {
    const { container } = render(
      <AssistantMessage text={"Run `npm run build`, then read the [docs](https://x.test/d)."} />,
    );
    expect(container.innerHTML).not.toContain("node=");
  });

  it("marks inline identifiers for value styling without changing their markdown emphasis", () => {
    render(<AssistantMessage text={"**Dimensions**: **`DIM_CUSTOMER`** and `DIM_PRODUCT`"} />);

    const emphasized = screen.getByText("DIM_CUSTOMER");
    expect(emphasized.tagName).toBe("CODE");
    expect(emphasized).toHaveClass("assistant-markdown__inline-code");
    expect(emphasized.parentElement?.tagName).toBe("STRONG");
    expect(screen.getByText("DIM_PRODUCT")).toHaveClass("assistant-markdown__inline-code");
  });

  it.each([
    ["an unlabelled fence", "```\nfirst  line\n  indented\n```", "first  line\n  indented\n"],
    ["an indented block", "    first  line\n      indented", "first  line\n  indented\n"],
  ])("keeps %s in a whitespace-preserving pre/code pair", (_name, markdown, source) => {
    const { container } = render(<AssistantMessage text={markdown} />);

    const pre = container.querySelector(".assistant-markdown > pre");
    const code = pre?.querySelector(":scope > code");
    expect(code).not.toBeNull();
    expect(code).not.toHaveClass("assistant-markdown__inline-code");
    expect(code?.textContent).toBe(source);
  });

  it("keeps an unlabelled fence inside rendered markdown in a pre/code pair", () => {
    const { container } = render(<AssistantMessage text={"````markdown\n```\nleft  right\n  nested\n```\n````"} />);

    const code = container.querySelector(".rich-card__prose pre > code");
    expect(code).not.toBeNull();
    expect(code).not.toHaveClass("assistant-markdown__inline-code");
    expect(code?.textContent).toBe("left  right\n  nested\n");
  });

  it("leaves an in-app prose link a plain navigation", () => {
    render(<AssistantMessage text={"Jump to [the summary](#summary)."} />);
    const link = screen.getByRole("link", { name: "the summary" });
    expect(link).not.toHaveAttribute("target");
  });

  it("renders an html fence as a sandboxed preview card on the preview origin", async () => {
    render(<AssistantMessage text={`Report below:\n\n\`\`\`html\n${DOC}\n\`\`\``} />);
    const frame = await screen.findByTitle("Sales Report");
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
    expect(frame.getAttribute("src")).toMatch(/^http:\/\/xf-preview\.localhost\//);
  });

  it("renders an svg fence as a sandboxed image preview", async () => {
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><circle r="2"/></svg>';
    render(<AssistantMessage text={`\`\`\`svg\n${svg}\n\`\`\``} />);
    const frame = await screen.findByTitle("SVG image");
    expect(frame.getAttribute("sandbox")).toBe("allow-scripts");
  });

  it("renders a markdown fence as prose, one level deep only", () => {
    const nested = "## Summary\n\n- point\n\n```sql\nSELECT 2;\n```";
    render(<AssistantMessage text={`\`\`\`markdown\n${nested}\n\`\`\``} />);
    // The fence body became real structure…
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Summary");
    expect(screen.getByRole("listitem")).toHaveTextContent("point");
    // …and the fence nested INSIDE it stays a code card (no further rungs).
    expect(screen.getByText("SQL")).toBeInTheDocument();
  });

  it("keeps an svg FRAGMENT in an svg fence on the code card", () => {
    const { container } = render(<AssistantMessage text={'```svg\n<circle r="2"/>\n```'} />);
    expect(container.querySelector("iframe")).toBeNull();
    expect(screen.getByText("SVG")).toBeInTheDocument();
  });

  it("renders a json array fence as a data table", () => {
    render(<AssistantMessage text={'```json\n[{"region":"EU","sales":42}]\n```'} />);
    expect(screen.getByRole("columnheader", { name: "region" })).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "EU" })).toBeInTheDocument();
  });

  it("falls back to the code card for malformed json", () => {
    const { container } = render(<AssistantMessage text={'```json\n[{"broken":\n```'} />);
    expect(screen.getByText("JSON")).toBeInTheDocument(); // card header label
    expect(container.querySelector("table")).toBeNull();
  });

  it("renders a csv fence as a data table", () => {
    render(<AssistantMessage text={'```csv\nname,age\n"Smith, Jo",40\n```'} />);
    expect(screen.getByRole("cell", { name: "Smith, Jo" })).toBeInTheDocument();
  });

  it("labels a generic fence with its language and keeps the source", () => {
    const { container } = render(<AssistantMessage text={"```sql\nSELECT 1;\n```"} />);
    // The LANGUAGE's name, not the tag the model typed.
    expect(screen.getByText("SQL")).toBeInTheDocument();
    // hljs splits the source into token spans — assert on assembled text.
    expect(container.querySelector(".rich-card__code")?.textContent).toContain("SELECT 1;");
    // The card supplies its own source <pre>; the markdown wrapper is removed.
    expect(container.querySelector(".assistant-markdown > pre .rich-card")).toBeNull();
  });

  it("renders a tsv fence as a data table", () => {
    render(<AssistantMessage text={'```tsv\nregion\tnet\n"EMEA, APAC"\t1200\n```'} />);
    // Split on tabs, not on the comma inside the quoted field.
    expect(screen.getByRole("cell", { name: "EMEA, APAC" })).toBeInTheDocument();
    expect(screen.getByRole("cell", { name: "1200" })).toBeInTheDocument();
  });

  it("falls back to the code card for a malformed tsv", () => {
    const { container } = render(<AssistantMessage text={'```tsv\na\tb\n"open\t2\n```'} />);
    expect(container.querySelector("table")).toBeNull();
    expect(container.querySelector(".rich-card__code")).not.toBeNull();
  });

  // The defect this fixed: a dialect tag used to turn highlighting off, so the
  // more precisely the model named its query, the worse the reader's copy looked.
  const DIALECT_CASES: Array<[string, string]> = [
    ["sparksql", "Spark SQL"],
    ["snowflake", "Snowflake SQL"],
    ["tsql", "T-SQL"],
    ["dax", "DAX"],
    ["kql", "KQL"],
    ["m", "Power Query M"],
    ["py", "Python"],
    ["yml", "YAML"],
  ];

  it.each(DIALECT_CASES)("highlights a %s fence and names it %s", (tag, name) => {
    const { container } = render(
      <AssistantMessage text={"```" + tag + "\nselect count(*) from sales where region = 'EMEA'\n```"} />,
    );
    expect(screen.getByText(name)).toBeInTheDocument();
    // Tokens were produced — the fence is not undifferentiated grey text.
    expect(container.querySelectorAll(".rich-card__code [class*='hljs-']").length).toBeGreaterThan(0);
  });

  it("names a tag it does not know verbatim rather than guessing one", () => {
    const { container } = render(<AssistantMessage text={"```zzql\nfrom sales | count\n```"} />);
    expect(screen.getByText("zzql")).toBeInTheDocument();
    // No grammar is guessed for it — `detect: false`.
    expect(container.querySelectorAll(".rich-card__code [class*='hljs-']").length).toBe(0);
    expect(container.querySelector(".rich-card__code")?.textContent).toContain("from sales | count");
  });

  it("shows a building placeholder for a preview fence while streaming", () => {
    const { container } = render(<AssistantMessage streaming text={`\`\`\`html\n${DOC}\n\`\`\``} />);
    expect(screen.getByText("Building…")).toBeInTheDocument();
    expect(container.querySelector("iframe")).toBeNull();
    expect(container.querySelector(".rich-card__frame-ghost")).not.toBeNull();
  });

  // The settle used to rename the artifact from "Building report…" to the
  // document's own title. The title now resolves from whatever has arrived.
  it("names a streaming report the same before and after it settles", async () => {
    const { rerender } = render(<AssistantMessage streaming text={`\`\`\`html\n${DOC}`} />);
    expect(screen.getByText("Sales Report")).toBeInTheDocument();

    rerender(<AssistantMessage text={`\`\`\`html\n${DOC}\n\`\`\``} />);
    await screen.findByTitle("Sales Report");
    expect(screen.getByText("Sales Report")).toBeInTheDocument();
  });

  // Detection requires the body to START with the document, so anything
  // trailing `</html>` is outside the span the preview renders. Copy, the
  // body, and the source view have to be the one string.
  it("copies the document span, not the fence body, when a sentence trails the document", async () => {
    render(<AssistantMessage text={`\`\`\`html\n${DOC}\nThat is the report.\n\`\`\``} />);
    await screen.findByTitle("Sales Report");

    await userEvent.click(screen.getByRole("button", { name: "Show source of Sales Report" }));
    const shown = document.querySelector(".rich-card__code")?.textContent;
    expect(shown).toBe(DOC);
    expect(shown).not.toContain("That is the report.");
  });

  it("gives a markdown fence a frame whose source reads back", async () => {
    render(<AssistantMessage text={"```markdown\n## Summary\n\n- point\n```"} />);
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Summary");

    await userEvent.click(screen.getByRole("button", { name: "Show source of Markdown" }));
    expect(document.querySelector(".rich-card__code")?.textContent).toContain("## Summary");
  });

  it("renders a streaming markdown fence as prose, never a placeholder", () => {
    render(<AssistantMessage streaming text={"```markdown\n## Partial"} />);
    expect(screen.getByRole("heading", { level: 2 })).toHaveTextContent("Partial");
    expect(screen.queryByRole("status")).toBeNull();
  });
});

describe("document fences are never tokenised (spec: chat-streaming-performance)", () => {
  beforeEach(() => {
    highlighted.fences = 0;
    highlighted.spans = 0;
  });

  const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><circle r="2"/></svg>';

  it("draws a closed html document without highlighting it", async () => {
    const { container } = render(<AssistantMessage text={`\`\`\`html\n${DOC}\n\`\`\``} />);
    await screen.findByTitle("Sales Report");
    expect(container.querySelectorAll("[class*='hljs-']")).toHaveLength(0);
    expect(highlighted).toEqual({ fences: 0, spans: 0 });
  });

  it("draws a standalone svg document without highlighting it", async () => {
    const { container } = render(<AssistantMessage text={`\`\`\`svg\n${svg}\n\`\`\``} />);
    await screen.findByTitle("SVG image");
    expect(container.querySelectorAll("[class*='hljs-']")).toHaveLength(0);
    expect(highlighted).toEqual({ fences: 0, spans: 0 });
  });

  it("draws an html document that is still streaming without highlighting it", () => {
    const open = `<!DOCTYPE html>\n<html><head><title>Live Board</title></head><body>${"<p>row</p>".repeat(20)}`;
    const { container } = render(<AssistantMessage streaming text={`\`\`\`html\n${open}`} />);
    expect(screen.getByRole("status")).toBeInTheDocument();
    expect(container.querySelectorAll("[class*='hljs-']")).toHaveLength(0);
    expect(highlighted).toEqual({ fences: 0, spans: 0 });
  });

  it("still highlights an html fragment and draws it as a code card", () => {
    const { container } = render(<AssistantMessage text={'```html\n<div class="x">hi</div>\n```'} />);
    expect(screen.getByLabelText("HTML source")).toBeInTheDocument();
    expect(container.querySelector("iframe")).toBeNull();
    expect(container.querySelectorAll("[class*='hljs-']").length).toBeGreaterThan(0);
    expect(highlighted.fences).toBe(1);
  });

  it("still highlights an html document nested in a markdown fence", () => {
    const { container } = render(<AssistantMessage text={`\`\`\`\`markdown\n\`\`\`html\n${DOC}\n\`\`\`\n\`\`\`\``} />);
    expect(container.querySelector("iframe")).toBeNull();
    expect(screen.getByLabelText("HTML source").querySelectorAll("[class*='hljs-']").length).toBeGreaterThan(0);
  });
});

describe("block splitting (spec: chat-streaming-performance)", () => {
  it("keeps a reference link resolvable when its definition is a separate block", () => {
    // Per-block rendering would sever the reference from its definition and the
    // link would come out as literal text. The splitter refuses to split here.
    render(<AssistantMessage text={"See [the docs][d] for more.\n\n[d]: https://example.com"} />);

    // The name carries the destination note every external prose link now
    // gets; what this test is about is that the reference resolved at all.
    expect(screen.getByRole("link", { name: "the docs, opens in the browser" })).toHaveAttribute(
      "href",
      "https://example.com",
    );
  });

  it("keeps both documents of a two-document answer addressable by ordinal", async () => {
    const second = DOC.replace("Sales Report", "Cost Report");
    render(
      <AssistantMessage
        conversationId="conv-1"
        messageId="msg-1"
        text={`First:\n\n\`\`\`html\n${DOC}\n\`\`\`\n\nSecond:\n\n\`\`\`html\n${second}\n\`\`\``}
      />,
    );

    // Each document lands in its own top-level block, so an ordinal resolved by
    // a mount counter would be right only by luck. Both cards must be present
    // and distinguishable.
    expect(await screen.findByTitle("Sales Report")).toBeInTheDocument();
    expect(await screen.findByTitle("Cost Report")).toBeInTheDocument();
    // A conversation with no project offers no save control: server reports are
    // saved by the assistant, so there is nothing here to address by ordinal.
    // (The project save resolves each card's ordinal; see SaveAsReportAction.test.)
    expect(screen.queryByRole("button", { name: /save/i })).not.toBeInTheDocument();
  });

  /**
   * The point of splitting: "still arriving" is a per-block fact. A fence that
   * closed is finished, whatever the tokens after it are doing, and holding it
   * as a placeholder until the whole answer lands is half-streamed output.
   */
  it("settles a fence that closed while later text is still streaming", async () => {
    render(<AssistantMessage streaming text={`\`\`\`html\n${DOC}\n\`\`\`\n\nAnd here is what it shows.`} />);

    expect(await screen.findByTitle("Sales Report")).toBeInTheDocument();
    expect(screen.queryByText("Building…")).not.toBeInTheDocument();
  });

  it("still shows a placeholder for the fence that is itself still arriving", () => {
    const { container } = render(<AssistantMessage streaming text={`Here it comes:\n\n\`\`\`html\n${DOC}`} />);

    expect(screen.getByText("Building…")).toBeInTheDocument();
    expect(container.querySelector("iframe")).toBeNull();
  });

  it("renders a heading and the paragraph after it as separate blocks", () => {
    render(<AssistantMessage text={"# Title\n\nBody text."} />);

    expect(screen.getByRole("heading", { level: 1 })).toHaveTextContent("Title");
    expect(screen.getByText("Body text.")).toBeInTheDocument();
  });
});

/**
 * The proof that consolidating the chrome moved no fence: every
 * `(language, body)` pair must land on the rung it landed on before.
 * `detect.ts` is not edited by this change, and this matrix is what would
 * catch it if it were.
 */
describe("classification matrix (spec: chat-rich-content)", () => {
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><circle r="2"/></svg>';
  const bodyOf = (container: HTMLElement) =>
    container.querySelector("iframe")
      ? "preview"
      : container.querySelector("table")
        ? "table"
        : container.querySelector(".rich-card__prose")
          ? "prose"
          : container.querySelector(".rich-card")
            ? "code"
            : "bare";

  const cases: Array<[string, string, string]> = [
    ["html document", `\`\`\`html\n${DOC}\n\`\`\``, "preview"],
    ["standalone svg", `\`\`\`svg\n${svg}\n\`\`\``, "preview"],
    ["markdown", "```markdown\n# Hi\n```", "prose"],
    ["json array of objects", '```json\n[{"a":1,"b":2}]\n```', "table"],
    ["json object", '```json\n{"a":1}\n```', "code"],
    ["malformed json", '```json\n[{"broken":\n```', "code"],
    ["csv", "```csv\nname,age\nJo,40\n```", "table"],
    ["tsv", "```tsv\nname\tage\nJo\t40\n```", "table"],
    ["malformed tsv", '```tsv\na\tb\n"open\t2\n```', "code"],
    ["sql", "```sql\nSELECT 1;\n```", "code"],
    ["sparksql", "```sparksql\nSELECT 1;\n```", "code"],
    ["dax", "```dax\nNet = SUM ( Sales[Amount] )\n```", "code"],
    ["py", "```py\nprint(1)\n```", "code"],
    // An unlabelled fence carries no `language-*` class, so it stays outside
    // the artifact ladder. "bare" describes that classification; the renderer
    // still keeps its semantic pre/code pair for whitespace fidelity.
    ["unlabelled prose", "```\njust words\n```", "bare"],
  ];

  for (const [name, text, expected] of cases) {
    it(`routes ${name} to the ${expected} body`, async () => {
      const { container } = render(<AssistantMessage text={text} />);
      await vi.waitFor(() => expect(bodyOf(container)).toBe(expected));
    });
  }
});
