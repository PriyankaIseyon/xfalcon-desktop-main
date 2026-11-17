import { describe, expect, it } from "vitest";
import { splitMarkdown, splitMarkdownIntoBlocks } from "./splitMarkdown";

describe("splitMarkdownIntoBlocks", () => {
  it("returns nothing for empty prose", () => {
    expect(splitMarkdownIntoBlocks("")).toEqual([]);
  });

  it("separates consecutive top-level blocks", () => {
    const blocks = splitMarkdownIntoBlocks("# Title\n\nFirst paragraph.\n\nSecond paragraph.");
    expect(blocks).toEqual(["# Title", "First paragraph.", "Second paragraph."]);
  });

  it("keeps a fenced block whole even when it contains a blank line", () => {
    const source = "Intro.\n\n```sql\nselect 1;\n\nselect 2;\n```\n\nOutro.";
    const blocks = splitMarkdownIntoBlocks(source);

    expect(blocks).toHaveLength(3);
    expect(blocks[1]).toBe("```sql\nselect 1;\n\nselect 2;\n```");
  });

  it("keeps a loose list as one list rather than one block per item", () => {
    const source = "- first\n\n- second\n\n- third";
    expect(splitMarkdownIntoBlocks(source)).toEqual([source]);
  });

  it("keeps a table whole", () => {
    const source = "| a | b |\n| --- | --- |\n| 1 | 2 |";
    expect(splitMarkdownIntoBlocks(source)).toEqual([source]);
  });

  /**
   * The premise per-block memoization rests on: an answer only ever grows, so
   * every block but the last is byte-identical from one token to the next.
   */
  it("produces a stable prefix as prose is appended", () => {
    const full = "# Title\n\nFirst paragraph.\n\n```sql\nselect 1;\n```\n\nClosing words.";
    const settled = splitMarkdownIntoBlocks(full).slice(0, -1);

    for (let cut = 1; cut <= full.length; cut += 1) {
      const partial = splitMarkdownIntoBlocks(full.slice(0, cut));
      const shared = Math.min(partial.length - 1, settled.length);
      for (let index = 0; index < shared; index += 1) {
        expect(partial[index]).toBe(settled[index]);
      }
    }
  });

  describe("constructs that lose their meaning when split", () => {
    it("keeps a message with a raw HTML block whole", () => {
      const source = "Before.\n\n<div>one</div>\n\n<div>two</div>\n\nAfter.";
      expect(splitMarkdownIntoBlocks(source)).toEqual([source]);
    });

    it("keeps a message with a link reference definition whole", () => {
      const source = "See [the docs][d] for more.\n\n[d]: https://example.com";
      expect(splitMarkdownIntoBlocks(source)).toEqual([source]);
    });

    it("keeps a message whole when the definition is nested inside a quote", () => {
      const source = "See [the docs][d].\n\n> quoted\n>\n> [d]: https://example.com";
      expect(splitMarkdownIntoBlocks(source)).toEqual([source]);
    });
  });
});

/**
 * The incremental path, held to the whole-message path byte for byte.
 *
 * Reusing a settled prefix is a real risk: block-level markdown is mostly
 * context-free above the block level, but not entirely — an indented code
 * block's node can begin after its indentation, and a reference definition
 * anywhere in the document changes what a link elsewhere renders as. The safety
 * net is not the reasoning, it is this: for every corpus below and EVERY prefix
 * of it, streaming the prefixes through the incremental path must produce exactly
 * what parsing that prefix whole produces.
 *
 * A divergence here is a misrendered answer, which is worse than a slow one, so
 * the incremental path exists only for as long as this passes.
 */
describe("incremental splitting matches whole-message splitting", () => {
  const corpora: Array<[name: string, source: string]> = [
    ["headings and paragraphs", "# Title\n\nFirst paragraph.\n\nSecond paragraph.\n\n## Next\n\nMore."],
    ["a fence with a blank line", "Intro.\n\n```sql\nselect 1;\n\nselect 2;\n```\n\nOutro."],
    ["an unterminated fence", "Intro.\n\n```python\nprint(1)\nprint(2)"],
    ["a loose list", "Before.\n\n- first\n\n- second\n\n- third\n\nAfter."],
    ["a tight list then prose", "- a\n- b\n- c\n\nparagraph"],
    ["a list with indented continuation", "- a\n\n  continued\n\n- b"],
    ["a table", "Intro.\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\nOutro."],
    ["a setext heading forming late", "Title\n===\n\nbody"],
    ["indented code", "para\n\n    indented code\n\npara two"],
    ["a blockquote", "> quoted\n> more\n\nplain"],
    ["nested quote with a fence", "> outer\n>\n> ```js\n> code\n> ```\n\nafter"],
    ["a raw HTML block", "Before.\n\n<div>one</div>\n\nAfter."],
    ["a link reference definition", "See [the docs][d].\n\n[d]: https://example.com\n\nAfter."],
    ["a footnote definition", "Text[^1].\n\n[^1]: the note\n\nAfter."],
    ["a thematic break", "one\n\n---\n\ntwo"],
    ["a numbered list starting at three", "3. third\n4. fourth\n\nafter"],
    ["trailing whitespace and blank lines", "one\n\n\n\ntwo   \n\nthree"],
    ["CRLF line endings", "one\r\n\r\ntwo\r\n\r\n```\r\ncode\r\n```"],
    ["a task list", "- [ ] todo\n- [x] done\n\nafter"],
    ["multibyte prose", "净销售额 📊\n\n**加粗**\n\n```sql\nselect 净销售额;\n```"],
    ["a heading with no blank line after", "# Title\nbody text\n\n## Second\nmore"],
  ];

  for (const [name, source] of corpora) {
    it(`agrees on every prefix of ${name}`, () => {
      let carried = splitMarkdown(source.slice(0, 1));
      for (let cut = 1; cut <= source.length; cut += 1) {
        const prefix = source.slice(0, cut);
        carried = splitMarkdown(prefix, carried);
        expect(carried.blocks).toEqual(splitMarkdownIntoBlocks(prefix));
        expect(carried.source).toBe(prefix);
      }
    });
  }

  it("re-parses whole when the source is not an extension of the previous one", () => {
    const first = splitMarkdown("# One\n\nbody");
    // A caller that replaces the text rather than appending to it must not get a
    // prefix stitched onto an unrelated tail.
    const replaced = splitMarkdown("# Two\n\nother", first);
    expect(replaced.blocks).toEqual(splitMarkdownIntoBlocks("# Two\n\nother"));
  });

  it("streams a long answer without re-parsing what settled", () => {
    // The property the incremental path exists for, stated as a count rather than
    // a duration: the settled blocks must be the SAME strings, not merely equal
    // ones, so a memo keyed on them holds.
    const sections: string[] = [];
    for (let i = 0; i < 20; i += 1) {
      sections.push(`## Section ${i}`, "", `Paragraph ${i}.`, "", "```sql", `select ${i};`, "```", "");
    }
    const source = sections.join("\n");

    let carried = splitMarkdown(source.slice(0, 1));
    let previousBlocks = carried.blocks;
    for (let cut = 2; cut <= source.length; cut += 1) {
      carried = splitMarkdown(source.slice(0, cut), carried);
      const shared = Math.min(previousBlocks.length - 1, carried.blocks.length - 1);
      for (let index = 0; index < shared; index += 1) {
        expect(carried.blocks[index]).toBe(previousBlocks[index]);
      }
      previousBlocks = carried.blocks;
    }
    expect(carried.blocks.length).toBeGreaterThan(20);
  });
});
