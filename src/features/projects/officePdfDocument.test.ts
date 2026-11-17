import { describe, expect, it, vi } from "vitest";
import type { PDFPageProxy } from "pdfjs-dist";
import { MAX_OFFICE_PDF_TEXT_CODE_UNITS, MAX_OFFICE_PDF_TEXT_ITEMS, readOfficePdfPageText } from "./officePdfDocument";

function pageWithItems(items: unknown[]) {
  const cancel = vi.fn();
  let sent = false;
  const stream = new ReadableStream({
    pull(controller) {
      if (sent) return;
      sent = true;
      controller.enqueue({ items, styles: {}, lang: null } as never);
    },
    cancel,
  });
  return {
    page: { streamTextContent: vi.fn(() => stream) } as unknown as PDFPageProxy,
    cancel,
  };
}

describe("readOfficePdfPageText", () => {
  it("caps accumulated selectable text and reports truncation", async () => {
    const { page, cancel } = pageWithItems([{ str: "x".repeat(MAX_OFFICE_PDF_TEXT_CODE_UNITS + 100) }]);
    const read = readOfficePdfPageText(page);

    await expect(read.promise).resolves.toEqual({
      text: "x".repeat(MAX_OFFICE_PDF_TEXT_CODE_UNITS),
      truncated: true,
    });
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("stops after an exact text limit without waiting for another stream chunk", async () => {
    const { page, cancel } = pageWithItems([{ str: "x".repeat(MAX_OFFICE_PDF_TEXT_CODE_UNITS) }]);
    const read = readOfficePdfPageText(page);

    await expect(read.promise).resolves.toEqual({ text: "x".repeat(MAX_OFFICE_PDF_TEXT_CODE_UNITS), truncated: true });
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("stops after an exact item limit without waiting for another stream chunk", async () => {
    let accessedItems = 0;
    const items = Array.from({ length: MAX_OFFICE_PDF_TEXT_ITEMS }, () => ({ str: "x" }));
    const boundedItems = new Proxy(items, {
      get(target, property, receiver) {
        if (typeof property === "string" && /^\d+$/.test(property)) accessedItems += 1;
        return Reflect.get(target, property, receiver);
      },
    });
    const { page, cancel } = pageWithItems(boundedItems);
    const read = readOfficePdfPageText(page);

    await expect(read.promise).resolves.toEqual({
      text: Array.from({ length: MAX_OFFICE_PDF_TEXT_ITEMS }, () => "x").join(" "),
      truncated: true,
    });
    expect(accessedItems).toBe(MAX_OFFICE_PDF_TEXT_ITEMS);
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("counts separators inside the text budget and never leaves a trailing separator", async () => {
    const halfLimit = MAX_OFFICE_PDF_TEXT_CODE_UNITS / 2;
    const { page } = pageWithItems([{ str: "a".repeat(halfLimit) }, { str: "b".repeat(halfLimit) }]);
    const read = readOfficePdfPageText(page);
    const result = await read.promise;

    expect(result.text.length).toBe(MAX_OFFICE_PDF_TEXT_CODE_UNITS);
    expect(result.text).toBe(`${"a".repeat(halfLimit)} ${"b".repeat(halfLimit - 1)}`);
    expect(result.text.endsWith(" ")).toBe(false);
    expect(result.truncated).toBe(true);
  });

  it("does not split a surrogate pair at the text limit", async () => {
    const { page } = pageWithItems([{ str: "a".repeat(MAX_OFFICE_PDF_TEXT_CODE_UNITS - 2) }, { str: "😀" }]);
    const read = readOfficePdfPageText(page);
    const result = await read.promise;

    expect(result.text).toBe("a".repeat(MAX_OFFICE_PDF_TEXT_CODE_UNITS - 2));
    expect(result.text.length).toBeLessThanOrEqual(MAX_OFFICE_PDF_TEXT_CODE_UNITS);
    const lastCodeUnit = result.text.charCodeAt(result.text.length - 1);
    expect(lastCodeUnit < 0xd800 || lastCodeUnit > 0xdbff).toBe(true);
    expect(result.truncated).toBe(true);
  });

  it("counts empty and non-text items toward the item limit", async () => {
    let accessedItems = 0;
    const sourceItems = Array.from({ length: MAX_OFFICE_PDF_TEXT_ITEMS + 5 }, (_, index) =>
      index % 2 === 0 ? { str: "" } : { type: "beginMarkedContent" },
    );
    const boundedItems = new Proxy(sourceItems, {
      get(target, property, receiver) {
        if (typeof property === "string" && /^\d+$/.test(property)) accessedItems += 1;
        return Reflect.get(target, property, receiver);
      },
    }) as unknown as Array<{ str: string }>;
    const { page, cancel } = pageWithItems(boundedItems);
    const read = readOfficePdfPageText(page);

    await expect(read.promise).resolves.toEqual({ text: "", truncated: true });
    expect(accessedItems).toBe(MAX_OFFICE_PDF_TEXT_ITEMS);
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("stops at the exact item limit when every item is empty or non-text", async () => {
    const items = Array.from({ length: MAX_OFFICE_PDF_TEXT_ITEMS }, (_, index) =>
      index % 2 === 0 ? { str: "" } : { type: "beginMarkedContent" },
    );
    const { page, cancel } = pageWithItems(items);
    const read = readOfficePdfPageText(page);

    await expect(read.promise).resolves.toEqual({ text: "", truncated: true });
    expect(cancel).toHaveBeenCalledTimes(1);
  });

  it("cancels an active PDF.js text stream", async () => {
    let cancelStream!: () => void;
    const stream = new ReadableStream({
      start() {},
      cancel() {
        cancelStream();
      },
    });
    const cancelled = new Promise<void>((resolve) => {
      cancelStream = resolve;
    });
    const page = { streamTextContent: vi.fn(() => stream) } as unknown as PDFPageProxy;
    const read = readOfficePdfPageText(page);

    read.cancel();
    await expect(cancelled).resolves.toBeUndefined();
    await expect(read.promise).resolves.toEqual({ text: "", truncated: false });
  });
});
