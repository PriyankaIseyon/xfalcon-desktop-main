import { describe, expect, it } from "vitest";
import { describeProjectError, inOriginalLineEndings, utf8ToBase64 } from "./projectFileText";

describe("utf8ToBase64", () => {
  it("encodes text a raw btoa would refuse", () => {
    // `btoa("é")` throws: the code point is above U+00FF once encoded.
    expect(utf8ToBase64("héllo — ok")).toBe(btoa(String.fromCharCode(...new TextEncoder().encode("héllo — ok"))));
    expect(utf8ToBase64("🙂")).toBe("8J+Zgg==");
  });

  it("round-trips through the same decoding the backend applies", () => {
    const text = "line one\nline two — with an em dash\n";
    const decoded = new TextDecoder().decode(
      Uint8Array.from(atob(utf8ToBase64(text)), (character) => character.charCodeAt(0)),
    );
    expect(decoded).toBe(text);
  });

  it("encodes content larger than one chunk without overflowing the stack", () => {
    const text = "x".repeat(0x8000 * 3 + 7);
    expect(atob(utf8ToBase64(text))).toHaveLength(text.length);
  });

  it("encodes an empty file as an empty payload rather than failing", () => {
    expect(utf8ToBase64("")).toBe("");
  });
});

describe("describeProjectError", () => {
  it("keeps the backend's own redacted detail as a readable sentence", () => {
    expect(describeProjectError("path escapes the project root", "This file could not be read.")).toBe(
      "This file could not be read. Path escapes the project root.",
    );
  });

  it("falls back rather than stringifying something that is not a message", () => {
    expect(describeProjectError({ unexpected: true }, "This file could not be read.")).toBe(
      "This file could not be read.",
    );
    expect(describeProjectError("   ", "This file could not be read.")).toBe("This file could not be read.");
  });
});

describe("inOriginalLineEndings", () => {
  it("puts CRLF back on a text area's LF when the file was CRLF throughout", () => {
    expect(inOriginalLineEndings("a\nb\nc\n", "x\r\ny\r\n")).toBe("a\r\nb\r\nc\r\n");
  });

  it("does not double a break that already is CRLF", () => {
    expect(inOriginalLineEndings("a\r\nb\n", "x\r\ny\r\n")).toBe("a\r\nb\r\n");
  });

  it("leaves a Unix file, a mixed file, and a one-line file alone", () => {
    expect(inOriginalLineEndings("a\nb\n", "x\ny\n")).toBe("a\nb\n");
    expect(inOriginalLineEndings("a\nb\n", "x\r\ny\n")).toBe("a\nb\n");
    expect(inOriginalLineEndings("a b", "x y")).toBe("a b");
  });
});
