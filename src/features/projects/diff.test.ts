import { describe, expect, it } from "vitest";
import { parseUnifiedDiff, toSplitRows } from "./diff";

describe("parseUnifiedDiff", () => {
  it("tracks line numbers and change markers across a hunk", () => {
    const parsed = parseUnifiedDiff(
      '--- a/data.json\n+++ b/data.json\n@@ -1,2 +1,2 @@\n {\n-  "value": 1\n+  "value": 2\n',
    );

    expect(parsed.kind).toBe("sections");
    if (parsed.kind !== "sections") return;
    const section = parsed.sections[0];
    expect(section.kind).toBe("text");
    if (section.kind !== "text") return;
    expect(section.path).toBe("data.json");
    expect(section.rows.map(({ kind, oldLine, newLine }) => ({ kind, oldLine, newLine }))).toEqual([
      { kind: "hunk", oldLine: null, newLine: null },
      { kind: "context", oldLine: 1, newLine: 1 },
      { kind: "delete", oldLine: 2, newLine: null },
      { kind: "add", oldLine: null, newLine: 2 },
    ]);
  });

  it("keeps binary and bounded-diff messages explicit", () => {
    expect(parseUnifiedDiff("Binary files a/data.bin and b/data.bin differ\n")).toEqual({
      kind: "sections",
      sections: [{ kind: "opaque", content: "Binary files a/data.bin and b/data.bin differ" }],
      truncated: false,
    });
    expect(parseUnifiedDiff("[diff truncated after 262144 bytes]\n")).toEqual({
      kind: "sections",
      sections: [{ kind: "opaque", content: "[diff truncated after 262144 bytes]" }],
      truncated: false,
    });
  });

  it("keeps every other file readable when one file's section fails to parse", () => {
    const source = [
      "--- a/data.json",
      "+++ b/data.json",
      "@@ -1,2 +1,2 @@",
      " {",
      '-  "value": 1',
      '+  "value": 2',
      "--- a/broken.txt",
      "+++ b/broken.txt",
      "@@ -1,1 +1,1 @@",
      "xyz",
      "--- a/notes.md",
      "+++ b/notes.md",
      "@@ -1 +1 @@",
      "-old",
      "+new",
      "",
    ].join("\n");

    const parsed = parseUnifiedDiff(source);
    expect(parsed.kind).toBe("sections");
    if (parsed.kind !== "sections") return;
    expect(parsed.sections.map((section) => section.kind)).toEqual(["text", "opaque", "text"]);
    const [first, broken, third] = parsed.sections;
    if (first.kind !== "text" || third.kind !== "text") throw new Error("expected text sections");
    expect(first.path).toBe("data.json");
    expect(third.path).toBe("notes.md");
    if (broken.kind !== "opaque") throw new Error("expected an opaque section");
    expect(broken.content).toContain("broken.txt");
    expect(broken.content).toContain("xyz");
  });
});

describe("toSplitRows", () => {
  it("pairs a replacement across a 'no newline' note between the delete and the add", () => {
    const parsed = parseUnifiedDiff(
      '--- a/data.json\n+++ b/data.json\n@@ -1 +1 @@\n-{"value":1}\n\\ No newline at end of file\n+{"value":2}\n\\ No newline at end of file\n',
    );
    expect(parsed.kind).toBe("sections");
    if (parsed.kind !== "sections") return;
    const section = parsed.sections[0];
    expect(section.kind).toBe("text");
    if (section.kind !== "text") return;

    const split = toSplitRows(section.rows);
    const pairs = split.filter((row) => row.kind === "pair");
    expect(pairs).toHaveLength(1);
    const [pair] = pairs;
    if (pair.kind !== "pair") throw new Error("expected a pair row");
    expect(pair.left?.kind).toBe("delete");
    expect(pair.right?.kind).toBe("add");
  });
});
