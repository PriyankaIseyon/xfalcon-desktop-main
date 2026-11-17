import { describe, expect, it } from "vitest";
import fixtures from "./htmlDocumentFixtures.json";
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
} from "./detect";

describe("detectSvgDocument", () => {
  const SVG = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 4 4"><circle r="2"/></svg>';
  it("accepts an svg fence with a standalone document", () => {
    expect(detectSvgDocument("svg", SVG)).toBe(true);
  });
  it("accepts an unlabelled fence whose body is one svg document", () => {
    expect(detectSvgDocument("", `\n${SVG}\n`)).toBe(true);
  });
  it("accepts an XML prolog before the root", () => {
    expect(detectSvgDocument("svg", `<?xml version="1.0"?>\n${SVG}`)).toBe(true);
  });
  it("rejects an svg fragment even in an svg fence — code card owns it", () => {
    expect(detectSvgDocument("svg", '<circle r="2"/>')).toBe(false);
  });
  it("rejects svg-looking bodies in a labelled non-svg fence", () => {
    expect(detectSvgDocument("xml", SVG)).toBe(false);
  });
  it("rejects a document missing its closing tag", () => {
    expect(detectSvgDocument("svg", '<svg viewBox="0 0 4 4"><rect/>')).toBe(false);
  });
});

const DOC = `<!DOCTYPE html>\n<html>\n<head><title>Sales Report</title></head>\n<body><h1>Report</h1><p>${"x".repeat(40)}</p></body>\n</html>`;

describe("detectHtmlDocument", () => {
  it("accepts a full document in an html fence", () => {
    expect(detectHtmlDocument("html", DOC)).toBe(true);
  });
  it("accepts an unlabelled fence carrying a document", () => {
    expect(detectHtmlDocument("", DOC)).toBe(true);
  });
  it("rejects an html fence holding a fragment", () => {
    expect(detectHtmlDocument("html", "<div>fragment</div>")).toBe(false);
  });
  it("rejects a document in a python fence", () => {
    expect(detectHtmlDocument("python", DOC)).toBe(false);
  });
});

describe("extractHtmlDocument", () => {
  it("slices the document span out of surrounding prose", () => {
    const text = `Here is your report:\n${DOC}\nEnjoy!`;
    expect(extractHtmlDocument(text)).toBe(DOC);
  });
  it("ends at the FIRST closing tag", () => {
    const two = `${DOC}\n${DOC}`;
    expect(extractHtmlDocument(two)).toBe(DOC);
  });
  it("returns null without a closing tag", () => {
    expect(extractHtmlDocument("<!doctype html><html><body>partial")).toBeNull();
  });
});

describe("resolveHtmlTitle", () => {
  it("reads the title", () => {
    expect(resolveHtmlTitle(DOC)).toBe("Sales Report");
  });
  it("null when absent", () => {
    expect(resolveHtmlTitle("<html><body></body></html>")).toBeNull();
  });
});

describe("parseJsonTable", () => {
  it("tabulates an array of objects with a key union", () => {
    const table = parseJsonTable('[{"a":1,"b":"x"},{"a":2,"c":true}]');
    expect(table?.headers).toEqual(["a", "b", "c"]);
    expect(table?.rows).toEqual([
      ["1", "x", ""],
      ["2", "", "true"],
    ]);
  });
  it("rejects malformed JSON", () => {
    expect(parseJsonTable('[{"a":1,')).toBeNull();
  });
  it("rejects arrays of scalars", () => {
    expect(parseJsonTable("[1,2,3]")).toBeNull();
  });
  it("stringifies nested values into cells", () => {
    const table = parseJsonTable('[{"a":{"n":1}}]');
    expect(table?.rows[0]?.[0]).toBe('{"n":1}');
  });
});

describe("parseDelimited", () => {
  it("parses quoted fields with embedded commas and escaped quotes", () => {
    const table = parseDelimited('name,note\n"Smith, Jo","said ""hi"""\nplain,ok', ",");
    expect(table?.headers).toEqual(["name", "note"]);
    expect(table?.rows).toEqual([
      ["Smith, Jo", 'said "hi"'],
      ["plain", "ok"],
    ]);
  });
  it("pads ragged rows to the header width", () => {
    const table = parseDelimited("a,b,c\n1,2\n1,2,3,4", ",");
    expect(table?.rows).toEqual([
      ["1", "2", ""],
      ["1", "2", "3"],
    ]);
  });
  it("rejects an unterminated quote", () => {
    expect(parseDelimited('a,b\n"open,2', ",")).toBeNull();
  });
  it("rejects a lone header", () => {
    expect(parseDelimited("a,b,c", ",")).toBeNull();
  });

  // The delimiter is a parameter over ONE parser, so the quoting rules that
  // make a comma inside a field one cell have to do the same for a tab.
  it("applies the same quoting rules to a tab-separated export", () => {
    const table = parseDelimited('region\tnote\n"EMEA\tAPAC"\t"said ""hi"""\nAMER\tok', "\t");
    expect(table?.headers).toEqual(["region", "note"]);
    expect(table?.rows).toEqual([
      ["EMEA\tAPAC", 'said "hi"'],
      ["AMER", "ok"],
    ]);
  });
  it("does not split a tab-separated row on its commas", () => {
    const table = parseDelimited("label\tvalue\nEMEA, APAC\t1200", "\t");
    expect(table?.rows).toEqual([["EMEA, APAC", "1200"]]);
  });
});

/**
 * The other half of the contract in `src-tauri/src/reports/tests.rs`. Both read
 * THIS file, so a case added for one scanner is enforced on the other — and a
 * divergence means the reader saves a document other than the one they clicked,
 * with nothing in the result saying so (spec: reports).
 */
describe("findHtmlDocuments", () => {
  it.each(fixtures.cases.map((testCase) => [testCase.name, testCase] as const))(
    "agrees with the Rust scanner: %s",
    (_name, testCase) => {
      expect(findHtmlDocuments(testCase.text)).toEqual(testCase.documents);
    },
  );

  it("reads the same fixture file the Rust test reads", () => {
    // A shared set with one case in it would pass while pinning nothing.
    expect(fixtures.cases.length).toBeGreaterThanOrEqual(10);
  });
});

describe("htmlDocumentIndex", () => {
  const documents = fixtures.cases.find(
    (testCase) => testCase.name === "three html fences separated by prose",
  )!.documents;

  it("resolves the second of three documents to ordinal 1", () => {
    expect(htmlDocumentIndex(documents, documents[1])).toBe(1);
  });

  it("tolerates surrounding whitespace before giving up", () => {
    expect(htmlDocumentIndex(documents, `\n  ${documents[2]}  \n`)).toBe(2);
  });

  it("reports -1 for a document the message does not hold", () => {
    expect(htmlDocumentIndex(documents, "<!doctype html><html><body>other</body></html>")).toBe(-1);
  });
});

describe("hasExternalReference", () => {
  it("finds a CDN script the preview scheme cannot load", () => {
    expect(
      hasExternalReference('<!doctype html><html><head><script src="https://cdn/x.js"></script></head></html>'),
    ).toBe(true);
  });

  it("finds an external stylesheet", () => {
    expect(
      hasExternalReference('<!doctype html><html><head><link rel="stylesheet" href="https://cdn/x.css"></head></html>'),
    ).toBe(true);
  });

  it("passes a genuinely self-contained document", () => {
    expect(
      hasExternalReference(
        "<!doctype html><html><head><style>body{color:#111}</style><script>const a=1;</script></head></html>",
      ),
    ).toBe(false);
  });
});
